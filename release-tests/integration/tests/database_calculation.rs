use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationOptions,
    CancellationToken, CellAddress, CellValue, EditBatch, FormulaText, RecalculationMode, SheetId,
    WorkbookCalculationSession, WorkbookChange, calculate_workbook,
};

#[test]
fn database_formula_criteria_match_full_calculation_after_incremental_edits() {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                text_change(sheet, "A1", "Name"),
                text_change(sheet, "B1", "Amount"),
                text_change(sheet, "A2", "Alice"),
                number_change(sheet, "B2", 10.0),
                text_change(sheet, "A3", "Bob"),
                number_change(sheet, "B3", 20.0),
                text_change(sheet, "A4", "Alfred"),
                number_change(sheet, "B4", 30.0),
                text_change(sheet, "F1", "Rule"),
                formula_change(sheet, "F2", "=B2>15"),
                text_change(sheet, "G1", "Rule"),
                formula_change(sheet, "G2", "=$J$1>0"),
                number_change(sheet, "J1", 1.0),
                formula_change(sheet, "H1", "=DSUM(A1:B4,\"Amount\",F1:F2)"),
                formula_change(sheet, "H2", "=DSUM(A1:B4,\"Amount\",G1:G2)"),
            ]),
        )
        .expect("database workbook batch");
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    assert_number(session.calculation(), sheet, "H1", 50.0);
    assert_number(session.calculation(), sheet, "H2", 60.0);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "B2", 25.0)]),
        )
        .expect("first database record edit");
    let first_record_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("first record incremental calculation");
    assert_eq!(
        first_record_delta.mode(),
        CalculationExecutionMode::Incremental
    );
    assert_number(session.calculation(), sheet, "H1", 75.0);
    assert_matches_full(&session, sheet, &["H1", "H2"]);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "B3", 5.0)]),
        )
        .expect("database record edit");
    let record_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("database record incremental calculation");
    assert_eq!(record_delta.mode(), CalculationExecutionMode::Incremental);
    assert_number(session.calculation(), sheet, "H1", 55.0);
    assert_matches_full(&session, sheet, &["H1", "H2"]);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "B4", 1.0)]),
        )
        .expect("last database record edit");
    let last_record_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("last record incremental calculation");
    assert_eq!(
        last_record_delta.mode(),
        CalculationExecutionMode::Incremental
    );
    assert_number(session.calculation(), sheet, "H1", 25.0);
    assert_matches_full(&session, sheet, &["H1", "H2"]);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([formula_change(sheet, "F2", "=B2>20")]),
        )
        .expect("criteria formula edit");
    let criteria_delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("criteria incremental calculation");
    assert_eq!(criteria_delta.mode(), CalculationExecutionMode::Full);
    assert_number(session.calculation(), sheet, "H1", 25.0);
    assert_matches_full(&session, sheet, &["H1", "H2"]);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([number_change(sheet, "J1", 0.0)]),
        )
        .expect("absolute criteria dependency edit");
    let absolute_delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("absolute criteria incremental calculation");
    assert_eq!(absolute_delta.mode(), CalculationExecutionMode::Incremental);
    assert_number(session.calculation(), sheet, "H2", 0.0);
    assert_matches_full(&session, sheet, &["H1", "H2"]);
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

fn text_change(sheet: SheetId, address: &str, value: &str) -> WorkbookChange {
    WorkbookChange::set_cell_value(
        sheet,
        cell_address(address),
        CellValue::Text(value.to_owned()),
    )
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
