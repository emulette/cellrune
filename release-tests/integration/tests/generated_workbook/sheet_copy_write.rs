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

fn range(value: &str) -> cellrune::CellRange {
    let (start, end) = value.split_once(':').expect("range");
    cellrune::CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

/// The table of the sheet named `sheet`.
fn table_of(draft: &WorkbookDraft, sheet: &str) -> cellrune::TableId {
    draft
        .workbook()
        .sheet_by_name(sheet)
        .expect("sheet")
        .tables()[0]
        .id()
}

#[test]
fn a_copied_table_saves_a_later_range_change_and_a_source_table_its_own() {
    let (mut draft, _) = copied(&fixture(), "S2", 2);
    let copied_table = table_of(&draft, "S2 (2)");
    let source_table = table_of(&draft, "S2");
    draft
        .apply_changes(cellrune::EditBatch::new([
            cellrune::WorkbookChange::set_table_range(copied_table, range("C1:C3")),
            cellrune::WorkbookChange::set_table_range(source_table, range("C1:D4")),
        ]))
        .expect("resize both tables");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, invalidating()).expect("write");
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open");
    let saved = |sheet: &str| {
        let table = &reopened
            .workbook()
            .sheet_by_name(sheet)
            .expect("sheet")
            .tables()[0];
        (table.range(), table.columns().len())
    };
    assert_eq!(saved("S2 (2)"), (range("C1:C3"), 1));
    assert_eq!(saved("S2"), (range("C1:D4"), 2));
}

#[test]
fn a_table_created_or_converted_on_a_copy_is_refused_with_the_copy() {
    let (mut created, _) = copied(&fixture(), "S2", 2);
    let copy = created
        .workbook()
        .sheet_by_name("S2 (2)")
        .expect("copy")
        .id();
    created
        .apply_changes(cellrune::EditBatch::new([
            cellrune::WorkbookChange::create_table(copy, range("J1:K3"), None),
        ]))
        .expect("create a table on the copy");
    let (mut converted, _) = copied(&fixture(), "S2", 2);
    let copied_table = table_of(&converted, "S2 (2)");
    converted
        .apply_changes(cellrune::EditBatch::new([
            cellrune::WorkbookChange::convert_table_to_range(copied_table),
        ]))
        .expect("convert the copied table");
    for draft in [created, converted] {
        let copy = draft.workbook().sheet_by_name("S2 (2)").expect("copy").id();
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let Err(error) = write_xlsx_draft_bytes(&draft, &calculation, invalidating()) else {
            panic!("the copy's tables changed");
        };
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
        assert_eq!(
            error.detail(),
            Some(
                format!(
                    "a sheet copy cannot save a table created or converted to a range on the copy or on its source before the copy: xl/worksheets/cellrune-sheet-{}.xml",
                    copy.get()
                )
                .as_str()
            )
        );
    }
}

#[test]
fn row_edits_of_a_copy_or_its_source_replay_on_the_parts_each_one_holds() {
    use cellrune::{EditBatch, Row, WorkbookChange};

    let insert =
        |sheet| WorkbookChange::insert_rows(sheet, Row::new(1).expect("row"), 2).expect("insert");
    // Rows inserted in S2 before the copy reach both; after it, only the sheet they were
    // inserted in. The chart reads A1:A3 of S2.
    for (before_copy, after_copy_in_copy, after_copy_in_source, source_chart, copy_chart) in [
        (false, true, false, "$A$1:$A$3", "$A$3:$A$5"),
        (true, false, false, "$A$3:$A$5", "$A$3:$A$5"),
        (false, false, true, "$A$3:$A$5", "$A$1:$A$3"),
    ] {
        // The fixture's first pivot cache reads S2, whose rows the edits move; it reads S3 here.
        let source = support::replace_part_text(
            &fixture(),
            "xl/pivotCache/pivotCacheDefinition1.xml",
            r#"<worksheetSource ref="F1:G3" sheet="S2"/>"#,
            r#"<worksheetSource ref="D1:D2" sheet="S3"/>"#,
        );
        let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
        let mut draft = WorkbookDraft::from_document(&document);
        let s2 = draft.workbook().sheet_by_name("S2").expect("S2").id();
        if before_copy {
            draft
                .apply_changes(EditBatch::new([insert(s2)]))
                .expect("insert");
        }
        let copy = draft.copy_sheet(s2, 2).expect("copy");
        if after_copy_in_copy {
            draft
                .apply_changes(EditBatch::new([insert(copy)]))
                .expect("insert");
        }
        if after_copy_in_source {
            draft
                .apply_changes(EditBatch::new([insert(s2)]))
                .expect("insert");
        }
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, invalidating()).unwrap_or_else(|error| {
                panic!("{before_copy} {after_copy_in_copy} {after_copy_in_source}: {error:?}")
            });
        let parts = support::archive_parts(output.bytes());
        let text = |part: &str| String::from_utf8(parts[part].clone()).expect("UTF-8");
        let source = text("xl/charts/chart2.xml");
        assert!(
            source.contains(&format!("<c:f>'S2'!{source_chart}</c:f>"))
                || source.contains(&format!("<c:f>S2!{source_chart}</c:f>")),
            "{source}"
        );
        let copied = text(&format!("xl/charts/cellrune-{}-chart2.xml", copy.get()));
        assert!(
            copied.contains(&format!("<c:f>'S2 (2)'!{copy_chart}</c:f>")),
            "{copied}"
        );
        let reopened =
            open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
        for sheet in [s2, copy] {
            let written = reopened.workbook().sheet_by_id(sheet).expect("sheet");
            let staged = draft.workbook().sheet_by_id(sheet).expect("sheet");
            assert_eq!(
                written
                    .cells()
                    .map(|cell| (cell.address(), format!("{:?}", cell.content())))
                    .collect::<Vec<_>>(),
                staged
                    .cells()
                    .map(|cell| (cell.address(), format!("{:?}", cell.content())))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                written
                    .comments()
                    .iter()
                    .map(cellrune::Comment::address)
                    .collect::<Vec<_>>(),
                staged
                    .comments()
                    .iter()
                    .map(cellrune::Comment::address)
                    .collect::<Vec<_>>()
            );
        }
        let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
        assert_eq!(results(&fresh), results(&calculation));
    }
}
