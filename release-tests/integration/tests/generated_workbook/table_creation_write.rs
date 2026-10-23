use cellrune::{
    CalculationOptions, CellAddress, CellRange, CellValue, EditBatch, OpenOptions,
    RecalculationWriteOptions, WorkbookChange, WorkbookDraft, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::table_structure_xlsx::generated_table_structure_fixture;

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn write(draft: &WorkbookDraft) -> Vec<u8> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, RecalculationWriteOptions::default())
        .expect("write")
        .bytes()
        .to_vec()
}

fn headed(draft: &mut WorkbookDraft, sheet: &str, cells: &[(&str, &str)]) -> cellrune::SheetId {
    let sheet_id = draft.workbook().sheet_by_name(sheet).expect("sheet").id();
    for (cell, value) in cells {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(cell).expect("cell"),
                CellValue::Text((*value).to_owned()),
            )
            .expect("cell");
    }
    sheet_id
}

#[test]
fn a_created_table_is_saved_as_a_new_part_beside_the_source_tables() {
    let source = generated_table_structure_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    // `S1` has no relationships yet; `S2` already holds `Things`.
    let s1 = headed(&mut draft, "S1", &[("C1", "h1"), ("D1", "h2"), ("C2", "x")]);
    let s2 = headed(&mut draft, "S2", &[("G1", "g"), ("G2", "y")]);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::create_table(s1, range("C1:D2"), None),
            WorkbookChange::create_table(s2, range("G1:G2"), None),
        ]))
        .expect("create tables");
    // Excel 16.114 names a table made beside one other `Table2`.
    assert!(draft.workbook().table("Table2").is_some());
    assert!(draft.workbook().table("Table3").is_some());
    let output = write(&draft);
    let parts = support::archive_parts(&output);
    let text = |part: &str| String::from_utf8(parts[part].clone()).expect("UTF-8");
    assert!(text("xl/worksheets/_rels/sheet1.xml.rels").contains("cellrune-table-2.xml"));
    assert!(text("xl/worksheets/sheet1.xml").contains(r#"<tableParts count="1">"#));
    let s2_xml = text("xl/worksheets/sheet2.xml");
    assert!(
        s2_xml.contains(r#"<tableParts count="2"><tablePart r:id="rId1"/>"#),
        "{s2_xml}"
    );
    assert!(text("[Content_Types].xml").contains(r#"PartName="/xl/tables/cellrune-table-3.xml""#));
    assert_eq!(
        parts["xl/tables/table1.xml"],
        support::archive_parts(&source)["xl/tables/table1.xml"]
    );

    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    for (name, expected) in [("Table2", range("C1:D2")), ("Table3", range("G1:G2"))] {
        let table = reopened.workbook().table(name).expect("created table");
        assert_eq!(table.range(), expected, "{name}");
        assert!(table.auto_filter().is_some(), "{name}");
    }
    assert!(reopened.workbook().table("Things").is_some());
}

#[test]
fn a_canonical_draft_and_an_added_sheet_save_their_created_tables() {
    let mut draft = WorkbookDraft::new();
    let sheet = headed(&mut draft, "Sheet1", &[("A1", "k"), ("A2", "x")]);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            sheet,
            range("A1:A2"),
            None,
        )]))
        .expect("create");
    let reopened = open_xlsx_document_bytes(&write(&draft), OpenOptions::default()).expect("open");
    assert_eq!(
        reopened
            .workbook()
            .table("Table1")
            .map(cellrune::Table::range),
        Some(range("A1:A2"))
    );

    let source = generated_table_structure_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let added = draft
        .add_sheet(cellrune::SheetName::new("Added").expect("name"))
        .expect("added");
    headed(&mut draft, "Added", &[("B2", "col"), ("B3", "v")]);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            added,
            range("B2:B3"),
            None,
        )]))
        .expect("create on the added sheet");
    let reopened = open_xlsx_document_bytes(&write(&draft), OpenOptions::default()).expect("open");
    assert_eq!(
        reopened
            .workbook()
            .table("Table2")
            .map(cellrune::Table::range),
        Some(range("B2:B3"))
    );
}
