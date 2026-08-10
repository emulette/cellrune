use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CalculationSnapshot, CellAddress, CellValue, ExcelError, FormulaText,
    SheetId, WorkbookDraft, calculate_workbook, scan_formula_capabilities,
};

#[test]
fn bessel_worksheet_functions_coerce_truncate_and_preserve_parity() {
    let (sheet, workbook) = workbook_with_formulas(&[
        ("A1", "=BESSELI(\"1.5\",2)"),
        ("A2", "=BESSELJ(1.5,2)"),
        ("A3", "=BESSELK(1.5,2)"),
        ("A4", "=BESSELY(1.5,2)"),
        ("A5", "=BESSELI(-1.5,3)=-BESSELI(1.5,3)"),
        ("A6", "=BESSELJ(-1.5,3)=-BESSELJ(1.5,3)"),
        ("A7", "=BESSELJ(1.5,2.9)=BESSELJ(1.5,2)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // mpmath 1.4.1, 80 decimal places. These are intentionally mathematical
    // references rather than Excel's legacy approximation values.
    assert_number(&calculation, sheet, "A1", 0.337_834_618_335_680_74, 2e-13);
    assert_number(&calculation, sheet, "A2", 0.232_087_672_144_214_72, 2e-13);
    assert_number(&calculation, sheet, "A3", 0.583_655_963_256_650_8, 2e-13);
    assert_number(&calculation, sheet, "A4", -0.932_193_759_762_973_9, 2e-13);
    for address in ["A5", "A6", "A7"] {
        assert_logical(&calculation, sheet, address, true);
    }
}

#[test]
fn bessel_frozen_excel_observations_use_their_scoped_abs_rel_tolerance() {
    let (sheet, workbook) = workbook_with_formulas(&[
        ("A1", "=BESSELI(1.5,2)"),
        ("A2", "=BESSELJ(1.5,2)"),
        ("A3", "=BESSELK(1.5,2)"),
        ("A4", "=BESSELY(1.5,2)"),
        ("A5", "=BESSELI(\"2\",2)"),
        ("A6", "=BESSELJ(\"2\",2)"),
        ("A7", "=BESSELK(\"2\",2)"),
        ("A8", "=BESSELY(\"2\",2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // These eight values are saved Excel observations, not the high-precision
    // kernel oracle. The release policy scopes their wider absolute tolerance
    // to this fixture only.
    for (address, expected) in [
        ("A1", 0.337_834_620_874_438_16),
        ("A2", 0.232_087_679_017_447_95),
        ("A3", 0.583_655_974_166_665_6),
        ("A4", -0.932_193_760_626_233_9),
        ("A5", 0.688_948_449_197_763_3),
        ("A6", 0.352_834_207_514_175_6),
        ("A7", 0.253_759_76),
        ("A8", -0.617_408_098_385_051_2),
    ] {
        assert_number_abs_rel(&calculation, sheet, address, expected, 2e-7, 5e-13);
    }
}

#[test]
fn bessel_worksheet_functions_enforce_their_domains() {
    let (sheet, workbook) = workbook_with_formulas(&[
        ("A1", "=BESSELI(1.5,-1)"),
        ("A2", "=BESSELJ(1.5,100001)"),
        ("A3", "=BESSELK(0,2)"),
        ("A4", "=BESSELY(0,2)"),
        ("A5", "=BESSELI(\"not a number\",2)"),
        ("A6", "=BESSELJ(1.5,-1)"),
        ("A7", "=BESSELK(1.5,-1)"),
        ("A8", "=BESSELY(1.5,-1)"),
        ("A9", "=BESSELI(1.5,-0.9)"),
        ("A10", "=BESSELJ(1.5,-0.9)"),
        ("A11", "=BESSELK(1.5,-0.9)"),
        ("A12", "=BESSELY(1.5,-0.9)"),
        ("A13", "=BESSELJ(1.5,\"2\")"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for address in [
        "A1", "A2", "A3", "A4", "A6", "A7", "A8", "A9", "A10", "A11", "A12",
    ] {
        assert_error(&calculation, sheet, address, ExcelError::Number);
    }
    assert_error(&calculation, sheet, "A5", ExcelError::Value);
    assert_number(&calculation, sheet, "A13", 0.232_087_672_144_214_72, 2e-13);
}

#[test]
fn bessel_worksheet_functions_preserve_extreme_finite_results() {
    let (sheet, workbook) =
        workbook_with_formulas(&[("A1", "=BESSELK(100,500)"), ("A2", "=BESSELY(9E307,0)")]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // mpmath 1.3.0 at 120 dps, using the exact binary64 worksheet inputs.
    assert_number_abs_rel(
        &calculation,
        sheet,
        "A1",
        2.731_383_171_990_178_5e279,
        0.0,
        2e-11,
    );
    assert_number_abs_rel(
        &calculation,
        sheet,
        "A2",
        4.066_895_414_404_214e-155,
        0.0,
        2e-11,
    );
}

#[test]
fn bessel_worksheet_functions_charge_the_function_iteration_budget() {
    let (sheet, workbook) = workbook_with_formulas(&[("A1", "=BESSELJ(1.5,20)")]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(1)
        .expect("positive function iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    let cell = CalculationCellId::new(sheet, cell_address("A1"));
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell) else {
        panic!("BESSELJ must stop when its function-iteration budget is exhausted");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_function_iterations"));
}

fn workbook_with_formulas(formulas: &[(&str, &str)]) -> (SheetId, cellrune::WorkbookSnapshot) {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (address, formula) in formulas {
        draft
            .set_cell_formula(
                sheet,
                cell_address(address),
                FormulaText::from_user_input(*formula).expect("valid Bessel test formula"),
            )
            .expect("Bessel formula mutation");
    }
    (sheet, draft.workbook().clone())
}

fn assert_number(
    calculation: &CalculationSnapshot,
    sheet: SheetId,
    address: &str,
    expected: f64,
    tolerance: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.cell(CalculationCellId::new(sheet, cell_address(address)))
    else {
        panic!("expected a numeric Bessel result at {address}");
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected Bessel result at {address}: expected {expected}, got {}",
        actual.get(),
    );
}

fn assert_number_abs_rel(
    calculation: &CalculationSnapshot,
    sheet: SheetId,
    address: &str,
    expected: f64,
    absolute: f64,
    relative: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.cell(CalculationCellId::new(sheet, cell_address(address)))
    else {
        panic!("expected a numeric Bessel result at {address}");
    };
    let tolerance = absolute + relative * expected.abs();
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected frozen Bessel result at {address}: expected {expected}, got {}",
        actual.get(),
    );
}

fn assert_logical(
    calculation: &CalculationSnapshot,
    sheet: SheetId,
    address: &str,
    expected: bool,
) {
    assert_eq!(
        calculation.cell(CalculationCellId::new(sheet, cell_address(address))),
        Some(&CalculationCellResult::Value(CellValue::Logical(expected))),
        "unexpected Bessel logical result at {address}",
    );
}

fn assert_error(
    calculation: &CalculationSnapshot,
    sheet: SheetId,
    address: &str,
    expected: ExcelError,
) {
    let Some(CalculationCellResult::Value(CellValue::Error(actual))) =
        calculation.cell(CalculationCellId::new(sheet, cell_address(address)))
    else {
        panic!("expected a Bessel error result at {address}");
    };
    assert_eq!(*actual, expected, "unexpected Bessel error at {address}");
}

fn cell_address(address: &str) -> CellAddress {
    CellAddress::from_a1(address).expect("valid Bessel test address")
}
