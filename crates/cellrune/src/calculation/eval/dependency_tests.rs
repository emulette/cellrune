use std::cell::Cell;

use super::{
    DependencyTarget, Engine, EvalContext, EvaluationBudget, ReferenceSelectionMode,
    VisitedDefinitions, compare_targets, table_dependency_by_id, workbook_table_topologies,
};
use crate::calculation::runtime::Rect;
use crate::{
    CalculationHints, CalculationLimits, CalculationOptions, CellAddress, CellContent, CellRange,
    CellValue, DateSystem, DefinedName, DefinedNameScope, FormulaCell, FormulaDialect,
    FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId,
    SheetName, SheetVisibility, Table, TableColumn, TableId, TableName, WorkbookDraft,
    WorkbookSnapshot, WorkbookSource,
};

#[test]
fn reference_selection_recurses_into_calls_and_honors_map_scope() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_defined_name(
            DefinedName::new("item", DefinedNameScope::Workbook, formula("Z1"), false)
                .expect("valid defined name"),
        )
        .expect("defined name edit");
    draft
        .set_cell_formula(sheet_id, address("B1"), formula("SUM(C1)"))
        .expect("ordinary call formula");
    draft
        .set_cell_formula(
            sheet_id,
            address("B2"),
            formula("MAP(A1,LAMBDA(item,item))"),
        )
        .expect("MAP formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());

    let ordinary = collect_reference_selection_inputs(&engine, (0, 1, 2));
    assert_eq!(ordinary, vec![rect(1, 3)]);

    let scoped = collect_reference_selection_inputs(&engine, (0, 2, 2));
    assert_eq!(scoped, vec![rect(1, 1)]);
}

#[test]
fn unresolved_dynamic_dependencies_honor_map_scope() {
    // `expr_has_unresolved_dynamic_dependency` decides whether every full calculation pays for
    // an extra evaluate-then-recollect pass over the whole workbook. Resolving a lambda
    // parameter as a workbook name makes it answer yes for a formula that holds no dynamic
    // reference at all. Its sibling `expr_contains_dynamic_reference_function` already carries
    // the shared scope discipline; the two have to agree.
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_defined_name(
            DefinedName::new(
                "item",
                DefinedNameScope::Workbook,
                // A target that cannot be resolved during analysis, so the name is genuinely
                // unresolved wherever it is not shadowed. The second assertion below is what
                // proves that; without it the first one would pass for the wrong reason.
                formula("INDIRECT(A1)"),
                false,
            )
            .expect("valid defined name"),
        )
        .expect("defined name edit");
    draft
        .set_cell_formula(
            sheet_id,
            address("B2"),
            formula("MAP(A1,LAMBDA(item,item+1))"),
        )
        .expect("MAP formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
    assert!(
        !has_unresolved_dynamic_dependency(&engine, (0, 2, 2)),
        "the lambda parameter shadows the dynamic workbook name"
    );

    draft
        .set_cell_formula(sheet_id, address("B3"), formula("SUM(item)"))
        .expect("unshadowed reference to the same name");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
    assert!(
        has_unresolved_dynamic_dependency(&engine, (0, 3, 2)),
        "an unshadowed reference to the same name still reports the dynamic dependency"
    );
}

#[test]
fn unresolved_dynamic_dependency_analysis_polls_cancellation_during_recursion() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_cell_formula(
            sheet_id,
            address("A1"),
            formula("SUM(LET(value,1,value+1))"),
        )
        .expect("formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
    let polls = Cell::new(0_u32);
    let cancelled = || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 3
    };

    assert_eq!(
        engine.has_unresolved_dynamic_dependencies(&cancelled),
        Err(()),
    );
    assert!(polls.get() >= 3);
}

