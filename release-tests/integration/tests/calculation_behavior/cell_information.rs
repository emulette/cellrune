use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CellAddress, CellValue,
    FormulaText, WorkbookDraft, calculate_workbook,
};

use super::materialized_result;

fn calculate(formulas: &[(&str, &str)]) -> (cellrune::CalculationSnapshot, cellrune::SheetId) {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, value) in [
        ("B2", CellValue::Text("label".to_owned())),
        ("B3", CellValue::number(5.0).expect("finite literal")),
        ("B4", CellValue::Logical(true)),
    ] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid literal address"),
                value,
            )
            .expect("literal edit");
    }
    for (address, formula) in [("B5", "=1+1"), ("B6", "=\"x\"&\"y\"")]
        .iter()
        .chain(formulas)
    {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid formula address"),
                FormulaText::from_user_input(*formula).expect("valid formula"),
            )
            .expect("formula edit");
    }
    (
        calculate_workbook(draft.workbook(), CalculationOptions::default()),
        sheet_id,
    )
}

#[test]
fn cell_reports_address_position_contents_and_type_from_the_workbook() {
    let (calculation, sheet_id) = calculate(&[
        ("D1", "=CELL(\"address\",B3)"),
        ("D2", "=CELL(\"address\",B3:C9)"),
        ("D3", "=CELL(\"row\",B3)"),
        ("D4", "=CELL(\"col\",B3)"),
        ("D5", "=CELL(\"contents\",B2:B4)"),
        ("D6", "=CELL(\"contents\",B5)"),
        ("D7", "=CELL(\"type\",B2)"),
        ("D8", "=CELL(\"type\",B3)"),
        ("D9", "=CELL(\"type\",B4)"),
        ("D10", "=CELL(\"type\",B20)"),
        ("D11", "=CELL(\"type\",B5)"),
    ]);
    for (address, expected) in [
        ("D1", CellValue::Text("$B$3".to_owned())),
        ("D2", CellValue::Text("$B$3".to_owned())),
        ("D3", CellValue::number(3.0).expect("finite")),
        ("D4", CellValue::number(2.0).expect("finite")),
        ("D5", CellValue::Text("label".to_owned())),
        ("D6", CellValue::number(2.0).expect("finite")),
        ("D7", CellValue::Text("l".to_owned())),
        ("D8", CellValue::Text("v".to_owned())),
        ("D9", CellValue::Text("v".to_owned())),
        ("D10", CellValue::Text("b".to_owned())),
        ("D11", CellValue::Text("v".to_owned())),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(expected)),
            "{address}"
        );
    }
}

#[test]
fn cell_leaves_host_and_format_dependent_information_unsupported() {
    let formulas = [
        ("D1", "=CELL(\"filename\",B2)"),
        ("D2", "=CELL(\"format\",B3)"),
        ("D3", "=CELL(\"width\",B3)"),
        ("D4", "=CELL(\"protect\",B3)"),
        ("D5", "=CELL(\"row\")"),
        ("D6", "=CELL(\"row\",B3:C9)"),
        ("D7", "=CELL(\"contents\",B20)"),
        ("D8", "=CELL(\"type\",B6)"),
        ("D9", "=CELL(\"ADDRESS\",B3)"),
        ("D10", "=CELL(\"no such type\",B3)"),
    ];
    let (calculation, sheet_id) = calculate(&formulas);
    for (address, formula) in formulas {
        assert!(
            matches!(
                materialized_result(&calculation, sheet_id, address),
                Some(CalculationCellResult::Unavailable(issue))
                    if issue.code() == CalculationIssueCode::UnsupportedExpression
            ),
            "{formula}"
        );
    }
}
