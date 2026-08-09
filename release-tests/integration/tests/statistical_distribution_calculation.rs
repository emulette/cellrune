use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationIssueCode,
    CalculationLimits, CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch,
    ExcelError, FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession,
    WorkbookChange, calculate_workbook,
};

const RESULT_CELLS: &[&str] = &[
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24", "F25", "F26", "F27",
];

#[test]
fn statistical_distribution_wave_matches_full_calculation_after_incremental_edits() {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                number_change(sheet, "A1", 1.0),
                number_change(sheet, "A2", 2.0),
                number_change(sheet, "A3", 3.0),
                number_change(sheet, "A4", 4.0),
                number_change(sheet, "B1", 1.1),
                number_change(sheet, "B2", 2.1),
                number_change(sheet, "B3", 3.2),
                number_change(sheet, "B4", 4.2),
                number_change(sheet, "D1", 1.5),
                number_change(sheet, "D2", 5.0),
                number_change(sheet, "D3", 7.0),
                number_change(sheet, "D4", 0.4),
                number_change(sheet, "D5", 10.0),
                number_change(sheet, "D6", 2.5),
                formula_change(sheet, "F1", "=F.DIST(D1,D2,D3,TRUE)"),
                formula_change(sheet, "F2", "=F.DIST.RT(D1,D2,D3)"),
                formula_change(sheet, "F3", "=F.INV(D4,D2,D3)"),
                formula_change(sheet, "F4", "=F.INV.RT(D4,D2,D3)"),
                formula_change(sheet, "F5", "=F.TEST(A1:A4,B1:B4)"),
                formula_change(sheet, "F6", "=FDIST(D1,D2,D3)"),
                formula_change(sheet, "F7", "=FINV(D4,D2,D3)"),
                formula_change(sheet, "F8", "=FTEST(A1:A4,B1:B4)"),
                formula_change(sheet, "F9", "=T.DIST(D1,D5,TRUE)"),
                formula_change(sheet, "F10", "=T.DIST.2T(D1,D5)"),
                formula_change(sheet, "F11", "=T.DIST.RT(D1,D5)"),
                formula_change(sheet, "F12", "=T.INV(0.9,D5)"),
                formula_change(sheet, "F13", "=T.INV.2T(0.1,D5)"),
                formula_change(sheet, "F14", "=T.TEST(A1:A4,B1:B4,2,1)"),
                formula_change(sheet, "F15", "=TDIST(D1,D5,2)"),
                formula_change(sheet, "F16", "=TINV(0.1,D5)"),
                formula_change(sheet, "F17", "=TTEST(A1:A4,B1:B4,2,1)"),
                formula_change(sheet, "F18", "=Z.TEST(A1:A4,D6)"),
                formula_change(sheet, "F19", "=ZTEST(A1:A4,D6)"),
                formula_change(sheet, "F20", "=COVARIANCE.S(A1:A4,B1:B4)"),
                formula_change(sheet, "F21", "=F.DIST.RT(1E13,1,2)"),
                formula_change(sheet, "F22", "=F.TEST({-7E153,7E153},{-1E-154,1E-154})"),
                formula_change(sheet, "F23", "=T.TEST({-1E100,1E100},{-2E100,2E100},2,3)"),
                formula_change(sheet, "F24", "=F.INV(0,0,1)"),
                formula_change(sheet, "F25", "=F.INV.RT(1,0,1)"),
                formula_change(sheet, "F26", "=Z.TEST({1,2},-1E308,1E-308)"),
                formula_change(sheet, "F27", "=Z.TEST({1.5,1.5,1.5,1.5},1.5,5E-324)"),
            ]),
        )
        .expect("statistical-distribution workbook batch");

    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    assert_number(
        session.calculation(),
        sheet,
        "F21",
        9.9999999999985e-14,
        5e-9,
    );
    assert_number(
        session.calculation(),
        sheet,
        "F22",
        1.8189136353359467e-308,
        5e-9,
    );
    assert_number(session.calculation(), sheet, "F23", 1.0, 0.0);
    assert_error(session.calculation(), sheet, "F24", ExcelError::Number);
    assert_error(session.calculation(), sheet, "F25", ExcelError::Number);
    assert_number(session.calculation(), sheet, "F26", 0.0, 0.0);
    assert_number(session.calculation(), sheet, "F27", 0.5, 0.0);
    assert_matches_full(&session, sheet, RESULT_CELLS);

    for (address, value) in [("D1", 2.0), ("D4", 0.6), ("B4", 5.0)] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([number_change(sheet, address, value)]),
            )
            .expect("incremental input edit");
        let delta = session
            .recalculate(
                RecalculationMode::Incremental,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("incremental calculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_matches_full(&session, sheet, RESULT_CELLS);
    }
}

