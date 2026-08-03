use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationOptions,
    CancellationToken, CellAddress, CellValue, EditBatch, ExcelError, FormulaText,
    RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange, calculate_workbook,
};

const DISTRIBUTION_CELLS: &[&str] = &["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8"];

#[test]
fn distribution_formulas_match_full_calculation_after_incremental_edits() {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                number_change(sheet, "A1", 0.6),
                number_change(sheet, "A2", 8.0),
                number_change(sheet, "A3", 10.0),
                number_change(sheet, "A4", 10.0),
                number_change(sheet, "A5", 0.4),
                number_change(sheet, "A6", 3.0),
                formula_change(sheet, "C1", "=BETA.DIST(A1,A2,A3,TRUE)"),
                formula_change(sheet, "C2", "=BETA.INV(A1,A2,A3)"),
                formula_change(sheet, "C3", "=GAMMA.DIST(2,A2,1.5,TRUE)"),
                formula_change(sheet, "C4", "=GAMMA.INV(A1,3,1.5)"),
                formula_change(sheet, "C5", "=BINOM.DIST(A6,A4,A5,TRUE)"),
                formula_change(sheet, "C6", "=CRITBINOM(A4,A5,A1)"),
                formula_change(sheet, "C7", "=NEGBINOMDIST(A6,4,A5)"),
                formula_change(sheet, "C8", "=HYPGEOM.DIST(1,4,8,A4*2,TRUE)"),
            ]),
        )
        .expect("distribution workbook batch");
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    assert_number(session.calculation(), sheet, "C6", 4.0);
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);

    // A value edit shifts every family, including both inverse solvers.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "A1", 0.95)]),
        )
        .expect("probability input edit");
    let probability_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("probability incremental calculation");
    assert_eq!(
        probability_delta.mode(),
        CalculationExecutionMode::Incremental
    );
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);

    // A shape edit drives the beta and gamma families into their #NUM! domain.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "A2", -1.0)]),
        )
        .expect("invalid shape edit");
    let invalid_shape_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("invalid shape incremental calculation");
    assert_eq!(
        invalid_shape_delta.mode(),
        CalculationExecutionMode::Incremental
    );
    assert_error(session.calculation(), sheet, "C1", ExcelError::Number);
    assert_error(session.calculation(), sheet, "C3", ExcelError::Number);
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);

    // Recovery out of the error domain must also match a fresh full pass.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "A2", 2.0)]),
        )
        .expect("shape recovery edit");
    let recovery_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("shape recovery incremental calculation");
    assert_eq!(recovery_delta.mode(), CalculationExecutionMode::Incremental);
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);

    // An out-of-range probability breaks the discrete family, then recovers.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "A5", 2.0)]),
        )
        .expect("invalid discrete probability edit");
    session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("invalid discrete probability incremental calculation");
    assert_error(session.calculation(), sheet, "C5", ExcelError::Number);
    assert_error(session.calculation(), sheet, "C6", ExcelError::Number);
    assert_error(session.calculation(), sheet, "C7", ExcelError::Number);
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "A5", 0.5)]),
        )
        .expect("discrete probability recovery edit");
    session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("discrete probability recovery incremental calculation");
    assert_matches_full(&session, sheet, DISTRIBUTION_CELLS);
}

fn assert_matches_full(session: &WorkbookCalculationSession, sheet: SheetId, addresses: &[&str]) {
    let full = calculate_workbook(session.workbook(), CalculationOptions::default());
    let incremental = session.calculation().expect("installed calculation");
    for address in addresses {
        let cell = CalculationCellId::new(sheet, cell_address(address));
        assert_eq!(
            incremental.cell(cell),
            full.cell(cell),
            "mismatch at {address}"
        );
    }
}

fn assert_number(
    calculation: Option<&cellrune::CalculationSnapshot>,
    sheet: SheetId,
    address: &str,
    expected: f64,
) {
    let cell = CalculationCellId::new(sheet, cell_address(address));
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.and_then(|snapshot| snapshot.cell(cell))
    else {
        panic!("expected numeric result at {address}");
    };
    assert_eq!(actual.get(), expected);
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
        panic!("expected an error result at {address}");
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
