use cellrune::{
    CellRange, Comment, EditBatch, Hyperlink, OpenOptions, ReadOptions, SheetName, WorkbookChange,
    WorkbookDraft, WriteOptions, open_xlsx_document_bytes, read_xlsx_bytes,
    write_xlsx_draft_edits_bytes,
};

use crate::support::generated_xlsx::{
    generated_annotation_workbook, generated_sheet_reference_fixture,
    generated_threaded_comment_workbook,
};

fn reference(range: CellRange) -> String {
    if range.start() == range.end() {
        range.start().to_string()
    } else {
        format!("{}:{}", range.start(), range.end())
    }
}

type HyperlinkSummary = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn hyperlinks(hyperlinks: &[Hyperlink]) -> Vec<HyperlinkSummary> {
    hyperlinks
        .iter()
        .map(|hyperlink| {
            (
                reference(hyperlink.range()),
                hyperlink.target().map(str::to_owned),
                hyperlink.location().map(str::to_owned),
                hyperlink.display().map(str::to_owned),
                hyperlink.tooltip().map(str::to_owned),
            )
        })
        .collect()
}

fn comments(comments: &[Comment]) -> Vec<(String, Option<String>, String)> {
    comments
        .iter()
        .map(|comment| {
            (
                comment.address().to_string(),
                comment.author().map(str::to_owned),
                comment.text().to_owned(),
            )
        })
        .collect()
}

fn some(value: &str) -> Option<String> {
    Some(value.to_owned())
}

#[test]
fn reads_hyperlinks_and_comments() {
    let source = generated_annotation_workbook(false);
    let snapshot = read_xlsx_bytes(&source, ReadOptions::default()).expect("read");
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    for workbook in [&snapshot, document.workbook()] {
        let notes = workbook.sheet_by_name("Notes").expect("Notes");
        assert_eq!(
            hyperlinks(notes.hyperlinks()),
            [
                (
                    "A1".to_owned(),
                    some("https://example.com/a?b=1&c=2"),
                    None,
                    some("Site"),
                    some("Open the site"),
                ),
                (
                    "B2:C3".to_owned(),
                    None,
                    some("Plain!A1"),
                    some("Plain sheet"),
                    None,
                ),
                ("D1".to_owned(), None, some("Sheet1!A1"), None, None),
            ],
            "a malformed ref is left unmodeled and an unresolved relationship has no target"
        );
        assert_eq!(
            comments(notes.comments()),
            [
                (
                    "A1".to_owned(),
                    some("Ana Kim"),
                    "Ana Kim: check & confirm".to_owned()
                ),
                ("B2".to_owned(), None, "Plain note".to_owned()),
            ],
            "phonetic runs are not text and a malformed ref is skipped"
        );
        let plain = workbook.sheet_by_name("Plain").expect("Plain");
        assert_eq!(
            hyperlinks(plain.hyperlinks()),
            [("A1".to_owned(), None, None, None, None)],
            "a relationship part that does not parse leaves the target unresolved"
        );
        assert!(plain.comments().is_empty());
    }
}

#[test]
fn a_comments_part_that_does_not_parse_leaves_the_sheet_without_comments() {
    let source = generated_annotation_workbook(true);
    let document = open_xlsx_document_bytes(&source, OpenOptions::default())
        .expect("a broken comments part does not fail the read");
    let notes = document.workbook().sheet_by_name("Notes").expect("Notes");
    assert!(notes.comments().is_empty());
    assert_eq!(notes.hyperlinks().len(), 3);
}

#[test]
fn sheet_rename_rewrites_internal_hyperlink_locations_as_the_saved_part_does() {
    let source = generated_sheet_reference_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let data = draft.workbook().sheet_by_name("Data").expect("Data").id();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_sheet(
            data,
            SheetName::new("New Data").expect("sheet name"),
        )]))
        .expect("rename");
    let summary = hyperlinks(
        draft
            .workbook()
            .sheet_by_name("Summary")
            .expect("Summary")
            .hyperlinks(),
    );
    assert_eq!(
        summary,
        [
            (
                "C1".to_owned(),
                None,
                some("'New Data'!A1"),
                some("Data"),
                None,
            ),
            (
                "C2".to_owned(),
                some("https://example.com/other.xlsx"),
                some("Data!A1"),
                None,
                None,
            ),
        ],
        "an external hyperlink's location points into its target and keeps its text"
    );
    let saved = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("edits write")
        .into_bytes();
    let reopened = open_xlsx_document_bytes(&saved, OpenOptions::default()).expect("reopen");
    assert_eq!(
        hyperlinks(
            reopened
                .workbook()
                .sheet_by_name("Summary")
                .expect("Summary")
                .hyperlinks()
        ),
        summary
    );
}

