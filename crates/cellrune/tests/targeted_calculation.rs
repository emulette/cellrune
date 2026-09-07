use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationOptions,
    CalculationTarget, CancellationToken, CellAddress, CellRange, CellValue, FormulaText, SheetId,
    TargetCalculationErrorCode, TargetCalculationLimits, WorkbookDraft, calculate_targets,
    calculate_workbook,
};

fn sheet() -> SheetId {
    SheetId::new(1).expect("sheet")
}
fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("address")
}
fn id(value: &str) -> CalculationCellId {
    CalculationCellId::new(sheet(), address(value))
}
fn target(value: &str) -> CalculationTarget {
    CalculationTarget::cell(id(value))
}
fn formula(draft: &mut WorkbookDraft, cell: &str, value: &str) {
    draft
        .set_cell_formula(
            sheet(),
            address(cell),
            FormulaText::from_xlsx(value).expect("formula"),
        )
        .expect("set formula");
}
fn value(draft: &mut WorkbookDraft, cell: &str, number: f64) {
    draft
        .set_cell_value(
            sheet(),
            address(cell),
            CellValue::number(number).expect("finite"),
        )
        .expect("set value");
}
fn number(number: f64) -> CalculationCellResult {
    CalculationCellResult::Value(CellValue::number(number).expect("finite"))
}