#[test]
fn dependency_formula_index_polls_cancellation_between_sparse_cells() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_cell_formula(sheet_id, address("A1"), formula("1"))
        .expect("first formula");
    draft
        .set_cell_formula(sheet_id, address("A2"), formula("A1+1"))
        .expect("second formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
    let polls = Cell::new(0_u32);
    let cancelled = || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 3
    };

    assert_eq!(engine.dependencies_cancellable(&cancelled), Err(()));
    assert_eq!(polls.get(), 3);
}

#[test]
fn let_dependencies_follow_sequential_scope_and_shadow_defined_names() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_defined_name(
            DefinedName::new(
                "second",
                DefinedNameScope::Workbook,
                formula("INDIRECT(A1)"),
                false,
            )
            .expect("valid defined name"),
        )
        .expect("defined name edit");
    draft
        .set_cell_formula(
            sheet_id,
            address("B1"),
            formula("LET(first,A1,second,first,second+1)"),
        )
        .expect("sequential LET formula");
    draft
        .set_cell_formula(
            sheet_id,
            address("B2"),
            formula("LET(first,second,second,A1,first)"),
        )
        .expect("forward LET formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());

    assert!(
        !has_unresolved_dynamic_dependency(&engine, (0, 1, 2)),
        "a completed LET binding must shadow the dynamic workbook name",
    );
    assert!(
        has_unresolved_dynamic_dependency(&engine, (0, 2, 2)),
        "a value expression must not see a binding declared later",
    );
    assert_eq!(
        collect_reference_selection_inputs(&engine, (0, 1, 2)),
        vec![rect(1, 1)],
    );
}

#[test]
fn formula_metadata_let_selectors_keep_bound_reference_value_dependencies() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_cell_value(sheet_id, address("A1"), CellValue::Blank)
        .expect("selector input");
    draft
        .set_cell_formula(sheet_id, address("C1"), formula("1+1"))
        .expect("first metadata target");
    draft
        .set_cell_formula(sheet_id, address("C2"), formula("2+2"))
        .expect("second metadata target");
    draft
        .set_cell_formula(
            sheet_id,
            address("B1"),
            formula("FORMULATEXT(LET(ref_value,A1,OFFSET(C1,ref_value,0)))"),
        )
        .expect("metadata selector formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
    let targets = engine
        .dependency_targets_cancellable(&|| false)
        .expect("dependency collection");

    assert_eq!(
        targets.get(&(0, 1, 2)),
        Some(&vec![
            DependencyTarget::Cell((0, 1, 1)),
            DependencyTarget::FormulaContent((0, 1, 3)),
        ]),
        "the bound reference selects metadata geometrically but remains a value dependency when used by OFFSET",
    );
}

