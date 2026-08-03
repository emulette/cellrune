use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CancellationToken, CellAddress,
    CellValue, EditBatch, FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession,
    WorkbookChange,
};

#[test]
fn continuous_distributions_preserve_valid_extreme_finite_inputs() {
    let mut session = WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                formula_change(sheet, "A1", "=GAMMALN.PRECISE(1E-307)"),
                formula_change(sheet, "A2", "=GAMMA(1E-307)"),
                formula_change(sheet, "A3", "=BETA.DIST(0,1,1,TRUE,-1E308,1E308)"),
                formula_change(sheet, "A4", "=BETA.INV(0.5,1,1,-1E308,1E308)"),
                formula_change(sheet, "A5", "=BETA.DIST(0,1,1,FALSE,-1E308,1E308)"),
                formula_change(sheet, "A6", "=GAMMA.DIST(1E308,1,1E-308,TRUE)"),
                formula_change(sheet, "A7", "=GAMMA.DIST(1E-308,0.5,1E308,FALSE)"),
                formula_change(sheet, "A8", "=GAMMA.DIST(1E-308,0.001,1E308,TRUE)"),
            ]),
        )
        .expect("continuous-distribution regression workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("continuous-distribution regression calculation");

    let calculation = session.calculation().expect("installed calculation");
    assert_number(calculation, sheet, "A1", 706.893_623_549_172, 1e-10, 0.0);
    assert_number(calculation, sheet, "A2", 1e307, 0.0, 1e-12);
    assert_number(calculation, sheet, "A3", 0.5, 2e-15, 0.0);
    assert_number(calculation, sheet, "A4", 0.0, 0.0, 0.0);
    assert_number(calculation, sheet, "A5", 5e-309, 0.0, 1e-12);
    assert_number(calculation, sheet, "A6", 1.0, 0.0, 0.0);
    assert_number(
        calculation,
        sheet,
        "A7",
        0.564_189_583_547_784_1,
        0.0,
        1e-12,
    );
    assert_number(
        calculation,
        sheet,
        "A8",
        0.242_242_491_462_598_63,
        0.0,
        1e-12,
    );
}

fn assert_number(
    calculation: &cellrune::CalculationSnapshot,
    sheet: SheetId,
    address: &str,
    expected: f64,
    absolute_tolerance: f64,
    relative_tolerance: f64,
) {
    let cell = CalculationCellId::new(sheet, cell_address(address));
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) = calculation.cell(cell)
    else {
        panic!(
            "expected numeric result at {address}, got {:?}",
            calculation.cell(cell),
        );
    };
    let tolerance = absolute_tolerance.max(relative_tolerance * expected.abs());
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected result at {address}: expected {expected}, got {}",
        actual.get(),
    );
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