#[test]
fn cycles_with_cross_edges_classify_all_members_and_consumers_like_full() {
    let mut draft = WorkbookDraft::new();
    formula(&mut draft, "A1", "B1+C1");
    formula(&mut draft, "B1", "A1");
    formula(&mut draft, "C1", "B1");
    formula(&mut draft, "D1", "A1+1");
    let result = calculate_targets(
        draft.workbook(),
        &[target("D1"), target("C1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("cycle scope");
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(result.cell(id("C1")), full.cell(id("C1")));
    assert_eq!(result.cell(id("D1")), full.cell(id("D1")));
}

#[test]
fn named_callable_aliases_and_builtin_shadows_load_their_reachable_names() {
    use cellrune::{DefinedName, DefinedNameScope};
    let mut draft = WorkbookDraft::new();
    formula(&mut draft, "B1", "3+4");
    for (name, text) in [
        ("Factor", "B1"),
        ("Adder", "LAMBDA(x,x+Factor)"),
        ("Alias", "Adder"),
        ("MAP", "LAMBDA(value,callback,Factor)"),
    ] {
        draft
            .set_defined_name(
                DefinedName::new(
                    name,
                    DefinedNameScope::Workbook,
                    FormulaText::from_xlsx(text).expect("formula"),
                    false,
                )
                .expect("name"),
            )
            .expect("set name");
    }
    formula(&mut draft, "A1", "Alias(2)+MAP(0,LAMBDA(x,x))");
    let result = calculate_targets(
        draft.workbook(),
        &[target("A1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("callable names");
    assert_eq!(result.cell(id("A1")), Some(&number(16.0)));
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(result.cell(id("A1")), full.cell(id("A1")));
}

#[test]
fn first_request_only_evaluates_precedents_and_keeps_source_unchanged() {
    let mut draft = WorkbookDraft::new();
    value(&mut draft, "A1", 1.0);
    formula(&mut draft, "B1", "A1+1");
    formula(&mut draft, "C1", "B1*2");
    formula(&mut draft, "D1", "C1+1");
    formula(&mut draft, "Y1", "UNKNOWN_FUNCTION(1)");
    formula(&mut draft, "Z1", "Z1");
    let fingerprint = draft.workbook().fingerprint();
    let result = calculate_targets(
        draft.workbook(),
        &[target("C1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("partial result");
    assert_eq!(result.cell(id("C1")), Some(&number(4.0)));
    assert_eq!(result.evaluated_count(), 2);
    assert_eq!(result.parsed_formula_count(), 2);
    assert_eq!(result.len(), 1);
    assert_eq!(result.cell(id("D1")), None);
    assert_eq!(draft.workbook().fingerprint(), fingerprint);
    assert_eq!(result.source_fingerprint(), fingerprint);
}

#[test]
fn overlapping_ranges_and_shared_precedents_are_deduplicated() {
    let mut draft = WorkbookDraft::new();
    value(&mut draft, "A1", 2.0);
    formula(&mut draft, "B1", "A1+1");
    formula(&mut draft, "C1", "B1*2");
    formula(&mut draft, "D1", "B1+3");
    formula(&mut draft, "E1", "C1+D1");
    let result = calculate_targets(
        draft.workbook(),
        &[
            target("E1"),
            CalculationTarget::new(
                sheet(),
                CellRange::new(address("C1"), address("F1")).expect("range"),
            ),
            target("C1"),
        ],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("partial result");
    assert_eq!(result.len(), 4);
    assert_eq!(result.evaluated_count(), 4);
    assert_eq!(result.cell(id("E1")), Some(&number(12.0)));
    assert_eq!(
        result.cell(id("F1")),
        Some(&CalculationCellResult::Value(CellValue::Blank))
    );
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for cell in ["C1", "D1", "E1"] {
        assert_eq!(result.cell(id(cell)), full.cell(id(cell)));
    }
}

#[test]
fn dynamic_selectors_are_resolved_after_their_input_formulas() {
    let mut draft = WorkbookDraft::new();
    value(&mut draft, "A1", 2.0);
    formula(&mut draft, "B1", "A1+2");
    formula(&mut draft, "C1", "\"B\"&1");
    formula(&mut draft, "D1", "INDIRECT(C1)+1");
    formula(&mut draft, "E1", "SUM(OFFSET(A1,0,1,1,1))");
    formula(&mut draft, "F1", "LET(n,\"B1\",INDIRECT(n))");
    let result = calculate_targets(
        draft.workbook(),
        &[target("D1"), target("E1"), target("F1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("dynamic result");
    assert_eq!(result.cell(id("D1")), Some(&number(5.0)));
    assert_eq!(result.cell(id("E1")), Some(&number(4.0)));
    assert_eq!(result.cell(id("F1")), Some(&number(4.0)));
    assert_eq!(result.evaluated_count(), 5);
}

#[test]
fn cycles_and_unsupported_dependencies_do_not_suppress_independent_targets() {
    let mut draft = WorkbookDraft::new();
    formula(&mut draft, "A1", "B1");
    formula(&mut draft, "B1", "A1");
    formula(&mut draft, "C1", "A1+1");
    formula(&mut draft, "D1", "UNKNOWN_FUNCTION(1)");
    formula(&mut draft, "E1", "D1+1");
    formula(&mut draft, "F1", "1+2");
    let result = calculate_targets(
        draft.workbook(),
        &[target("A1"), target("C1"), target("E1"), target("F1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("independent results");
    for (cell, expected) in [
        ("A1", CalculationIssueCode::CircularReference),
        ("C1", CalculationIssueCode::BlockedByUpstream),
        ("E1", CalculationIssueCode::BlockedByUpstream),
    ] {
        let Some(CalculationCellResult::Unavailable(issue)) = result.cell(id(cell)) else {
            panic!("expected issue for {cell}")
        };
        assert_eq!(issue.code(), expected);
    }
    assert_eq!(result.cell(id("F1")), Some(&number(3.0)));
}

#[test]
fn invalid_targets_limits_and_cancellation_fail_without_partial_results() {
    let mut draft = WorkbookDraft::new();
    formula(&mut draft, "A1", "B1+1");
    formula(&mut draft, "B1", "1+1");
    let options = CalculationOptions::default();
    let limits = TargetCalculationLimits::default();
    assert_eq!(
        calculate_targets(
            draft.workbook(),
            &[],
            options,
            limits,
            CancellationToken::new()
        )
        .expect_err("empty")
        .code(),
        TargetCalculationErrorCode::EmptyTargets
    );
    let huge = CalculationTarget::new(
        sheet(),
        CellRange::new(address("A1"), address("XFD1048576")).expect("range"),
    );
    assert_eq!(
        calculate_targets(
            draft.workbook(),
            &[huge],
            options,
            limits,
            CancellationToken::new()
        )
        .expect_err("range limit")
        .code(),
        TargetCalculationErrorCode::TargetLimitExceeded
    );
    assert_eq!(
        calculate_targets(
            draft.workbook(),
            &[target("A1")],
            options,
            TargetCalculationLimits::new(1, 1, 1).expect("limits"),
            CancellationToken::new()
        )
        .expect_err("work limit")
        .code(),
        TargetCalculationErrorCode::EvaluationLimitExceeded
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        calculate_targets(
            draft.workbook(),
            &[target("A1")],
            options,
            limits,
            cancellation
        )
        .expect_err("cancelled")
        .code(),
        TargetCalculationErrorCode::Cancelled
    );
}

#[test]
fn long_dependency_chain_uses_an_iterative_schedule() {
    let mut draft = WorkbookDraft::new();
    value(&mut draft, "A1", 1.0);
    for row in 2..=3_000 {
        formula(&mut draft, &format!("A{row}"), &format!("A{}+1", row - 1));
    }
    let result = calculate_targets(
        draft.workbook(),
        &[target("A3000")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("long chain");
    assert_eq!(result.cell(id("A3000")), Some(&number(3000.0)));
    assert_eq!(result.evaluated_count(), 2999);
}

#[test]
fn dynamic_references_inside_called_lambdas_resolve_formula_inputs() {
    let mut draft = WorkbookDraft::new();
    formula(&mut draft, "B1", "3+4");
    formula(&mut draft, "C1", "LET(f,LAMBDA(x,INDIRECT(x)),f(\"B1\"))");
    let result = calculate_targets(
        draft.workbook(),
        &[target("C1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("lambda result");
    assert_eq!(result.cell(id("C1")), Some(&number(7.0)));
}

#[test]
fn requested_undeclared_anchor_resolves_a_follower_read_earlier_in_target_order() {
    let mut draft = WorkbookDraft::new();
    draft
        .set_cell_dynamic_formula(
            sheet(),
            address("D1"),
            FormulaText::from_xlsx("SEQUENCE(3)").expect("formula"),
            None,
        )
        .expect("dynamic formula");
    formula(&mut draft, "A1", "D3+1");
    let result = calculate_targets(
        draft.workbook(),
        &[target("A1"), target("D1"), target("D3")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("spill scope");
    assert_eq!(result.cell(id("A1")), Some(&number(4.0)));
    assert_eq!(result.cell(id("D3")), Some(&number(3.0)));
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(result.cell(id("A1")), full.cell(id("A1")));
}

#[test]
fn scoped_names_cross_sheet_and_three_dimensional_precedents_match_full() {
    use cellrune::{DefinedName, DefinedNameScope, SheetName};
    let mut draft = WorkbookDraft::new();
    let second = draft
        .add_sheet(SheetName::new("Second").expect("name"))
        .expect("sheet");
    let third = draft
        .add_sheet(SheetName::new("Third").expect("name"))
        .expect("sheet");
    formula(&mut draft, "B1", "1+1");
    for (sheet, text) in [(second, "3+4"), (third, "5+6")] {
        draft
            .set_cell_formula(
                sheet,
                address("B1"),
                FormulaText::from_xlsx(text).expect("formula"),
            )
            .expect("formula");
    }
    for (name, text, scope) in [
        ("Factor", "Second!B1*2", DefinedNameScope::Workbook),
        ("Local", "Factor+Third!B1", DefinedNameScope::Sheet(sheet())),
    ] {
        draft
            .set_defined_name(
                DefinedName::new(
                    name,
                    scope,
                    FormulaText::from_xlsx(text).expect("formula"),
                    false,
                )
                .expect("name"),
            )
            .expect("set name");
    }
    formula(&mut draft, "A1", "Local+SUM(Sheet1:Third!B1)");
    let result = calculate_targets(
        draft.workbook(),
        &[target("A1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("name and 3D scope");
    assert_eq!(result.cell(id("A1")), Some(&number(45.0)));
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(result.cell(id("A1")), full.cell(id("A1")));
    assert_eq!(result.parsed_formula_count(), 4);
}

#[test]
fn declared_spill_followers_calculate_the_whole_anchor_and_check_collisions() {
    let mut draft = WorkbookDraft::new();
    draft
        .set_cell_dynamic_formula(
            sheet(),
            address("B1"),
            FormulaText::from_xlsx("SEQUENCE(3)").expect("formula"),
            Some(CellRange::new(address("B1"), address("B3")).expect("range")),
        )
        .expect("dynamic formula");
    let result = calculate_targets(
        draft.workbook(),
        &[target("B3")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("follower");
    assert_eq!(result.cell(id("B3")), Some(&number(3.0)));
    assert_eq!(result.evaluated_count(), 1);
    draft
        .set_cell_dynamic_formula(
            sheet(),
            address("D1"),
            FormulaText::from_xlsx("SEQUENCE(3)").expect("formula"),
            None,
        )
        .expect("dynamic formula");
    value(&mut draft, "D3", 99.0);
    let result = calculate_targets(
        draft.workbook(),
        &[target("D1")],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("collision");
    let full = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(result.cell(id("D1")), full.cell(id("D1")));
    assert_eq!(
        result.cell(id("D1")),
        Some(&CalculationCellResult::Value(CellValue::Error(
            cellrune::ExcelError::Spill
        )))
    );
    let original = draft.workbook().clone();
    draft
        .set_cell_dynamic_formula(
            sheet(),
            address("B1"),
            FormulaText::from_xlsx("SEQUENCE(2)").expect("smaller array formula"),
            Some(CellRange::new(address("B1"), address("B2")).expect("smaller range")),
        )
        .expect("shrink array");
    let read_followers = |workbook: &cellrune::WorkbookSnapshot| {
        calculate_targets(
            workbook,
            &[target("B1"), target("B2"), target("B3")],
            CalculationOptions::default(),
            TargetCalculationLimits::default(),
            CancellationToken::new(),
        )
        .expect("array followers after an edit")
    };
    let smaller = read_followers(draft.workbook());
    assert_eq!(smaller.cell(id("B2")), Some(&number(2.0)));
    assert_eq!(
        smaller.cell(id("B3")),
        Some(&CalculationCellResult::Value(CellValue::Blank))
    );
    value(&mut draft, "B1", 8.0);
    let replaced = read_followers(draft.workbook());
    assert_eq!(replaced.cell(id("B1")), Some(&number(8.0)));
    assert_eq!(
        replaced.cell(id("B2")),
        Some(&CalculationCellResult::Value(CellValue::Blank))
    );
    draft
        .clear_cell(sheet(), address("B1"))
        .expect("clear anchor");
    assert_eq!(
        read_followers(draft.workbook()).cell(id("B1")),
        Some(&CalculationCellResult::Value(CellValue::Blank))
    );
    assert_eq!(read_followers(&original).cell(id("B3")), Some(&number(3.0)));
}

#[test]
fn session_partial_requests_preserve_full_state_and_reuse_only_current_options() {
    use cellrune::{EditBatch, RecalculationMode, WorkbookCalculationSession, WorkbookChange};
    let mut draft = WorkbookDraft::new();
    value(&mut draft, "A1", 1.0);
    formula(&mut draft, "B1", "A1+1");
    let mut session = WorkbookCalculationSession::new(draft);
    let options = CalculationOptions::default();
    let limits = TargetCalculationLimits::default();
    let partial = session
        .calculate_targets(&[target("B1")], options, limits, CancellationToken::new())
        .expect("first partial");
    assert_eq!(partial.cell(id("B1")), Some(&number(2.0)));
    assert!(session.calculation().is_none());
    session
        .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
        .expect("full calculation");
    let cached = session
        .calculate_targets(
            &[target("B1"), target("A1"), target("C1")],
            options,
            limits,
            CancellationToken::new(),
        )
        .expect("cached partial");
    assert_eq!(cached.reused_count(), 1);
    assert_eq!(cached.evaluated_count(), 0);
    assert_eq!(cached.cell(id("B1")), Some(&number(2.0)));
    let changed_options = session
        .calculate_targets(
            &[target("B1")],
            options.with_arithmetic_semantics(cellrune::ArithmeticSemantics::Ieee754),
            limits,
            CancellationToken::new(),
        )
        .expect("new options");
    assert_eq!(changed_options.reused_count(), 0);
    let stale = session
        .prepare_target_calculation(&[target("B1")], options, limits, CancellationToken::new())
        .expect("prepare")
        .run()
        .expect("run");
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet(),
                address("A1"),
                CellValue::number(5.0).expect("number"),
            )]),
        )
        .expect("edit");
    assert_eq!(
        session
            .finish_target_calculation(stale)
            .expect_err("stale")
            .code(),
        TargetCalculationErrorCode::StaleResult
    );
    let dirty = session
        .calculate_targets(&[target("B1")], options, limits, CancellationToken::new())
        .expect("dirty partial");
    assert_eq!(dirty.cell(id("B1")), Some(&number(6.0)));
    assert_eq!(dirty.reused_count(), 0);
    assert_eq!(
        session
            .calculation()
            .expect("old complete state")
            .cell(id("B1")),
        Some(&number(2.0))
    );
    session
        .recalculate(
            RecalculationMode::Incremental,
            options,
            CancellationToken::new(),
        )
        .expect("dirty state survived");
    assert_eq!(
        session
            .calculation()
            .expect("current full state")
            .cell(id("B1")),
        Some(&number(6.0))
    );
}
