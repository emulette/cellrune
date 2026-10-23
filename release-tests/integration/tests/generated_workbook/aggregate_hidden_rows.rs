use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellValue,
    ExcelError, OpenOptions, calculate_workbook, open_xlsx_document_bytes,
};

use crate::support::generated_xlsx::generated_hidden_rows_workbook;

#[derive(Debug, Clone, Copy)]
enum Expected {
    Number(f64),
    Error(ExcelError),
}

#[test]
fn aggregate_skips_hidden_rows_errors_and_nested_totals_by_option() {
    let cases = [
        // Option 5 skips hidden rows only.
        ("_xlfn.AGGREGATE(9,5,A1:A3)", Expected::Number(40.0)),
        ("_xlfn.AGGREGATE(9,4,A1:A3)", Expected::Number(60.0)),
        // Option 6 skips errors but keeps the nested SUBTOTAL and the hidden rows.
        ("_xlfn.AGGREGATE(9,6,A1:A6)", Expected::Number(160.0)),
        // Option 3 skips hidden rows, errors, and nested totals.
        ("_xlfn.AGGREGATE(9,3,A1:A6)", Expected::Number(40.0)),
        (
            "_xlfn.AGGREGATE(9,0,A1:A6)",
            Expected::Error(ExcelError::DivisionByZero),
        ),
        ("_xlfn.AGGREGATE(14,6,A1:A6,2)", Expected::Number(40.0)),
        ("_xlfn.AGGREGATE(14,3,A1:A6,1)", Expected::Number(30.0)),
        // A calculated array keeps its hidden rows and nested totals.
        ("_xlfn.AGGREGATE(14,3,A1:A6*1,1)", Expected::Number(60.0)),
        ("_xlfn.AGGREGATE(12,1,A1:A3)", Expected::Number(20.0)),
        // Excel 16.114 keeps the rows a filter hides under options 0, 2, 4, and 6 and skips them
        // under the odd options (excel-measure RESULTS.md, section C).
        ("_xlfn.AGGREGATE(4,1,C11:C14)", Expected::Number(7.0)),
        ("_xlfn.AGGREGATE(4,4,C11:C14)", Expected::Number(50.0)),
        ("_xlfn.AGGREGATE(9,0,C11:C14)", Expected::Number(65.0)),
        ("_xlfn.AGGREGATE(9,7,C11:C14)", Expected::Number(15.0)),
        (
            "_xlfn.AGGREGATE(9,6,{1,2,3})",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(14,6,A1:A6)",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(20,6,A1:A3)",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(9,8,A1:A3)",
            Expected::Error(ExcelError::Value),
        ),
        ("_xlfn.AGGREGATE(16,6,A1:A6,0.5)", Expected::Number(30.0)),
        ("_xlfn.AGGREGATE(2,3,A1:A6)", Expected::Number(2.0)),
    ];
    let formulas = cases.map(|(formula, _)| formula);
    let document = open_xlsx_document_bytes(
        &generated_hidden_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let workbook = document.workbook();
    let sheet = &workbook.sheets()[0];
    let calculation = calculate_workbook(workbook, CalculationOptions::default());

    for (offset, (formula, expected)) in cases.iter().enumerate() {
        let cell = CalculationCellId::new(
            sheet.id(),
            CellAddress::from_indices(offset as u32 + 1, 5).expect("valid formula address"),
        );
        match (expected, calculation.cell(cell)) {
            (
                Expected::Number(expected),
                Some(CalculationCellResult::Value(CellValue::Number(actual))),
            ) => {
                assert!(
                    (actual.get() - expected).abs() < 1e-9,
                    "{formula}: {}",
                    actual.get()
                );
            }
            (
                Expected::Error(expected),
                Some(CalculationCellResult::Value(CellValue::Error(actual))),
            ) => {
                assert_eq!(actual, expected, "{formula}");
            }
            (_, actual) => panic!("{formula}: expected {expected:?}, got {actual:?}"),
        }
    }
}
