use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CalculationTarget, CancellationToken, CellAddress, CellValue, DefinedName,
    DefinedNameScope, EditBatch, FormulaText, RecalculationMode, TargetCalculationLimits,
    WorkbookCalculationSession, WorkbookChange, WorkbookDraft, analyze_defined_name,
    calculate_targets, calculate_workbook, scan_formula_capabilities, scan_function_usage,
};

// One mebibyte is the default main-thread stack on Windows and the smallest host stack these
// entry points must tolerate at their default limits.
const SMALL_HOST_STACK: usize = 1024 * 1024;

fn formula_cell() -> CellAddress {
    CellAddress::from_a1("A1").unwrap()
}

fn name(draft: &mut WorkbookDraft, name: &str, formula: &str) {
    draft
        .set_defined_name(
            DefinedName::new(
                name,
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx(formula).unwrap(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
}

fn workbook_with(formula: &str, names: &[(String, String)]) -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    for (defined, body) in names {
        name(&mut draft, defined, body);
    }
    let sheet = draft.workbook().sheets()[0].id();
    draft
        .set_cell_formula(
            sheet,
            formula_cell(),
            FormulaText::from_xlsx(formula).unwrap(),
        )
        .unwrap();
    draft
}

fn nested(open: &str, depth: usize) -> String {
    format!("{}1{}", open.repeat(depth), ")".repeat(depth))
}

/// Runs every public entry point that parses, analyzes, or evaluates formulas on a small stack
/// and returns the full-calculation result of A1.
fn run_on_small_stack(draft: WorkbookDraft, names: Vec<String>) -> CalculationCellResult {
    std::thread::Builder::new()
        .stack_size(SMALL_HOST_STACK)
        .spawn(move || {
            let workbook = draft.workbook();
            let sheet = workbook.sheets()[0].id();
            let cell = CalculationCellId::new(sheet, formula_cell());
            let full = calculate_workbook(workbook, CalculationOptions::default())
                .cell(cell)
                .unwrap()
                .clone();
            let targeted = calculate_targets(
                workbook,
                &[CalculationTarget::cell(cell)],
                CalculationOptions::default(),
                TargetCalculationLimits::default(),
                CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(targeted.cell(cell), Some(&full));
            scan_formula_capabilities(workbook);
            scan_function_usage(workbook);
            for defined in &names {
                let _ = analyze_defined_name(workbook, defined, None);
            }

            let mut session = WorkbookCalculationSession::new(draft.clone());
            session
                .recalculate(
                    RecalculationMode::Full,
                    CalculationOptions::default(),
                    CancellationToken::new(),
                )
                .unwrap();
            let revision = session.workbook().semantic_revision();
            session
                .apply_changes(
                    revision,
                    EditBatch::new([WorkbookChange::set_cell_value(
                        sheet,
                        CellAddress::from_a1("B1").unwrap(),
                        CellValue::number(1.0).unwrap(),
                    )]),
                )
                .unwrap();
            session
                .recalculate(
                    RecalculationMode::Auto,
                    CalculationOptions::default(),
                    CancellationToken::new(),
                )
                .unwrap();
            full
        })
        .unwrap()
        .join()
        .unwrap()
}

fn number(value: f64) -> CalculationCellResult {
    CalculationCellResult::Value(CellValue::number(value).unwrap())
}

fn assert_limit(result: &CalculationCellResult) {
    let CalculationCellResult::Unavailable(issue) = result else {
        panic!("expected a resource limit, got {result:?}");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn deepest_nesting_within_the_default_limit_completes_on_a_small_stack() {
    let limit = usize::try_from(CalculationLimits::default().max_formula_nesting_depth()).unwrap();
    // Each repetition adds this many syntax-tree levels.
    for (open, levels) in [
        ("SUM(", 1),
        ("(", 1),
        ("IF(TRUE,", 1),
        ("-(", 2),
        ("LET(x,1,x+", 2),
    ] {
        let within = (limit - 1) / levels;
        let result = run_on_small_stack(workbook_with(&nested(open, within), &[]), Vec::new());
        assert!(
            matches!(result, CalculationCellResult::Value(_)),
            "{open}: {result:?}"
        );
        let beyond = limit / levels + 1;
        let result = run_on_small_stack(workbook_with(&nested(open, beyond), &[]), Vec::new());
        assert_limit(&result);
    }
}

#[test]
fn recursive_lambda_at_the_default_depth_completes_on_a_small_stack() {
    let depth = CalculationLimits::default().max_lambda_depth();
    let names = vec![(
        "Count".to_owned(),
        "LAMBDA(n,IF(n<=1,1,1+Count(n-1)))".to_owned(),
    )];
    let within = run_on_small_stack(
        workbook_with(&format!("Count({})", depth - 1), &names),
        vec!["Count".to_owned()],
    );
    assert_eq!(within, number((depth - 1) as f64));
    let beyond = run_on_small_stack(
        workbook_with(&format!("Count({})", depth + 1), &names),
        vec!["Count".to_owned()],
    );
    assert_limit(&beyond);
}

#[test]
fn long_defined_name_chains_complete_or_report_limits_on_a_small_stack() {
    for depth in [100_usize, 1_000, 5_000] {
        let mut names = vec![("Chain_0".to_owned(), "1".to_owned())];
        names.extend(
            (1..=depth).map(|level| (format!("Chain_{level}"), format!("Chain_{}+1", level - 1))),
        );
        let analyzed = vec![format!("Chain_{depth}")];
        let result = run_on_small_stack(workbook_with(&format!("Chain_{depth}"), &names), analyzed);
        if depth == 100 {
            assert_eq!(result, number(101.0));
        } else {
            assert_limit(&result);
        }
    }
}
