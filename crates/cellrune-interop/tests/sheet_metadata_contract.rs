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
