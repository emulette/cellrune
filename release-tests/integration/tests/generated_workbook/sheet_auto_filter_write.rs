use cellrune::{
    CalculationOptions, CellAddress, CellRange, CellValue, DefinedNameScope, EditBatch,
    OpenOptions, RecalculationWriteOptions, SheetId, WorkbookChange, WorkbookDraft,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_column_widening_fixture;
use crate::support::sheet_structure_xlsx::generated_sheet_structure_fixture;

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn edited(source: &[u8], sheet: &str, filter: Option<&str>) -> (WorkbookDraft, SheetId) {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let sheet_id = draft.workbook().sheet_by_name(sheet).expect("sheet").id();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet_id,
            filter.map(range),
        )]))
        .expect("filter edit");
    (draft, sheet_id)
}

fn write(draft: &WorkbookDraft) -> Vec<u8> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, RecalculationWriteOptions::default())
        .expect("write")
        .bytes()
        .to_vec()
}

fn filter_database(workbook: &cellrune::WorkbookSnapshot, sheet_id: SheetId) -> Option<String> {
    workbook
        .defined_names()
        .iter()
        .find(|name| {
            name.scope() == DefinedNameScope::Sheet(sheet_id)
                && name.name() == "_xlnm._FilterDatabase"
        })
        .map(|name| name.formula().as_str().to_owned())
}

#[test]
fn a_changed_sheet_filter_keeps_the_settings_of_the_columns_it_still_covers() {
    let source = generated_column_widening_fixture();
    let (draft, sheet_id) = edited(&source, "Data", Some("C1:E10"));
    let output = write(&draft);
    let sheet = support::part_text(&output, "xl/worksheets/sheet1.xml");
    // Column B's filter leaves; column D's keeps its criteria as the second column.
    assert!(sheet.contains(r#"<autoFilter ref="C1:E10">"#), "{sheet}");
    assert!(!sheet.contains(r#"val="North""#), "{sheet}");
    assert!(
        sheet.contains(r#"<filterColumn colId="1"><customFilters>"#),
        "{sheet}"
    );
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    let filter = reopened
        .workbook()
        .sheet_by_id(sheet_id)
        .expect("Data")
        .auto_filter()
        .expect("filter");
    assert_eq!(filter.filter_column_ids(), [1]);
    assert_eq!(
        filter_database(reopened.workbook(), sheet_id).as_deref(),
        Some("Data!$C$1:$E$10")
    );
}

#[test]
fn a_removed_sheet_filter_leaves_with_its_filter_database_name() {
    let source = generated_column_widening_fixture();
    let (draft, sheet_id) = edited(&source, "Data", None);
    let output = write(&draft);
    let sheet = support::part_text(&output, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("autoFilter"), "{sheet}");
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    assert!(
        reopened
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("Data")
            .auto_filter()
            .is_none()
    );
    assert_eq!(filter_database(reopened.workbook(), sheet_id), None);
}

#[test]
fn a_new_sheet_filter_is_written_where_the_worksheet_schema_places_it() {
    let source = generated_sheet_structure_fixture();
    let (draft, sheet_id) = edited(&source, "S1", Some("A1:B7"));
    let output = write(&draft);
    let sheet = support::part_text(&output, "xl/worksheets/sheet1.xml");
    // Excel 16.114 writes `<autoFilter>` and a hidden `_xlnm._FilterDatabase` for a new filter;
    // the filter precedes the conditional format.
    assert!(
        sheet.contains(r#"</sheetData><autoFilter ref="A1:B7"/><conditionalFormatting"#),
        "{sheet}"
    );
    let workbook = support::part_text(&output, "xl/workbook.xml");
    assert!(
        workbook.contains(
            r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">'S1'!$A$1:$B$7</definedName>"#
        ),
        "{workbook}"
    );
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("S1")
            .auto_filter()
            .map(cellrune::SheetAutoFilter::range),
        Some(range("A1:B7"))
    );
}

#[test]
fn a_canonical_draft_writes_its_sheet_filter() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (cell, value) in [("A1", "k"), ("B1", "v"), ("A2", "x")] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(cell).expect("cell"),
                CellValue::Text(value.to_owned()),
            )
            .expect("cell");
    }
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet_id,
            Some(range("A1:B2")),
        )]))
        .expect("filter");
    let reopened = open_xlsx_document_bytes(&write(&draft), OpenOptions::default()).expect("open");
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .auto_filter()
            .map(cellrune::SheetAutoFilter::range),
        Some(range("A1:B2"))
    );
    assert_eq!(
        filter_database(reopened.workbook(), sheet_id).as_deref(),
        Some("Sheet1!$A$1:$B$2")
    );
}