#[test]
fn let_scope_resolves_dynamic_and_resized_dependencies() {
    let mut dynamic = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    dynamic
        .set_cell_formula(sheet_id, address("A2"), formula("40+2"))
        .expect("dynamic target formula");
    dynamic
        .set_cell_formula(
            sheet_id,
            address("B1"),
            formula("LET(step,1,target,OFFSET(A1,step,0),target)"),
        )
        .expect("dynamic LET formula");
    let engine = Engine::evaluate(dynamic.workbook(), CalculationOptions::default());
    assert_eq!(
        engine.dependencies.get(&(0, 1, 2)),
        Some(&vec![(0, 2, 1)]),
        "OFFSET must resolve LET bindings while collecting the second-pass graph",
    );

    let mut resized = WorkbookDraft::new();
    for address_text in ["A1", "A2", "A3", "C1", "C2", "C3"] {
        resized
            .set_cell_formula(sheet_id, address(address_text), formula("1"))
            .expect("range input formula");
    }
    resized
        .set_cell_formula(
            sheet_id,
            address("B1"),
            formula(r#"LET(values,A1,SUMIF(C1:C3,">0",values))"#),
        )
        .expect("resized LET formula");
    let engine = Engine::evaluate(resized.workbook(), CalculationOptions::default());
    assert_eq!(
        engine.dependencies.get(&(0, 1, 2)),
        Some(&vec![
            (0, 1, 1),
            (0, 1, 3),
            (0, 2, 1),
            (0, 2, 3),
            (0, 3, 1),
            (0, 3, 3),
        ]),
        "SUMIF must resize a reference-valued LET binding to the criteria shape",
    );
}

#[test]
fn multi_area_dependencies_keep_dynamic_reference_selection_inputs() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    for (cell, value) in [("A1", "1"), ("A2", "2"), ("C1", "3"), ("D1", "1")] {
        draft
            .set_cell_formula(sheet_id, address(cell), formula(value))
            .expect("input formula");
    }
    draft
        .set_cell_formula(
            sheet_id,
            address("F1"),
            formula("SUM((OFFSET(A1,D1,0),C1))"),
        )
        .expect("dynamic union formula");
    draft
        .set_cell_formula(sheet_id, address("G1"), formula("SUM(OFFSET(A1,D1,0) A2)"))
        .expect("dynamic intersection formula");

    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
    let expected = vec![(0, 1, 1), (0, 1, 3), (0, 1, 4), (0, 2, 1)];
    assert_eq!(
        engine.dependencies.get(&(0, 1, 6)),
        Some(&expected),
        "the union keeps its final cells and OFFSET selector inputs",
    );
    assert_eq!(
        engine.dependencies.get(&(0, 1, 7)),
        Some(&vec![(0, 1, 1), (0, 1, 4), (0, 2, 1)]),
        "the intersection keeps its final cell and OFFSET selector inputs",
    );
}

#[test]
fn three_d_dependencies_stay_compact_and_cover_every_formula_sheet() {
    let mut draft = WorkbookDraft::new();
    let first = SheetId::new(1).expect("default sheet ID");
    let second = draft
        .add_sheet(SheetName::new("Sheet2").expect("valid sheet name"))
        .expect("second sheet");
    let third = draft
        .add_sheet(SheetName::new("Sheet3").expect("valid sheet name"))
        .expect("third sheet");
    for sheet in [first, second, third] {
        draft
            .set_cell_formula(sheet, address("A1"), formula("1"))
            .expect("input formula");
    }
    draft
        .set_cell_formula(
            first,
            address("B1"),
            formula("SUM(Sheet1:Sheet3!A1,Sheet3:Sheet1!A1)"),
        )
        .expect("3-D formula");

    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
    let spans = engine
        .dependency_rectangles()
        .remove(&(0, 1, 2))
        .expect("consumer dependency spans");

    assert_eq!(
        spans.len(),
        1,
        "equivalent forward and reverse spans must share one compact dependency"
    );
    assert_eq!(
        spans[0].rects().map(|rect| rect.sheet).collect::<Vec<_>>(),
        vec![0, 1, 2],
    );
    assert_eq!(
        engine.dependencies.get(&(0, 1, 2)),
        Some(&vec![(0, 1, 1), (1, 1, 1), (2, 1, 1)]),
    );

    for (max_edges, exceeded) in [(3, false), (2, true)] {
        let limits = CalculationLimits::default()
            .with_max_dependency_edges(max_edges)
            .expect("nonzero dependency limit");
        let limited = Engine::evaluate(
            draft.workbook(),
            CalculationOptions::default().with_limits(limits),
        );
        assert_eq!(
            limited.dependency_limit_exceeded, exceeded,
            "max_dependency_edges={max_edges}",
        );
    }
}

