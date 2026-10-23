use super::{CsvErrorCode, CsvReadOptions, CsvWriteOptions, read_csv, write_csv};
use crate::{
    CalculationOptions, CellAddress, CellContent, CellValue, EditBatch, ExcelError, FormulaText,
    SheetId, SheetName, WorkbookChange, WorkbookDraft, calculate_workbook,
};

fn literal(draft: &WorkbookDraft, address: &str) -> Option<CellValue> {
    let sheet = &draft.workbook().sheets()[0];
    sheet
        .cell(CellAddress::from_a1(address).expect("address"))
        .map(|cell| match cell.content() {
            CellContent::Literal(value) => value.clone(),
            CellContent::Formula(_) => panic!("CSV never creates formulas"),
        })
}

fn number(value: f64) -> Option<CellValue> {
    Some(CellValue::number(value).expect("finite"))
}

fn text(value: &str) -> Option<CellValue> {
    Some(CellValue::Text(value.to_owned()))
}

#[test]
fn reads_fields_by_the_documented_value_rule() {
    let input = "\u{feff}12,-3.5,.5,1e-3,007,\"007\",TRUE,false,#N/A,=SUM(A1),2024-01-02\r\n\
                 ,\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\",\"\", x \n\
                 \n\
                 last";
    let draft = read_csv(input.as_bytes(), &CsvReadOptions::default()).expect("read");
    assert_eq!(draft.workbook().sheets()[0].name().as_str(), "Sheet1");
    for (address, expected) in [
        ("A1", number(12.0)),
        ("B1", number(-3.5)),
        ("C1", number(0.5)),
        ("D1", number(0.001)),
        ("E1", number(7.0)),
        ("F1", text("007")),
        ("G1", Some(CellValue::Logical(true))),
        ("H1", Some(CellValue::Logical(false))),
        ("I1", Some(CellValue::Error(ExcelError::NotAvailable))),
        ("J1", text("=SUM(A1)")),
        ("K1", text("2024-01-02")),
        ("A2", None),
        ("B2", text("a,b")),
        ("C2", text("say \"hi\"")),
        ("D2", text("two\nlines")),
        ("E2", text("")),
        ("F2", text(" x ")),
        ("A3", None),
        ("A4", text("last")),
    ] {
        assert_eq!(literal(&draft, address), expected, "{address}");
    }
}

#[test]
fn reads_another_delimiter_and_sheet_name() {
    let options = CsvReadOptions::new()
        .with_delimiter(';')
        .expect("delimiter")
        .with_sheet_name(SheetName::new("Import").expect("name"));
    let draft = read_csv(b"1;2,5\r\n", &options).expect("read");
    assert_eq!(draft.workbook().sheets()[0].name().as_str(), "Import");
    assert_eq!(literal(&draft, "A1"), number(1.0));
    assert_eq!(literal(&draft, "B1"), text("2,5"));
}

#[test]
fn rejects_malformed_input_with_stable_codes() {
    for (input, code) in [
        (&b"\"open"[..], CsvErrorCode::UnterminatedQuote),
        (b"\"a\"b,c", CsvErrorCode::InvalidQuote),
        (b"a,\xff", CsvErrorCode::InvalidUtf8),
    ] {
        let error = read_csv(input, &CsvReadOptions::default()).expect_err("malformed");
        assert_eq!(error.code(), code);
    }
    let error =
        read_csv(b"1,2,3", &CsvReadOptions::default().with_max_cells(2)).expect_err("cell limit");
    assert_eq!(error.code(), CsvErrorCode::TooManyCells);
    let full_width = "x,".repeat(16_383) + "x";
    assert!(read_csv(full_width.as_bytes(), &CsvReadOptions::default()).is_ok());
    let wide = full_width + ",x";
    assert_eq!(
        read_csv(wide.as_bytes(), &CsvReadOptions::default())
            .err()
            .map(|error| error.code()),
        Some(CsvErrorCode::TooManyColumns)
    );
    for delimiter in ['"', '\n', 'é'] {
        assert_eq!(
            CsvReadOptions::new()
                .with_delimiter(delimiter)
                .expect_err("delimiter")
                .code(),
            CsvErrorCode::InvalidDelimiter
        );
    }
}

#[test]
fn writes_calculated_values_that_read_back_unchanged() {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("sheet");
    let set = |address: &str, value: CellValue| {
        WorkbookChange::set_cell_value(sheet, CellAddress::from_a1(address).expect("a1"), value)
    };
    draft
        .apply_changes(EditBatch::new([
            set("A1", CellValue::number(0.1).expect("number")),
            set("B1", CellValue::Text("007".to_owned())),
            set("C1", CellValue::Text("a,\"b\"".to_owned())),
            set("A2", CellValue::Logical(true)),
            set("B3", CellValue::Text(String::new())),
            set("C3", CellValue::Text("TRUE".to_owned())),
            WorkbookChange::set_cell_formula(
                sheet,
                CellAddress::from_a1("C2").expect("a1"),
                FormulaText::from_user_input("=A1*3").expect("formula"),
            ),
        ]))
        .expect("edits");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let bytes = write_csv(
        draft.workbook(),
        &calculation,
        sheet,
        CsvWriteOptions::default(),
    )
    .expect("write");
    assert_eq!(
        String::from_utf8(bytes.clone()).expect("utf8"),
        "0.1,\"007\",\"a,\"\"b\"\"\"\r\nTRUE,,0.30000000000000004\r\n,\"\",\"TRUE\"\r\n"
    );
    let reread = read_csv(&bytes, &CsvReadOptions::default()).expect("reread");
    for address in ["A1", "B1", "C1", "A2", "B3", "C3"] {
        assert_eq!(
            literal(&reread, address),
            literal(&draft, address),
            "{address}"
        );
    }
    assert_eq!(literal(&reread, "C2"), number(0.1 * 3.0));

    let stale = calculate_workbook(draft.workbook(), CalculationOptions::default());
    draft
        .apply_changes(EditBatch::new([set("D4", CellValue::Logical(false))]))
        .expect("edit");
    assert_eq!(
        write_csv(draft.workbook(), &stale, sheet, CsvWriteOptions::default())
            .expect_err("stale")
            .code(),
        CsvErrorCode::StaleCalculation
    );
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
            sheet,
            CellAddress::from_a1("D1").expect("a1"),
            FormulaText::from_user_input("=NOSUCHFUNCTION(1)").expect("formula"),
        )]))
        .expect("formula");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let error = write_csv(
        draft.workbook(),
        &calculation,
        sheet,
        CsvWriteOptions::default(),
    )
    .expect_err("unavailable");
    assert_eq!(error.code(), CsvErrorCode::UnavailableResult);
    assert_eq!(error.detail(), Some("D1"));
    assert_eq!(
        write_csv(
            draft.workbook(),
            &calculation,
            SheetId::new(9).expect("sheet"),
            CsvWriteOptions::default()
        )
        .expect_err("unknown sheet")
        .code(),
        CsvErrorCode::UnknownSheet
    );
}
