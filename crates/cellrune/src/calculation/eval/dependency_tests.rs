use std::collections::BTreeSet;

use super::{Engine, EvalContext};
use crate::calculation::runtime::{Rect, RectSpan};
use crate::{
    CalculationLimits, CalculationOptions, CellAddress, CellRange, DefinedName, DefinedNameScope,
    FormulaText, SheetId, SheetName, WorkbookDraft,
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

fn has_unresolved_dynamic_dependency(engine: &Engine<'_>, cell: (usize, u32, u32)) -> bool {
    let expr = engine.parsed_expr(cell).expect("parsed test formula");
    engine.expr_has_unresolved_dynamic_dependency(
        EvalContext::for_cell(cell),
        expr,
        &mut BTreeSet::new(),
        &mut Vec::new(),
    )
}

fn collect_reference_selection_inputs(engine: &Engine<'_>, cell: (usize, u32, u32)) -> Vec<Rect> {
    let expr = engine.parsed_expr(cell).expect("parsed test formula");
    let mut output = Vec::new();
    engine.collect_reference_selection_inputs(
        EvalContext::for_cell(cell),
        expr,
        &mut BTreeSet::new(),
        &mut Vec::new(),
        &mut output,
    );
    output.iter().flat_map(RectSpan::rects).collect()
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
