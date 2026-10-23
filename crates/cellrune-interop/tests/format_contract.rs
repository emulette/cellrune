use cellrune_interop::{
    CalculationOptionsDto, EditBatchV2Dto, RangeRequestDto, WorkbookSession, WriteOptionsDto,
};
use serde_json::{Value, json};

fn fixture() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../binding-contract/sheet-metadata.xlsx"),
    )
    .expect("shared sheet metadata fixture")
}

fn range(sheet: &str, start: &str, end: &str) -> RangeRequestDto {
    RangeRequestDto {
        sheet: sheet.to_owned(),
        start: start.to_owned(),
        end: end.to_owned(),
        offset: 0,
        limit: 0,
    }
}

fn formats(session: &WorkbookSession, start: &str, end: &str) -> Value {
    serde_json::to_value(
        session
            .read_formats(&range("Data", start, end))
            .expect("formats"),
    )
    .expect("serialize")
}

#[test]
fn format_pages_report_source_formats_widths_and_heights() {
    let session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let page = formats(&session, "A1", "C1");
    assert_eq!(page["total_cells"], 3);
    assert_eq!(
        page["cells"][0],
        json!({
            "address": "A1",
            "format": {
                "font": {
                    "name": "Arial", "size": 12.0, "bold": true, "italic": false,
                    "underline": "none", "strike": false,
                    "color": {"kind": "argb", "argb": "FFFF0000", "tint": 0.0}
                },
                "fill": {
                    "kind": "pattern", "pattern": "solid",
                    "foreground": {"kind": "argb", "argb": "FFFFFF00", "tint": 0.0},
                    "background": null
                },
                "border": {
                    "left": {"style": "none", "color": null},
                    "right": {"style": "none", "color": null},
                    "top": {"style": "none", "color": null},
                    "bottom": {"style": "thin", "color": {"kind": "indexed", "index": 64, "tint": 0.0}},
                    "diagonal": {"style": "none", "color": null},
                    "diagonal_up": false, "diagonal_down": false
                },
                "alignment": {
                    "horizontal": "center", "vertical": "bottom", "wrap_text": true,
                    "shrink_to_fit": false, "indent": 0, "text_rotation": 0
                }
            }
        })
    );
    assert_eq!(page["cells"][1]["format"]["font"]["name"], "Calibri");
    assert_eq!(
        page["cells"][1]["format"]["font"]["color"],
        json!({"kind": "theme", "index": 1, "tint": 0.0})
    );
    assert_eq!(
        page["column_widths"],
        json!([{"first": 1, "last": 2, "width": 18.5}])
    );
    assert_eq!(page["row_heights"], json!([{"row": 1, "height": 24.0}]));
    let below = formats(&session, "B2", "B3");
    assert_eq!(
        below["column_widths"],
        json!([{"first": 2, "last": 2, "width": 18.5}])
    );
    assert_eq!(below["row_heights"], json!([]));
}

