use cellrune::{
    CalculationOptions, CellAddress, CellValue, FormulaText, NumberFormat, NumberFormatKind,
    OpenOptions, PhoneticWriteOptions, RecalculationWriteOptions, SheetName, WorkbookDraft,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_row_column_style_workbook;

const SHEET_PART: &str = "xl/worksheets/sheet1.xml";
const STYLES_PART: &str = "xl/styles.xml";

#[test]
fn new_cells_take_their_row_or_column_format_and_existing_cells_keep_theirs() {
    let source = generated_row_column_style_workbook();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let sheet_id = document.workbook().sheets()[0].id();
    let address = |text| CellAddress::from_a1(text).expect("address");
    let number = |value| CellValue::number(value).expect("finite value");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(sheet_id, address("B2"), number(1.5))
        .expect("B2");
    draft
        .set_cell_formula(
            sheet_id,
            address("A2"),
            FormulaText::from_xlsx("D2*2").expect("formula"),
        )
        .expect("A2");
    draft
        .set_cell_value(sheet_id, address("B3"), number(0.25))
        .expect("B3");
    draft
        .set_cell_value(sheet_id, address("C3"), number(3.0))
        .expect("C3");
    draft
        .set_cell_value(sheet_id, address("B4"), number(0.5))
        .expect("B4");
    draft
        .set_annotated_text(
            sheet_id,
            address("A5"),
            "text",
            Vec::new(),
            PhoneticWriteOptions::new(false),
        )
        .expect("A5");
    draft
        .set_cell_value(sheet_id, address("C2"), number(2.0))
        .expect("C2");
    draft
        .set_cell_number_format(
            sheet_id,
            address("C2"),
            NumberFormat::custom(165, "0.000", NumberFormatKind::Number).expect("format"),
        )
        .expect("format C2");
    for cell in ["B1", "C1", "A1", "D2"] {
        draft
            .set_cell_value(sheet_id, address(cell), number(0.75))
            .expect("value in a styled cell");
    }
    let added = draft
        .add_sheet(SheetName::new("Added").expect("name"))
        .expect("add sheet");
    draft
        .set_cell_value(added, address("B2"), number(1.0))
        .expect("added B2");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("styled output");

    // A new cell takes its row's style when the row declares customFormat, else its column's
    // style. A cell element already in the sheet, blank or not, keeps its own style.
    let sheet = support::part_text(output.bytes(), SHEET_PART);
    for (cell, style) in [
        ("B2", Some("1")),
        ("A2", Some("1")),
        ("B3", Some("2")),
        ("C3", None),
        ("B4", Some("2")),
        ("A5", Some("1")),
        ("C2", Some("3")),
        ("B1", None),
        ("C1", Some("1")),
        ("A1", Some("2")),
        ("D2", None),
    ] {
        assert_eq!(style_attribute(&sheet, cell), style, "{cell} in {sheet}");
    }
    for element in [
        r#"<cols><col min="2" max="2" width="12" style="2" customWidth="1"/></cols>"#,
        r#"<row r="2" s="1" customFormat="1">"#,
        r#"<row r="4" s="1">"#,
        r#"<row r="5" s="1" customFormat="1">"#,
    ] {
        assert!(sheet.contains(element), "{element} in {sheet}");
    }

    // The number-format edit builds on the bold row style instead of the default style.
    let styles = support::part_text(output.bytes(), STYLES_PART);
    let added_style = styles.split("<xf ").nth(4).expect("one style added for C2");
    assert!(
        added_style.contains(r#"numFmtId="165""#) && added_style.contains(r#"fontId="1""#),
        "{styles}"
    );

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    let styled = reopened
        .workbook()
        .sheet_by_name("Styled")
        .expect("styled sheet");
    for (cell, id) in [
        ("B2", 2),
        ("A2", 2),
        ("B3", 10),
        ("C3", 0),
        ("B4", 10),
        ("A5", 2),
        ("C2", 165),
        ("B1", 0),
        ("C1", 2),
        ("A1", 10),
        ("D2", 0),
    ] {
        assert_eq!(
            styled
                .cell(address(cell))
                .expect("written cell")
                .number_format()
                .id(),
            id,
            "{cell}"
        );
    }
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_name("Added")
            .expect("added sheet")
            .cell(address("B2"))
            .expect("added cell")
            .number_format()
            .id(),
        0
    );
}

#[test]
fn a_cell_written_after_its_source_cell_is_cleared_takes_the_new_cell_format() {
    let document = open_xlsx_document_bytes(
        &generated_row_column_style_workbook(),
        OpenOptions::default(),
    )
    .expect("open");
    let sheet_id = document.workbook().sheets()[0].id();
    let address = |text| CellAddress::from_a1(text).expect("address");
    let mut draft = WorkbookDraft::from_document(&document);
    for cell in ["A1", "D2"] {
        draft.clear_cell(sheet_id, address(cell)).expect("clear");
        draft
            .set_cell_value(
                sheet_id,
                address(cell),
                CellValue::number(2.0).expect("finite value"),
            )
            .expect("rewrite");
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("rewritten output");

    // The cleared cells' own styles go with them: A1 now takes the default style and D2 its
    // formatted row's style.
    let sheet = support::part_text(output.bytes(), SHEET_PART);
    assert_eq!(style_attribute(&sheet, "A1"), None, "{sheet}");
    assert_eq!(style_attribute(&sheet, "D2"), Some("1"), "{sheet}");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    let styled = &reopened.workbook().sheets()[0];
    for (cell, id) in [("A1", 0), ("D2", 2)] {
        assert_eq!(
            styled
                .cell(address(cell))
                .expect("rewritten cell")
                .number_format()
                .id(),
            id,
            "{cell}"
        );
    }
}

/// Returns the `s` attribute of the cell element at `address`.
fn style_attribute<'a>(sheet: &'a str, address: &str) -> Option<&'a str> {
    let start = sheet
        .find(&format!(r#"<c r="{address}""#))
        .unwrap_or_else(|| panic!("{address} is written"));
    let element = &sheet[start..start + sheet[start..].find('>').expect("element end")];
    element
        .split_once(r#" s=""#)
        .map(|(_, rest)| &rest[..rest.find('"').expect("attribute end")])
}