#[test]
fn large_shape_asymptotic_work_obeys_the_function_iteration_limit() {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([formula_change(
                sheet,
                "A1",
                "=F.DIST(1.001,2000000,2000000,TRUE)",
            )]),
        )
        .expect("large-shape formula");
    let limits = CalculationLimits::default()
        .with_max_function_iterations(1)
        .expect("positive function iteration limit");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default().with_limits(limits),
            CancellationToken::new(),
        )
        .expect("bounded calculation");
    let cell = CalculationCellId::new(sheet, cell_address("A1"));
    let Some(CalculationCellResult::Unavailable(issue)) = session
        .calculation()
        .and_then(|snapshot| snapshot.cell(cell))
    else {
        panic!("large-shape series must stop at the function iteration limit");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_function_iterations"));
}

fn assert_matches_full(session: &WorkbookCalculationSession, sheet: SheetId, addresses: &[&str]) {
    let full = calculate_workbook(session.workbook(), CalculationOptions::default());
    let incremental = session.calculation().expect("installed calculation");
    for address in addresses {
        let cell = CalculationCellId::new(sheet, cell_address(address));
        assert_eq!(
            incremental.cell(cell),
            full.cell(cell),
            "mismatch at {address}",
        );
    }
}

fn assert_number(
    calculation: Option<&cellrune::CalculationSnapshot>,
    sheet: SheetId,
    address: &str,
    expected: f64,
    relative_tolerance: f64,
) {
    let cell = CalculationCellId::new(sheet, cell_address(address));
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.and_then(|snapshot| snapshot.cell(cell))
    else {
        panic!("expected numeric result at {address}");
    };
    let difference = (actual.get() - expected).abs();
    let tolerance = 2.0 * f64::from_bits(1) + relative_tolerance * expected.abs();
    assert!(
        difference <= tolerance,
        "unexpected result at {address}: expected {expected}, got {}",
        actual.get(),
    );
}

fn assert_error(
    calculation: Option<&cellrune::CalculationSnapshot>,
    sheet: SheetId,
    address: &str,
    expected: ExcelError,
) {
    let cell = CalculationCellId::new(sheet, cell_address(address));
    let Some(CalculationCellResult::Value(CellValue::Error(actual))) =
        calculation.and_then(|snapshot| snapshot.cell(cell))
    else {
        panic!("expected error result at {address}");
    };
    assert_eq!(*actual, expected);
}

fn number_change(sheet: SheetId, address: &str, value: f64) -> WorkbookChange {
    WorkbookChange::set_cell_value(
        sheet,
        cell_address(address),
        CellValue::number(value).expect("finite test number"),
    )
}

fn formula_change(sheet: SheetId, address: &str, formula: &str) -> WorkbookChange {
    WorkbookChange::set_cell_formula(
        sheet,
        cell_address(address),
        FormulaText::from_user_input(formula).expect("valid test formula"),
    )
}

fn cell_address(address: &str) -> CellAddress {
    CellAddress::from_a1(address).expect("valid test address")
}