#[test]
fn three_d_dependencies_connect_intermediate_array_owners() {
    let mut draft = WorkbookDraft::new();
    let first = SheetId::new(1).expect("default sheet ID");
    let second = draft
        .add_sheet(SheetName::new("Sheet2").expect("valid sheet name"))
        .expect("second sheet");
    draft
        .add_sheet(SheetName::new("Sheet3").expect("valid sheet name"))
        .expect("third sheet");
    draft
        .set_cell_dynamic_formula(
            second,
            address("A1"),
            formula("SEQUENCE(2)"),
            Some(CellRange::new(address("A1"), address("A2")).expect("valid spill range")),
        )
        .expect("intermediate array formula");
    draft
        .set_cell_formula(first, address("B1"), formula("SUM(Sheet1:Sheet3!A2)"))
        .expect("3-D spill consumer");

    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());

    assert_eq!(
        engine.dependencies.get(&(0, 1, 2)),
        Some(&vec![(1, 1, 1)]),
        "the referenced spill follower must depend on its intermediate-sheet anchor",
    );
}

#[test]
fn typed_targets_preserve_table_identity_empty_bands_and_current_rows() {
    let (workbook, table_id) = table_dependency_workbook("B3");
    let engine = Engine::analyze(&workbook, CalculationOptions::default());
    let targets = engine
        .dependency_targets_cancellable(&|| false)
        .expect("dependency targets");

    let current_row = targets.get(&(0, 2, 3)).expect("current-row targets");
    assert!(
        current_row.iter().any(|target| matches!(
            target,
            DependencyTarget::TableIdentity(table) if table.table_id() == table_id
        )),
        "a structured reference retains stable table identity",
    );
    assert!(
        current_row
            .iter()
            .any(|target| matches!(target, DependencyTarget::Cell((0, 2, 2)))),
        "a current-row selector depends on the cell in its actual data row",
    );

    let empty_totals = targets.get(&(0, 1, 4)).expect("empty totals targets");
    assert_eq!(
        empty_totals
            .iter()
            .filter(|target| matches!(target, DependencyTarget::TableIdentity(_)))
            .count(),
        1,
    );
    assert!(
        empty_totals
            .iter()
            .all(|target| matches!(target, DependencyTarget::TableIdentity(_))),
        "metadata-only empty bands retain table identity without inventing a cell area",
    );

    let original = table_dependency_by_id(&workbook, table_id).expect("original topology");
    let (same_geometry, _) = table_dependency_workbook("B3");
    assert_eq!(
        table_dependency_by_id(&same_geometry, table_id),
        Some(original),
    );
    let (grown, _) = table_dependency_workbook("B4");
    assert_ne!(
        table_dependency_by_id(&grown, table_id),
        Some(original),
        "geometry growth changes the compiled table topology revision",
    );
    let (shrunk, _) = table_dependency_workbook("B2");
    assert_ne!(
        table_dependency_by_id(&shrunk, table_id),
        Some(original),
        "geometry shrink changes the compiled table topology revision",
    );

    let evaluated = Engine::evaluate(&workbook, CalculationOptions::default());
    let compiled = evaluated
        .compiled(&|| false)
        .expect("compiled dependency topology");
    assert_eq!(
        compiled.table_topology_matches(&same_geometry, &|| false),
        Ok(true),
    );
    assert_eq!(
        compiled.table_topology_matches(&grown, &|| false),
        Ok(false),
    );
    assert_eq!(
        compiled.table_topology_matches(&shrunk, &|| false),
        Ok(false),
    );
}

#[test]
fn spill_references_record_typed_anchor_targets_and_schedule_producers() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            address("B1"),
            formula("SEQUENCE(2,2)"),
            Some(CellRange::new(address("B1"), address("C2")).expect("spill range")),
        )
        .expect("dynamic anchor");
    draft
        .set_cell_formula(sheet_id, address("F1"), formula("SUM(B1#)"))
        .expect("spill consumer");
    draft
        .set_cell_formula(sheet_id, address("G1"), formula("AREAS(B1#)"))
        .expect("metadata-only spill consumer");
    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
    let targets = engine
        .dependency_targets_cancellable(&|| false)
        .expect("dependency targets");

    assert_eq!(
        targets.get(&(0, 1, 6)),
        Some(&vec![DependencyTarget::SpillAnchor((0, 1, 2))]),
    );
    assert_eq!(
        engine.dependencies.get(&(0, 1, 6)),
        Some(&vec![(0, 1, 2)]),
        "the spill producer must be evaluated before its reference consumer",
    );
    assert_eq!(
        targets.get(&(0, 1, 7)),
        Some(&vec![DependencyTarget::SpillAnchor((0, 1, 2))]),
        "metadata-only consumers still depend on spill shape identity",
    );
}

