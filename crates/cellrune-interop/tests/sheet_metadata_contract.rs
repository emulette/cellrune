use cellrune_interop::{SheetPageRequestDto, WorkbookSession};
use serde_json::json;

fn fixture() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../binding-contract/sheet-metadata.xlsx"),
    )
    .expect("shared sheet metadata fixture")
}

fn request(sheet: &str, offset: u64, limit: u32) -> SheetPageRequestDto {
    SheetPageRequestDto {
        sheet: sheet.to_owned(),
        offset,
        limit,
    }
}

#[test]
fn data_validation_pages_report_every_rule_attribute() {
    let session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let page = session
        .data_validations(&request("data", 0, 0))
        .expect("data validations");
    assert_eq!(
        serde_json::to_value(&page).expect("serialize"),
        json!({
            "schema_version": 1,
            "sheet": "Data",
            "total": 3,
            "offset": 0,
            "next_offset": null,
            "entries": [
                {
                    "ranges": ["A1:A5", "C1"],
                    "validation_type": "list",
                    "operator": "between",
                    "formula1": "\"Yes,No\"",
                    "formula2": null,
                    "allow_blank": true,
                    "in_cell_dropdown": true,
                    "show_input_message": false,
                    "show_error_message": true,
                    "error_style": "warning",
                    "error_title": null,
                    "error_message": "Pick one",
                    "prompt_title": null,
                    "prompt_message": null
                },
                {
                    "ranges": ["B2"],
                    "validation_type": "whole",
                    "operator": "notBetween",
                    "formula1": "1",
                    "formula2": "9",
                    "allow_blank": false,
                    "in_cell_dropdown": false,
                    "show_input_message": false,
                    "show_error_message": false,
                    "error_style": "stop",
                    "error_title": null,
                    "error_message": null,
                    "prompt_title": "Range",
                    "prompt_message": "Outside 1-9"
                },
                {
                    "ranges": ["D1"],
                    "validation_type": "list",
                    "operator": "between",
                    "formula1": "Lists!$A$1:$A$2",
                    "formula2": null,
                    "allow_blank": true,
                    "in_cell_dropdown": true,
                    "show_input_message": false,
                    "show_error_message": false,
                    "error_style": "stop",
                    "error_title": null,
                    "error_message": null,
                    "prompt_title": null,
                    "prompt_message": null
                }
            ]
        })
    );
}

#[test]
fn data_validation_pages_are_bounded_and_reject_unknown_sheets_and_offsets() {
    let session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let first = session
        .data_validations(&request("Data", 0, 1))
        .expect("first page");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.next_offset, Some(1));
    let second = session
        .data_validations(&request("Data", 1, 1))
        .expect("second page");
    assert_eq!(second.entries[0].ranges, ["B2"]);
    assert_eq!(second.next_offset, Some(2));
    let end = session
        .data_validations(&request("Data", 3, 1))
        .expect("an offset at the end is an empty page");
    assert!(end.entries.is_empty());

    for (request, code) in [
        (request("Missing", 0, 0), "interop.sheet.not_found"),
        (request("Data", 4, 0), "interop.page.offset_out_of_range"),
        (request("Data", 0, 10_001), "interop.page.limit_exceeded"),
    ] {
        let error = session
            .data_validations(&request)
            .expect_err("invalid request");
        assert_eq!(error.code(), code);
    }

    let created = WorkbookSession::create();
    let empty = created
        .data_validations(&request("Sheet1", 0, 0))
        .expect("a created workbook has no rules");
    assert_eq!((empty.total, empty.entries.len()), (0, 0));
}

#[test]
fn hyperlink_and_comment_pages_report_every_attribute() {
    let session = WorkbookSession::open_bytes(&fixture()).expect("open");
    assert_eq!(
        serde_json::to_value(
            session
                .hyperlinks(&request("Data", 0, 0))
                .expect("hyperlinks")
        )
        .expect("serialize"),
        json!({
            "schema_version": 1, "sheet": "Data", "total": 2, "offset": 0, "next_offset": null,
            "entries": [
                {
                    "range": "B4", "target": "https://example.com/docs?a=1&b=2", "location": null,
                    "display": "Docs", "tooltip": "Open the docs"
                },
                {
                    "range": "B5:C5", "target": null, "location": "Lists!A1",
                    "display": "Lists", "tooltip": null
                }
            ]
        })
    );
    let comments = session.comments(&request("Data", 1, 1)).expect("comments");
    assert_eq!(
        serde_json::to_value(&comments).expect("serialize"),
        json!({
            "schema_version": 1, "sheet": "Data", "total": 2, "offset": 1, "next_offset": null,
            "entries": [{"address": "C3", "author": "Bo", "text": "Plain note"}]
        })
    );
    let first = session
        .comments(&request("Data", 0, 1))
        .expect("first comment");
    assert_eq!(first.entries[0].text, "Ana: check & confirm");
    assert_eq!(first.next_offset, Some(1));
    for error in [
        session.hyperlinks(&request("Missing", 0, 0)).err(),
        session.comments(&request("Data", 3, 0)).err(),
    ] {
        assert!(error.is_some());
    }
}

/// A one-sheet package whose A1 holds a comment thread with one reply, and whose B2 holds a note.
fn threaded_comment_package() -> Vec<u8> {
    use std::io::Write as _;
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/xl/threadedComments/threadedComment1.xml" ContentType="application/vnd.ms-excel.threadedcomments+xml"/><Override PartName="/xl/persons/person.xml" ContentType="application/vnd.ms-excel.person+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Thread" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2017/10/relationships/person" Target="persons/person.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2017/10/relationships/threadedComment" Target="../threadedComments/threadedComment1.xml"/></Relationships>"#,
        ),
        (
            "xl/comments1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>tc={1}</author><author>Bo Lee</author></authors><commentList><comment ref="A1" authorId="0"><text><t>[Threaded comment] placeholder</t></text></comment><comment ref="B2" authorId="1"><text><t>Note</t></text></comment></commentList></comments>"#,
        ),
        (
            "xl/threadedComments/threadedComment1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><threadedComment ref="A1" personId="{P1}" id="{1}"><text>Check</text></threadedComment><threadedComment ref="A1" personId="{P2}" id="{2}" parentId="{1}"><text>Done</text></threadedComment></ThreadedComments>"#,
        ),
        (
            "xl/persons/person.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><personList xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><person displayName="Ana Kim" id="{P1}"/><person displayName="Bo Lee" id="{P2}"/></personList>"#,
        ),
    ];
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(content.as_bytes()).expect("content");
    }
    writer.finish().expect("archive").into_inner()
}

#[test]
fn comment_pages_carry_the_replies_of_a_comment_thread() {
    let session = WorkbookSession::open_bytes(&threaded_comment_package()).expect("open");
    let page = session
        .comments(&request("Thread", 0, 0))
        .expect("comments");
    assert_eq!(
        serde_json::to_value(&page).expect("serialize")["entries"],
        json!([
            {
                "address": "A1",
                "author": "Ana Kim",
                "text": "Check",
                "replies": [{"author": "Bo Lee", "text": "Done"}],
            },
            {"address": "B2", "author": "Bo Lee", "text": "Note"},
        ])
    );
}
