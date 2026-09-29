use cellrune::{
    CalculationCellResult, CalculationOptions, CellValue, ExcelError, calculate_workbook,
    scan_formula_capabilities,
};

use super::support::{cell_id, workbook_with_formulas};

#[test]
fn base_ten_and_base_two_logarithms_scale_the_principal_natural_logarithm() {
    for (formula, expected) in [
        (
            "IMLOG10(COMPLEX(3,4))",
            "0.698970004336019+0.402719196273373i",
        ),
        ("IMLOG2(COMPLEX(3,4))", "2.32192809488736+1.33780421245098i"),
        ("IMLOG2(\"3+4j\")", "2.32192809488736+1.33780421245098j"),
        ("IMLOG10(\"100\")", "2"),
        ("IMLOG2(\"8\")", "3"),
        ("IMLOG10(\"-10\")", "1+1.36437635384184i"),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(
            scan_formula_capabilities(&workbook).is_supported(),
            "{formula}"
        );
        let result = calculate_workbook(&workbook, CalculationOptions::default());
        assert_eq!(
            result.cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "{formula}"
        );
    }
}

#[test]
fn complex_logarithms_reject_zero_and_invalid_inumbers() {
    for (formula, expected) in [
        ("IMLOG10(\"0\")", ExcelError::Number),
        ("IMLOG2(\"0\")", ExcelError::Number),
        ("IMLOG10(\"abc\")", ExcelError::Number),
        ("IMLOG2(#N/A)", ExcelError::NotAvailable),
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_eq!(
            result.cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Error(expected))),
            "{formula}"
        );
    }
}