/// Sheet `Data` holds a table and 300 external hyperlinks, so its relationship part is read once
/// for the table and again for the hyperlinks; sheet `After` follows it.
fn table_and_hyperlinks_workbook() -> Vec<(&'static str, String)> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="After" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let links = (1..=300)
        .map(|row| format!(r#"<hyperlink ref="C{row}" r:id="rIdL{row}"/>"#))
        .collect::<String>();
    let data = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Key</t></is></c></row><row r="2"><c r="A2"><v>1</v></c></row></sheetData><hyperlinks>{links}</hyperlinks><tableParts count="1"><tablePart r:id="rIdT"/></tableParts></worksheet>"#
    );
    let targets = (1..=300)
        .map(|row| format!(r#"<Relationship Id="rIdL{row}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/{row}" TargetMode="External"/>"#))
        .collect::<String>();
    let data_relationships = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdT" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>{targets}</Relationships>"#
    );
    let table = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Keys" displayName="Keys" ref="A1:A2" headerRowCount="1"><tableColumns count="1"><tableColumn id="1" name="Key"/></tableColumns></table>"#;
    let after = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#;
    vec![
        ("[Content_Types].xml", content_types.to_owned()),
        ("_rels/.rels", root.to_owned()),
        ("xl/workbook.xml", workbook.to_owned()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships.to_owned(),
        ),
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/_rels/sheet1.xml.rels", data_relationships),
        ("xl/tables/table1.xml", table.to_owned()),
        ("xl/worksheets/sheet2.xml", after.to_owned()),
    ]
}

#[test]
fn annotation_reads_never_spend_the_budget_a_required_part_needs() {
    let parts = table_and_hyperlinks_workbook();
    let entries = parts
        .iter()
        .map(|(name, contents)| (*name, contents.as_str()))
        .collect::<Vec<_>>();
    let bytes = crate::support::generated_xlsx::generated_package(&entries);
    // Every part fits the limit once, as 0.1.26 read it; the hyperlinks need the relationship
    // part a second time.
    let total = parts
        .iter()
        .map(|(_, contents)| contents.len() as u64)
        .sum::<u64>();
    let limits = cellrune::ReadLimits::default()
        .with_max_total_uncompressed_bytes(total)
        .expect("limit");
    let document = open_xlsx_document_bytes(&bytes, OpenOptions::new(ReadOptions::new(limits)))
        .expect("an open that read every required part within the limit succeeds");
    let workbook = document.workbook();
    assert_eq!(workbook.sheets().len(), 2);
    assert_eq!(workbook.sheets()[0].tables().len(), 1);
    let links = workbook.sheets()[0].hyperlinks();
    assert_eq!(links.len(), 300);
    assert!(links.iter().all(|link| link.target().is_none()));
    let codes = document
        .presentation()
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code().as_str())
        .collect::<Vec<_>>();
    assert_eq!(codes, ["xlsx.annotations.skipped"]);

    let roomy = open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("open");
    assert_eq!(
        roomy.workbook().sheets()[0].hyperlinks()[0].target(),
        Some("https://example.com/1")
    );
    assert!(roomy.presentation().diagnostics().is_empty());
}

#[test]
fn threaded_comments_replace_their_placeholder_notes() {
    let source = generated_threaded_comment_workbook();
    let snapshot = read_xlsx_bytes(&source, ReadOptions::default()).expect("read");
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    for workbook in [&snapshot, document.workbook()] {
        let sheet = workbook.sheet_by_name("Thread").expect("sheet");
        assert_eq!(
            comments(sheet.comments()),
            [
                (
                    "A1".to_owned(),
                    some("Ana Kim"),
                    "Check this & that".to_owned()
                ),
                ("B2".to_owned(), None, "Who wrote this?".to_owned()),
                ("C3".to_owned(), some("Bo Lee"), "Plain note".to_owned()),
            ]
        );
        let replies = sheet.comments()[0]
            .replies()
            .iter()
            .map(|reply| (reply.author().map(str::to_owned), reply.text().to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            replies,
            [
                (some("Bo Lee"), "Done".to_owned()),
                (some("Ana Kim"), "Thanks".to_owned()),
            ]
        );
        assert!(sheet.comments()[1].replies().is_empty());
        assert!(sheet.comments()[2].replies().is_empty());
    }
}
