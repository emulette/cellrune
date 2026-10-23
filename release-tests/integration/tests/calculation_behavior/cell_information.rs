use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CellAddress, CellValue,
    FormulaText, NumberFormat, NumberFormatKind, WorkbookDraft, calculate_workbook,
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
        // Excel 16.114 for Mac: the upper-left row of a range, 0 for an empty cell's contents,
        // "l" for a formula's text result, and the info type in any case.
        ("D12", "=CELL(\"row\",B3:C9)"),
        ("D13", "=CELL(\"contents\",B20)"),
        ("D14", "=CELL(\"type\",B6)"),
        ("D15", "=CELL(\"ADDRESS\",B3)"),
        ("D16", "=CELL(\"Row\",B3)"),
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
        ("D12", CellValue::number(3.0).expect("finite")),
        ("D13", CellValue::number(0.0).expect("finite")),
        ("D14", CellValue::Text("l".to_owned())),
        ("D15", CellValue::Text("$B$3".to_owned())),
        ("D16", CellValue::number(3.0).expect("finite")),
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
        // An empty position takes its row or column style, which is not a calculation input.
        ("D2", "=CELL(\"format\",B20)"),
        ("D3", "=CELL(\"width\",B3)"),
        ("D4", "=CELL(\"protect\",B3)"),
        // An omitted or empty reference reads the selection at calculation time.
        ("D5", "=CELL(\"row\")"),
        ("D6", "=CELL(\"row\",)"),
        // Excel was not measured on a multi-cell "col" or "type".
        ("D7", "=CELL(\"col\",B3:C9)"),
        ("D8", "=CELL(\"type\",B3:C9)"),
        ("D9", "=CELL(\"prefix\",B3)"),
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

#[test]
fn cell_reports_the_documented_codes_of_number_formats() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let built_in = |id| NumberFormat::built_in(id, NumberFormatKind::Number).expect("built-in");
    let custom =
        |code| NumberFormat::custom(164, code, NumberFormatKind::Number).expect("custom format");
    let cases = [
        (NumberFormat::default(), "G", 0.0),
        (built_in(2), "F2", 0.0),
        (built_in(4), ",2", 0.0),
        (built_in(6), "C0-", 1.0),
        (built_in(10), "P2", 0.0),
        (built_in(11), "S2", 0.0),
        (built_in(14), "D4", 0.0),
        (built_in(15), "D1", 0.0),
        (built_in(17), "D3", 0.0),
        (built_in(20), "D9", 0.0),
        (built_in(22), "D4", 0.0),
        (
            custom("\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)"),
            "C2-",
            1.0,
        ),
        (custom("0.00"), "F2", 0.0),
        (custom("H:MM:SS"), "D8", 0.0),
        // Excel 16.114 for Mac.
        (custom("yyyy-mm-dd"), "G", 0.0),
    ];
    for (row, (number_format, _, _)) in (1..).zip(&cases) {
        let address = CellAddress::from_indices(row, 1).expect("valid source address");
        draft
            .set_cell_value(
                sheet_id,
                address,
                CellValue::number(1.0).expect("finite literal"),
            )
            .expect("literal edit");
        draft
            .set_cell_number_format(sheet_id, address, number_format.clone())
            .expect("number format edit");
        for (column, info_type) in [(2, "format"), (3, "color"), (4, "parentheses")] {
            draft
                .set_cell_formula(
                    sheet_id,
                    CellAddress::from_indices(row, column).expect("valid formula address"),
                    FormulaText::from_user_input(format!("=CELL(\"{info_type}\",A{row})"))
                        .expect("valid formula"),
                )
                .expect("formula edit");
        }
    }
    let undocumented = CellAddress::from_a1("A20").expect("valid address");
    draft
        .set_cell_value(
            sheet_id,
            undocumented,
            CellValue::number(1.0).expect("finite literal"),
        )
        .expect("literal edit");
    draft
        .set_cell_number_format(sheet_id, undocumented, custom("0.000"))
        .expect("number format edit");
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("B20").expect("valid formula address"),
            FormulaText::from_user_input("=CELL(\"format\",A20)").expect("valid formula"),
        )
        .expect("formula edit");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (row, (number_format, code, color)) in (1..).zip(cases) {
        for (column, expected) in [
            ("B", CellValue::Text(code.to_owned())),
            ("C", CellValue::number(color).expect("finite")),
            ("D", CellValue::number(0.0).expect("finite")),
        ] {
            assert_eq!(
                materialized_result(&calculation, sheet_id, &format!("{column}{row}")),
                Some(&CalculationCellResult::Value(expected)),
                "{number_format:?} at {column}{row}"
            );
        }
    }
    assert!(matches!(
        materialized_result(&calculation, sheet_id, "B20"),
        Some(CalculationCellResult::Unavailable(issue))
            if issue.code() == CalculationIssueCode::UnsupportedExpression
    ));
}
