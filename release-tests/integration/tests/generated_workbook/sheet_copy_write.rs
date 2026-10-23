use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, FormulaText,
    OpenOptions, RecalculationWriteOptions, RecalculationWritePolicy, WorkbookDraft,
    XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::sheet_deletion_xlsx::generated_sheet_deletion_fixture;

/// The sheet deletion fixture without the pivot table on `S2`, which a copy cannot duplicate:
/// `S2` keeps the `Things` table, a drawing with a chart of `S2` and an image, and comments.
fn fixture() -> Vec<u8> {
    support::replace_part_text(
        &generated_sheet_deletion_fixture(),
        "xl/worksheets/_rels/sheet2.xml.rels",
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/pivotTable1.xml"/>"#,
        "",
    )
}

fn copied(source: &[u8], sheet: &str, position: usize) -> (WorkbookDraft, cellrune::SheetId) {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let sheet_id = draft.workbook().sheet_by_name(sheet).expect("sheet").id();
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("H1").expect("H1"),
            FormulaText::from_xlsx("'S2'!A1*2").expect("formula"),
        )
        .expect("self-reference");
    let copy = draft.copy_sheet(sheet_id, position).expect("copy");
    (draft, copy)
}

/// `'S3'!LocalS3` in the fixture has no calculated value, so its result is invalidated.
fn invalidating() -> RecalculationWriteOptions {
    RecalculationWriteOptions::default()
        .with_policy(RecalculationWritePolicy::InvalidateUnavailable)
}

fn results(
    calculation: &cellrune::CalculationSnapshot,
) -> Vec<(CalculationCellId, CalculationCellResult)> {
    calculation
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect()
}

#[test]
fn a_copied_sheet_duplicates_its_parts_and_names_itself_in_them() {
    let (draft, copy) = copied(&fixture(), "S2", 2);
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, invalidating()).expect("write");
    let parts = support::archive_parts(output.bytes());
    let text = |part: &str| String::from_utf8(parts[part].clone()).expect("UTF-8");
    let workbook = text("xl/workbook.xml");
    let copy_entry = workbook.find(r#"name="S2 (2)""#).expect("copy entry");
    assert!(
        workbook.find(r#"name="S2""#).expect("S2") < copy_entry
            && copy_entry < workbook.find(r#"name="S3""#).expect("S3"),
        "{workbook}"
    );
    let sheet = format!("xl/worksheets/cellrune-sheet-{}.xml", copy.get());
    assert!(
        text(&sheet).contains("<f>'S2 (2)'!A1*2</f>"),
        "{}",
        text(&sheet)
    );
    // Two tables before the copy: `Things` becomes `Things4`, with the next table ID.
    let table = text("xl/tables/cellrune-table-3.xml");
    assert!(table.contains(r#"displayName="Things4""#), "{table}");
    assert!(table.contains(r#"id="3""#), "{table}");
    let chart = text(&format!("xl/charts/cellrune-{}-chart2.xml", copy.get()));
    assert!(chart.contains("<c:f>'S2 (2)'!$A$1:$A$3</c:f>"), "{chart}");
    for duplicated in [
        format!("xl/drawings/cellrune-{}-drawing2.xml", copy.get()),
        format!("xl/media/cellrune-{}-image1.png", copy.get()),
        format!("xl/cellrune-{}-comments1.xml", copy.get()),
        format!("xl/drawings/cellrune-{}-vmlDrawing1.vml", copy.get()),
    ] {
        assert!(parts.contains_key(&duplicated), "{duplicated}");
    }
    let content_types = text("[Content_Types].xml");
    assert!(content_types.contains(&format!(
        r#"PartName="/xl/charts/cellrune-{}-chart2.xml""#,
        copy.get()
    )));
    // The source sheet's parts keep their bytes.
    let source_parts = support::archive_parts(&fixture());
    for part in [
        "xl/charts/chart2.xml",
        "xl/tables/table1.xml",
        "xl/comments1.xml",
    ] {
        assert_eq!(source_parts[part], parts[part], "{part}");
    }

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    let copy_sheet = reopened.workbook().sheet_by_id(copy).expect("copy");
    assert_eq!(copy_sheet.name().as_str(), "S2 (2)");
    assert_eq!(copy_sheet.comments().len(), 1);
    assert_eq!(
        reopened
            .workbook()
            .table("Things4")
            .map(cellrune::Table::range),
        draft
            .workbook()
            .table("Things4")
            .map(cellrune::Table::range)
    );
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    assert_eq!(results(&fresh), results(&calculation));
}

#[test]
fn copying_a_sheet_with_a_pivot_table_is_refused_with_the_part() {
    let (draft, _) = copied(&generated_sheet_deletion_fixture(), "S1", 1);
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let Err(error) = write_xlsx_draft_bytes(&draft, &calculation, invalidating()) else {
        panic!("a pivot table cannot be duplicated");
    };
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    assert_eq!(
        error.detail(),
        Some(
            "a sheet copy cannot duplicate a part its source sheet reaches: xl/pivotTables/pivotTable2.xml"
        )
    );
}

#[test]
fn a_canonical_draft_and_an_added_sheet_save_their_copies() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();
    draft
        .set_cell_formula(
            first,
            CellAddress::from_a1("A1").expect("A1"),
            FormulaText::from_xlsx("Sheet1!B1+1").expect("formula"),
        )
        .expect("formula");
    let copy = draft.copy_sheet(first, 0).expect("copy");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("canonical write");
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open");
    let names = reopened
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| (sheet.id(), sheet.name().as_str().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            (copy, "Sheet1 (2)".to_owned()),
            (first, "Sheet1".to_owned())
        ]
    );
    assert_eq!(
        support::formula_at(&reopened, "Sheet1 (2)", "A1")
            .text()
            .expect("text")
            .as_str(),
        "'Sheet1 (2)'!B1+1"
    );

    let document = open_xlsx_document_bytes(&fixture(), OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let added = draft
        .add_sheet(cellrune::SheetName::new("Added").expect("name"))
        .expect("added");
    let added_copy = draft.copy_sheet(added, 1).expect("copy the added sheet");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, invalidating()).expect("write");
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open");
    assert_eq!(
        reopened.workbook().sheets()[1].id(),
        added_copy,
        "the copy takes position 1"
    );
}