#[test]
fn v2_format_changes_keep_the_semantic_revision_and_survive_a_save() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let revision = session.summary().semantic_revision;
    let batch: EditBatchV2Dto = serde_json::from_value(json!({
        "changes": [
            {"kind": "add_sheet", "name": "Report"},
            {
                "kind": "set_cell_format", "sheet": "report", "address": "B2",
                "font": {"name": "Georgia", "size": 13.5, "italic": true, "underline": "single"},
                "fill": {"pattern": "darkGrid", "foreground": {"kind": "theme", "index": 5, "tint": -0.5}},
                "border": {"top": {"style": "dashed"}, "diagonal_up": true},
                "alignment": {"horizontal": "right", "vertical": "center", "indent": 3}
            },
            {"kind": "set_cell_format", "sheet": "Data", "address": "A1", "alignment": {}},
            {"kind": "set_column_width", "sheet": "Data", "first": 2, "last": 3, "width": 30.0},
            {"kind": "set_row_height", "sheet": "Data", "row": 1, "height": null},
            {"kind": "set_row_height", "sheet": "Report", "row": 2, "height": 33.0}
        ]
    }))
    .expect("v2 batch");
    session
        .apply_changes_v2(revision, batch)
        .expect("format changes");
    assert_ne!(
        session.summary().semantic_revision,
        revision,
        "adding a sheet is a semantic change"
    );
    let edited = formats(&session, "A1", "C1");
    assert_eq!(
        edited["cells"][0]["format"]["alignment"]["horizontal"],
        "general"
    );
    assert_eq!(
        edited["cells"][0]["format"]["font"]["name"], "Arial",
        "an edit keeps the components it does not set"
    );
    assert_eq!(
        edited["column_widths"],
        json!([
            {"first": 1, "last": 1, "width": 18.5},
            {"first": 2, "last": 3, "width": 30.0}
        ])
    );
    assert_eq!(edited["row_heights"], json!([]));
    let report = serde_json::to_value(
        session
            .read_formats(&range("Report", "B2", "B2"))
            .expect("report formats"),
    )
    .expect("serialize");

    let revision = session.summary().semantic_revision;
    let format_only: EditBatchV2Dto = serde_json::from_value(json!({
        "changes": [{"kind": "set_row_height", "sheet": "Data", "row": 4, "height": 12.0}]
    }))
    .expect("format-only batch");
    session
        .apply_changes_v2(revision, format_only)
        .expect("row height");
    assert_eq!(
        session.summary().semantic_revision,
        revision,
        "a format-only batch keeps the semantic revision"
    );

    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    let (bytes, _) = session
        .save_bytes(WriteOptionsDto::default())
        .expect("save");
    let reopened = WorkbookSession::open_bytes(&bytes).expect("reopen");
    assert_eq!(formats(&reopened, "A1", "C1"), edited);
    let reopened_report = serde_json::to_value(
        reopened
            .read_formats(&range("Report", "B2", "B2"))
            .expect("report formats"),
    )
    .expect("serialize");
    assert_eq!(reopened_report, report);
    assert_eq!(
        reopened_report["cells"][0]["format"]["fill"],
        json!({
            "kind": "pattern", "pattern": "darkGrid",
            "foreground": {"kind": "theme", "index": 5, "tint": -0.5}, "background": null
        })
    );
    assert_eq!(
        formats(&reopened, "A4", "A4")["row_heights"],
        json!([{"row": 4, "height": 12.0}])
    );
}

#[test]
fn invalid_format_changes_fail_with_stable_codes() {
    let mut session = WorkbookSession::create();
    let revision = session.summary().semantic_revision;
    for (change, code) in [
        (
            json!({"kind": "set_cell_format", "sheet": "Sheet1", "address": "A1", "fill": {"pattern": "plaid"}}),
            "interop.change.invalid",
        ),
        (
            json!({"kind": "set_cell_format", "sheet": "Sheet1", "address": "A1", "font": {"size": 0.5}}),
            "validation.format_value_out_of_range",
        ),
        (
            json!({"kind": "set_cell_format", "sheet": "Sheet1", "address": "A1", "font": {"color": {"kind": "argb", "argb": "red"}}}),
            "interop.change.invalid",
        ),
        (
            json!({"kind": "set_column_width", "sheet": "Sheet1", "first": 1, "last": 1, "width": 300.0}),
            "validation.format_value_out_of_range",
        ),
        (
            json!({"kind": "set_row_height", "sheet": "Missing", "row": 1, "height": 10.0}),
            "interop.sheet.not_found",
        ),
    ] {
        let batch: EditBatchV2Dto =
            serde_json::from_value(json!({"changes": [change]})).expect("batch shape");
        let error = session
            .apply_changes_v2(revision, batch)
            .expect_err("invalid change");
        assert_eq!(error.code(), code);
    }
}
