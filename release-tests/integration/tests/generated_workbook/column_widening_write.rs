use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellRange,
    CellValue, EditBatch, OpenOptions, RecalculationWriteOptions, TableId, WorkbookChange,
    WorkbookDraft, WorkbookSnapshot, XlsxDocument, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_column_widening_fixture;

const DATA_PART: &str = "xl/worksheets/sheet1.xml";
const TABLE_PART: &str = "xl/tables/table1.xml";
const WORKBOOK_PART: &str = "xl/workbook.xml";
const FILTER_COLUMNS: &str = r#"<filterColumn colId="0"><filters><filter val="North"/></filters></filterColumn><filterColumn colId="2"><customFilters><customFilter operator="notEqual" val="n4"/></customFilters></filterColumn>"#;
const SORT_STATE: &str =
    r#"<sortState ref="B2:D10"><sortCondition descending="1" ref="C2:C10"/></sortState>"#;

#[test]
fn widening_a_sheet_auto_filter_right_keeps_its_filter_and_moves_the_filter_database_name() {
    let (output, reopened) = widen(|document| {
        WorkbookChange::widen_sheet_auto_filter(sheet_id(document, "Data"), range("B1:F10"))
    });

    assert!(
        support::part_text(&output, DATA_PART).contains(&format!(
            r#"<autoFilter ref="B1:F10">{FILTER_COLUMNS}{SORT_STATE}</autoFilter>"#
        )),
        "the filter widens and keeps its criteria and sort state byte for byte"
    );
    assert!(support::part_text(&output, WORKBOOK_PART).contains(
        r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$B$1:$F$10</definedName>"#
    ));
    let filter = reopened
        .workbook()
        .sheet_by_name("Data")
        .and_then(|sheet| sheet.auto_filter())
        .expect("reopened sheet auto-filter");
    assert_eq!(filter.range(), range("B1:F10"));
    assert_eq!(filter.filter_column_ids(), [0, 2]);
    assert_eq!(
        reopened.workbook().defined_names()[0].formula().as_str(),
        "Data!$B$1:$F$10"
    );
}

