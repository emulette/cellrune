use std::collections::BTreeMap;

use super::super::materialization::MaterializationPlan;
use super::super::{RecalculationWritePolicy, WriteLimits};
use super::unchanged_sheets;
use crate::xlsx::package::PartPath;
use crate::{
    CalculationOptions, CalculationSnapshot, CellAddress, CellRange, CellValue, EditBatch,
    OpenOptions, RecalculationWriteOptions, SheetId, SheetName, TableName, WorkbookChange,
    WorkbookDraft, XlsxDocument, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_bytes,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("address")
}

/// A saved three-sheet workbook whose second sheet holds the table `Things`.
fn document() -> XlsxDocument {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();
    let second = draft
        .add_sheet(SheetName::new("S2").expect("name"))
        .expect("sheet");
    draft
        .add_sheet(SheetName::new("S3").expect("name"))
        .expect("sheet");
    for (sheet, cell, value) in [
        (first, "A1", CellValue::number(1.0).expect("number")),
        (second, "A1", CellValue::Text("h".to_owned())),
        (second, "A2", CellValue::number(2.0).expect("number")),
    ] {
        draft
            .set_cell_value(sheet, address(cell), value)
            .expect("value");
    }
    draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            second,
            CellRange::new(address("A1"), address("A2")).expect("range"),
            Some(TableName::new("Things").expect("table name")),
        )]))
        .expect("table");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open")
}

fn plan(draft: &WorkbookDraft) -> MaterializationPlan {
    let calculation = CalculationSnapshot::new(
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        draft.workbook(),
        CalculationOptions::default(),
    );
    MaterializationPlan::new(
        &calculation,
        RecalculationWritePolicy::RequireComplete,
        WriteLimits::default(),
    )
    .expect("empty materialization")
}

fn unchanged(document: &XlsxDocument, draft: &WorkbookDraft, replaced: &[&PartPath]) -> Vec<u32> {
    let replacements = replaced
        .iter()
        .map(|part| ((*part).clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    unchanged_sheets(document, draft, &plan(draft), &replacements)
        .keys()
        .map(|sheet| sheet.get())
        .collect()
}

#[test]
fn only_sheets_the_draft_and_the_save_left_alone_skip_verification() {
    let document = document();
    let ids = document
        .workbook()
        .sheets()
        .iter()
        .map(crate::Sheet::id)
        .collect::<Vec<SheetId>>();
    let part = |index: usize| document.worksheet_part_path(ids[index]).expect("part");
    let untouched = WorkbookDraft::from_document(&document);
    assert_eq!(unchanged(&document, &untouched, &[]), [1, 2, 3]);
    // A replaced worksheet, relationship, or table part makes its sheet verified.
    assert_eq!(unchanged(&document, &untouched, &[part(0)]), [2, 3]);
    let relationships = part(2).relationship_part().expect("relationships");
    assert_eq!(unchanged(&document, &untouched, &[&relationships]), [1, 2]);
    let table = document.workbook().sheets()[1].tables()[0].id();
    let table_part = document.table_part_path(table).expect("table part");
    assert_eq!(unchanged(&document, &untouched, &[table_part]), [1, 3]);
    // A replaced styles part changes how every sheet's cells read.
    let styles = PartPath::from_archive_name(
        document
            .package_summary()
            .styles_part()
            .expect("styles part")
            .as_str()
            .as_bytes(),
    )
    .expect("styles path");
    assert!(unchanged(&document, &untouched, &[&styles]).is_empty());

    let mut edited = WorkbookDraft::from_document(&document);
    edited
        .set_cell_value(
            ids[2],
            address("B1"),
            CellValue::number(5.0).expect("number"),
        )
        .expect("edit");
    edited
        .apply_changes(EditBatch::new([WorkbookChange::rename_table(
            table,
            TableName::new("Renamed").expect("table name"),
        )]))
        .expect("table rename");
    assert_eq!(unchanged(&document, &edited, &[]), [1]);
    let mut renamed = WorkbookDraft::from_document(&document);
    renamed
        .rename_sheet(ids[0], SheetName::new("First").expect("name"))
        .expect("rename");
    // A rename keeps the part; verification still reads every sheet's name.
    assert_eq!(unchanged(&document, &renamed, &[]), [1, 2, 3]);
}