#[test]
fn table_topology_hashing_polls_cancellation_inside_column_work() {
    let (workbook, _) = table_dependency_workbook("B3");
    let polls = Cell::new(0_u32);
    let cancelled = || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 3
    };

    assert_eq!(workbook_table_topologies(&workbook, &cancelled), Err(()));
    assert_eq!(polls.get(), 3);
}

fn table_dependency_workbook(end: &str) -> (WorkbookSnapshot, TableId) {
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let table_id = TableId::new(1).expect("table ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (address_text, value) in [
        ("A1", CellValue::Text("Label".to_owned())),
        ("B1", CellValue::Text("Amount".to_owned())),
        ("A2", CellValue::Text("First".to_owned())),
        ("B2", CellValue::number(10.0).expect("finite table value")),
        ("A3", CellValue::Text("Second".to_owned())),
        ("B3", CellValue::number(20.0).expect("finite table value")),
        ("A4", CellValue::Text("Third".to_owned())),
        ("B4", CellValue::number(30.0).expect("finite table value")),
    ] {
        sheet
            .insert_cell(address(address_text), CellContent::Literal(value))
            .expect("unique literal");
    }
    for (address_text, formula_text) in [
        ("C2", "Sales[@Amount]"),
        ("C3", "Sales[@Amount]"),
        ("D1", "AREAS(Sales[#Totals])"),
    ] {
        sheet
            .insert_cell(
                address(address_text),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    formula(formula_text),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("unique formula");
    }
    sheet.set_tables(vec![
        Table::new(
            table_id,
            TableName::new("Sales").expect("table name"),
            TableName::new("Sales").expect("display name"),
            CellRange::new(address("A1"), address(end)).expect("table range"),
            1,
            0,
            vec![
                TableColumn::new(1, "Label", None).expect("label column"),
                TableColumn::new(2, "Amount", None).expect("amount column"),
            ],
        )
        .expect("valid table"),
    ]);
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("dependency-target-test", "1").expect("provider"),
            None,
        ),
    )
    .expect("valid workbook");
    (workbook, table_id)
}

fn has_unresolved_dynamic_dependency(engine: &Engine<'_>, cell: (usize, u32, u32)) -> bool {
    let expr = engine.parsed_expr(cell).expect("parsed test formula");
    let budget = EvaluationBudget::default();
    engine.expr_has_unresolved_dynamic_dependency(
        EvalContext::for_evaluation(cell, &budget),
        expr,
        &mut VisitedDefinitions::default(),
        &mut Vec::new(),
    )
}

fn collect_reference_selection_inputs(engine: &Engine<'_>, cell: (usize, u32, u32)) -> Vec<Rect> {
    let expr = engine.parsed_expr(cell).expect("parsed test formula");
    let mut output = Vec::new();
    let budget = EvaluationBudget::default();
    engine.collect_reference_selection_inputs(
        EvalContext::for_evaluation(cell, &budget),
        ReferenceSelectionMode::ReferenceValue,
        expr,
        &mut VisitedDefinitions::default(),
        &mut Vec::new(),
        &mut output,
    );
    output.sort_by(compare_targets);
    output.dedup();
    output
        .iter()
        .filter_map(DependencyTarget::span)
        .flat_map(|span| span.rects().collect::<Vec<_>>())
        .collect()
}

fn rect(row: u32, column: u32) -> Rect {
    Rect {
        sheet: 0,
        row_start: row,
        col_start: column,
        row_end: row,
        col_end: column,
        whole_rows: false,
    }
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid test address")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("valid test formula")
}
