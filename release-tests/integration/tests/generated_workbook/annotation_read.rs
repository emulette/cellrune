use cellrune::{
    CellRange, Comment, EditBatch, Hyperlink, OpenOptions, ReadOptions, SheetName, WorkbookChange,
    WorkbookDraft, WriteOptions, open_xlsx_document_bytes, read_xlsx_bytes,
    write_xlsx_draft_edits_bytes,
};

use crate::support::generated_xlsx::{
    generated_annotation_workbook, generated_sheet_reference_fixture,
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