#[test]
fn widening_a_sheet_auto_filter_left_renumbers_its_filter_columns() {
    let (output, reopened) = widen(|document| {
        WorkbookChange::widen_sheet_auto_filter(sheet_id(document, "Data"), range("A1:D10"))
    });

    let shifted = FILTER_COLUMNS
        .replace(r#"colId="2""#, r#"colId="3""#)
        .replace(r#"colId="0""#, r#"colId="1""#);
    assert!(
        support::part_text(&output, DATA_PART).contains(&format!(
            r#"<autoFilter ref="A1:D10">{shifted}{SORT_STATE}</autoFilter>"#
        )),
        "filter columns count from the new first column; absolute sort ranges stay"
    );
    assert!(support::part_text(&output, WORKBOOK_PART).contains(">Data!$A$1:$D$10</definedName>"));
    let filter = reopened
        .workbook()
        .sheet_by_name("Data")
        .and_then(|sheet| sheet.auto_filter())
        .expect("reopened sheet auto-filter");
    assert_eq!(filter.range(), range("A1:D10"));
    assert_eq!(filter.filter_column_ids(), [1, 3]);
}

#[test]
fn widening_a_table_right_adds_its_header_as_a_column_with_a_fresh_id() {
    let source = open();
    assert_eq!(number_at(source.workbook(), "G1"), 3.0);
    let (output, reopened) =
        widen(|_| WorkbookChange::widen_table_columns(table_id(), range("B1:E4")));

    let table_xml = support::part_text(&output, TABLE_PART);
    assert!(table_xml.contains(r#"ref="B1:E4""#), "{table_xml}");
    assert!(
        table_xml.contains(r#"<autoFilter ref="B1:E4"><filterColumn colId="1">"#),
        "{table_xml}"
    );
    assert!(
        table_xml.contains(r#"<tableColumns count="4">"#),
        "{table_xml}"
    );
    assert_in_order(
        &table_xml,
        &[
            r#"<tableColumn id="1" name="Region"/>"#,
            r#"<tableColumn id="5" name="Amount"/>"#,
            r#"<tableColumn id="3" name="Note"/>"#,
            r#"<tableColumn id="6" name="Margin"/></tableColumns>"#,
        ],
    );
    assert_table_columns(
        reopened.workbook(),
        &[(1, "Region"), (5, "Amount"), (3, "Note"), (6, "Margin")],
    );
    assert_eq!(number_at(reopened.workbook(), "G1"), 4.0);
    assert_eq!(number_at(reopened.workbook(), "H1"), 6.0);
}

#[test]
fn widening_a_table_on_both_sides_inserts_columns_in_order_and_renumbers_its_filter() {
    let (output, reopened) =
        widen(|_| WorkbookChange::widen_table_columns(table_id(), range("A1:E4")));

    let table_xml = support::part_text(&output, TABLE_PART);
    assert!(
        table_xml.contains(r#"<autoFilter ref="A1:E4"><filterColumn colId="2">"#),
        "{table_xml}"
    );
    assert!(
        table_xml.contains(r#"<tableColumns count="5">"#),
        "{table_xml}"
    );
    assert_in_order(
        &table_xml,
        &[
            r#"<tableColumn id="6" name="Code"/><tableColumn id="1" name="Region"/>"#,
            r#"<tableColumn id="5" name="Amount"/>"#,
            r#"<tableColumn id="3" name="Note"/>"#,
            r#"<tableColumn id="7" name="Margin"/></tableColumns>"#,
        ],
    );
    assert_table_columns(
        reopened.workbook(),
        &[
            (6, "Code"),
            (1, "Region"),
            (5, "Amount"),
            (3, "Note"),
            (7, "Margin"),
        ],
    );
    let table = &reopened
        .workbook()
        .sheet_by_name("Sales")
        .expect("sheet")
        .tables()[0];
    let filter = table.auto_filter().expect("table auto-filter");
    assert_eq!(filter.range(), range("A1:E4"));
    assert_eq!(filter.filter_columns()[0].column_id(), 2);
    assert_eq!(number_at(reopened.workbook(), "G1"), 5.0);
}

fn open() -> XlsxDocument {
    open_xlsx_document_bytes(&generated_column_widening_fixture(), OpenOptions::default())
        .expect("column widening fixture")
}

/// Applies one change to the fixture, writes it, and reopens the output.
fn widen(change: impl FnOnce(&XlsxDocument) -> WorkbookChange) -> (Vec<u8>, XlsxDocument) {
    let document = open();
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new([change(&document)]))
        .expect("widen");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write widened draft")
        .bytes()
        .to_vec();
    let reopened =
        open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen widened output");
    (output, reopened)
}

fn sheet_id(document: &XlsxDocument, name: &str) -> cellrune::SheetId {
    document.workbook().sheet_by_name(name).expect("sheet").id()
}

fn table_id() -> TableId {
    TableId::new(1).expect("table ID")
}

fn range(a1: &str) -> CellRange {
    let (start, end) = a1.split_once(':').expect("A1 range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn assert_in_order(text: &str, parts: &[&str]) {
    let mut from = 0;
    for part in parts {
        let offset = text[from..]
            .find(part)
            .unwrap_or_else(|| panic!("{part} after byte {from} in {text}"));
        from += offset + part.len();
    }
}

fn assert_table_columns(workbook: &WorkbookSnapshot, expected: &[(u32, &str)]) {
    let table = &workbook.sheet_by_name("Sales").expect("sheet").tables()[0];
    let columns = table
        .columns()
        .iter()
        .map(|column| (column.id(), column.name()))
        .collect::<Vec<_>>();
    assert_eq!(columns, expected);
}

fn number_at(workbook: &WorkbookSnapshot, address: &str) -> f64 {
    let sheet = workbook.sheet_by_name("Sales").expect("sheet").id();
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    match calculation.cell(CalculationCellId::new(
        sheet,
        CellAddress::from_a1(address).expect("valid address"),
    )) {
        Some(CalculationCellResult::Value(CellValue::Number(number))) => number.get(),
        other => panic!("{address}: expected a number, got {other:?}"),
    }
}
