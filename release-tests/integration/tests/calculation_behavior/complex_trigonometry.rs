use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CellValue, ExcelError,
    calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, cell_id, workbook_with_formulas};

#[test]
fn complex_trigonometry_formats_fifteen_significant_digits() {
    for (formula, expected) in [
        // Microsoft's examples.
        ("IMSIN(\"4+3i\")", "-7.61923172032141-6.548120040911i"),
        ("IMCOS(\"1+i\")", "0.833730025131149-0.988897705762865i"),
        (
            "_xlfn.IMTAN(\"4+3i\")",
            "0.00490825806749606+1.00070953606723i",
        ),
        ("IMCOT(\"4+3i\")", "0.00490118239430447-0.999266927805902i"),
        // The Oracle's results for 3+4i.
        ("IMSIN(COMPLEX(3,4))", "3.85373803791938-27.0168132580039i"),
        ("IMCOS(COMPLEX(3,4))", "-27.0349456030742-3.85115333481178i"),
        (
            "IMTAN(COMPLEX(3,4))",
            "-0.000187346204629478+0.999355987381473i",
        ),
        (
            "IMCOT(COMPLEX(3,4))",
            "-0.000187587737983659-1.00064439247156i",
        ),
        (
            "IMSEC(COMPLEX(3,4))",
            "-0.0362534969158689+0.00516434460775318i",
        ),
        (
            "IMCSC(COMPLEX(3,4))",
            "0.0051744731840194+0.036275889628626i",
        ),
        ("IMSINH(COMPLEX(3,4))", "-6.548120040911-7.61923172032141i"),
        (
            "IMSECH(COMPLEX(3,4))",
            "-0.065294027857947+0.0752249603027732i",
        ),
        (
            "IMCSCH(COMPLEX(3,4))",
            "-0.0648774713706355+0.0754898329158637i",
        ),
        // Excel 16.114 for Mac near the poles of real arguments, where cos 2a ± cosh 2b cancels.
        ("IMTAN(\"1.5707963\")", "37320539.6343548"),
        ("IMCOT(\"0.00001\")", "99999.9999966667"),
        ("IMCOT(\"1E-9\")", "1000000000"),
        ("IMCOT(\"3.14159\")", "-376847.997569741"),
        // Nearly real arguments, rounded from 40-digit references.
        (
            "IMTAN(\"1.5707963+0.000001i\")",
            "26775.6725495693+999282.548623373i",
        ),
        (
            "IMCOT(\"0.00001+0.0000001i\")",
            "99990.0009965667-999.900010032333i",
        ),
        // Real arguments and the j suffix.
        ("IMSIN(\"2j\")", "3.62686040784702j"),
        ("IMTAN(\"0\")", "0"),
        ("IMSEC(\"0\")", "1"),
        ("IMSINH(\"0\")", "0"),
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
fn complex_trigonometry_rejects_logical_and_invalid_inumbers() {
    for (formula, expected) in [
        ("IMSIN(TRUE)", ExcelError::Value),
        ("IMCOT(FALSE)", ExcelError::Value),
        ("IMSECH(\"x\")", ExcelError::Number),
        ("IMCOS(\"1+2k\")", ExcelError::Number),
        ("IMSINH(#N/A)", ExcelError::NotAvailable),
        // The unbounded functions overflow like IMEXP.
        ("IMSIN(\"1+1000i\")", ExcelError::Number),
        ("IMSINH(\"1000\")", ExcelError::Number),
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
    for formula in [
        "IMCOT(\"0\")",
        "IMCSC(\"0\")",
        "IMCSCH(\"0\")",
        "IMTAN(\"1+400i\")",
        "IMSEC(\"1+1000i\")",
        "IMSECH(\"1000\")",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
}
