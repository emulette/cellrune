use cellrune_interop::{
    CalculationLimitOptionsDto, CalculationOptionsDto, CalculationResultDto, CalculationTargetDto,
    EditBatchV2Dto, RangeRequestDto, ReadLimitOptionsDto, SheetPageRequestDto,
    TargetCalculationLimitsDto, TargetCalculationRequestDto, WorkbookSession,
};
use serde_json::{Value, json};

fn fixture() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../binding-contract/surface-authoring.xlsx"),
    )
    .expect("shared surface authoring fixture")
}

fn page(sheet: &str) -> SheetPageRequestDto {
    SheetPageRequestDto {
        sheet: sheet.to_owned(),
        offset: 0,
        limit: 0,
    }
}

fn batch(changes: Value) -> EditBatchV2Dto {
    serde_json::from_value(json!({ "changes": changes })).expect("v2 batch")
}

fn summary_sheet(session: &WorkbookSession, index: usize) -> Value {
    serde_json::to_value(session.summary()).expect("summary")["sheets"][index].clone()
}

#[test]
fn frozen_panes_and_phonetics_are_read_from_the_package() {
    let session = WorkbookSession::open_bytes(&fixture()).expect("open");
    assert_eq!(
        summary_sheet(&session, 0)["frozen_pane"],
        json!({"rows": 1, "columns": 0})
    );
    // A sheet without a frozen pane keeps the summary shape it had before the field existed.
    assert!(
        summary_sheet(&session, 1)
            .as_object()
            .is_some_and(|sheet| !sheet.contains_key("frozen_pane"))
    );

    let phonetics = session.phonetics(&page("data")).expect("phonetics");
    assert_eq!(
        serde_json::to_value(&phonetics).expect("serialize"),
        json!({
            "schema_version": 1,
            "sheet": "Data",
            "total": 1,
            "offset": 0,
            "next_offset": null,
            "entries": [{
                "address": "D2",
                "runs": [
                    {"start_utf16": 0, "end_utf16": 2, "text": "トウキョウ"},
                    {"start_utf16": 2, "end_utf16": 3, "text": "ト"}
                ],
                "properties": {"font_id": 0, "phonetic_type": "Hiragana", "alignment": null},
                "visible": true
            }]
        })
    );
    assert_eq!(
        session.phonetics(&page("Filter")).expect("empty").entries,
        []
    );
    let error = session
        .phonetics(&SheetPageRequestDto {
            offset: 2,
            ..page("Data")
        })
        .expect_err("offset past the end");
    assert_eq!(error.code(), "interop.page.offset_out_of_range");
    let error = session.phonetics(&page("Missing")).expect_err("sheet");
    assert_eq!(error.code(), "interop.sheet.not_found");
}

#[test]
fn widening_edits_reach_the_table_the_filter_and_the_saved_package() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes_v2(
            revision,
            batch(json!([
                {"kind": "widen_table_columns", "table_id": 1, "range": "A1:C3"},
                {"kind": "widen_sheet_auto_filter", "sheet": "filter", "range": "A1:C3"}
            ])),
        )
        .expect("widen");
    assert_eq!(receipt.changed_table_ids, [1]);
    assert_eq!(receipt.receipt.result_revision, revision + 1);

    let check = |session: &WorkbookSession| {
        let table = &summary_sheet(session, 0)["tables"][0];
        assert_eq!(table["range"], "A1:C3");
        assert_eq!(table["columns"][2]["name"], "Region");
        assert_eq!(summary_sheet(session, 1)["auto_filter"], "A1:C3");
    };
    check(&session);
    let (bytes, _) = session.save_edits_bytes().expect("save edits");
    check(&WorkbookSession::open_bytes(&bytes).expect("reopen"));

    let error = session
        .apply_changes_v2(
            receipt.receipt.result_revision,
            batch(
                json!([{"kind": "widen_sheet_auto_filter", "sheet": "Filter", "range": "A1:B3"}]),
            ),
        )
        .expect_err("narrowing is not widening");
    assert_eq!(error.kind(), cellrune_interop::InteropErrorKind::Validation);
}

#[test]
fn presentation_edits_keep_the_semantic_revision_and_save() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes_v2(
            revision,
            batch(json!([
                {"kind": "set_frozen_pane", "sheet": "Filter", "rows": 2, "columns": 1},
                {"kind": "set_frozen_pane", "sheet": "Data", "rows": 0, "columns": 0},
                {"kind": "set_phonetics", "sheet": "Data", "address": "A2",
                 "runs": [{"start_utf16": 0, "end_utf16": 5, "text": "ノース"}],
                 "visible": false, "properties": {"alignment": "center"}},
                {"kind": "clear_phonetics", "sheet": "Data", "address": "D2"}
            ])),
        )
        .expect("presentation batch");
    assert_eq!(receipt.receipt.result_revision, revision);

    let check = |session: &WorkbookSession| {
        assert!(
            summary_sheet(session, 0)
                .as_object()
                .is_some_and(|sheet| !sheet.contains_key("frozen_pane"))
        );
        assert_eq!(
            summary_sheet(session, 1)["frozen_pane"],
            json!({"rows": 2, "columns": 1})
        );
        let phonetics = serde_json::to_value(session.phonetics(&page("Data")).expect("page"))
            .expect("serialize");
        assert_eq!(
            phonetics["entries"],
            json!([{
                "address": "A2",
                "runs": [{"start_utf16": 0, "end_utf16": 5, "text": "ノース"}],
                "properties": {"font_id": 0, "phonetic_type": null, "alignment": "center"},
                "visible": false
            }])
        );
    };
    check(&session);
    let (bytes, _) = session.save_edits_bytes().expect("save edits");
    check(&WorkbookSession::open_bytes(&bytes).expect("reopen"));

    for (change, code) in [
        (
            json!({"kind": "set_phonetics", "sheet": "Data", "address": "B2",
                   "runs": [{"start_utf16": 0, "end_utf16": 1, "text": "x"}]}),
            "validation.phonetics_require_text_cell",
        ),
        (
            json!({"kind": "set_phonetics", "sheet": "Data", "address": "A3",
                   "runs": [{"start_utf16": 0, "end_utf16": 9, "text": "x"}]}),
            "validation.phonetic_range_out_of_bounds",
        ),
        (
            json!({"kind": "set_phonetics", "sheet": "Data", "address": "A3",
                   "runs": [{"start_utf16": 0, "end_utf16": 1, "text": "x"}],
                   "properties": {"phonetic_type": "katakana"}}),
            "interop.change.invalid",
        ),
        (
            json!({"kind": "set_frozen_pane", "sheet": "Data", "rows": 1_048_576, "columns": 0}),
            "validation.frozen_rows_out_of_range",
        ),
    ] {
        let error = session
            .apply_changes_v2(revision, batch(json!([change])))
            .expect_err("rejected presentation change");
        assert_eq!(error.code(), code);
    }
    let error = session
        .apply_changes_v2(
            revision,
            batch(
                json!([{"kind": "set_value", "sheet": "Data", "address": "A2",
                          "value": {"kind": "text", "value": "West"}}]),
            ),
        )
        .expect_err("annotated text needs its phonetics cleared first");
    assert_eq!(
        error.code(),
        "validation.annotated_text_replacement_required"
    );
}

#[test]
fn read_limit_failures_report_their_figures() {
    let error = WorkbookSession::open_bytes_with_limits(
        &fixture(),
        ReadLimitOptionsDto {
            max_phonetic_runs_per_item: Some(1),
            ..ReadLimitOptionsDto::default()
        },
    )
    .expect_err("two runs exceed a per-item limit of one");
    assert_eq!(error.code(), "xlsx.too_many_phonetic_runs");
    let serialized = serde_json::to_value(&error).expect("serialize");
    assert_eq!(
        serialized["details"]["limit"],
        json!({"name": "max_phonetic_runs_per_item", "observed": 2})
    );

    let error = WorkbookSession::open_bytes_with_limits(
        &fixture(),
        ReadLimitOptionsDto {
            max_entry_uncompressed_bytes: Some(64),
            ..ReadLimitOptionsDto::default()
        },
    )
    .expect_err("every part is larger than 64 bytes");
    let limit = error.details().limit.as_ref().expect("limit figures");
    assert!(limit.name.is_none());
    assert!(limit.observed.is_some_and(|observed| observed > 64));

    // Failures without figures keep the error shape they had before the field existed.
    let error = WorkbookSession::open_bytes(b"not a package").expect_err("not a ZIP");
    let serialized = serde_json::to_value(&error).expect("serialize");
    assert!(serialized["details"].get("limit").is_none());
}

#[test]
fn calculation_limits_name_the_limit_an_issue_exceeded() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=1+2+3+4+5", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "A2", "=1", None)
        .expect("formula");
    session
        .calculate(CalculationOptionsDto {
            calculation_limits: CalculationLimitOptionsDto {
                max_formula_tokens: Some(3),
                ..CalculationLimitOptionsDto::default()
            },
            ..CalculationOptionsDto::default()
        })
        .expect("calculate");
    let range = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "A2".to_owned(),
            offset: 0,
            limit: 0,
        })
        .expect("range");
    let Some(CalculationResultDto::Unavailable { code, limit, .. }) = &range.cells[0].calculated
    else {
        panic!("the long formula must exceed the token limit");
    };
    assert_eq!(code, "calculation.resource_limit_exceeded");
    assert_eq!(limit.as_deref(), Some("max_formula_tokens"));
    let calculated = serde_json::to_value(&range.cells[1].calculated).expect("serialize");
    assert_eq!(
        calculated,
        json!({"kind": "value", "value": {"kind": "number", "value": 1.0}})
    );
}

#[test]
fn targeted_limit_failures_report_counted_cells_and_work() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=1", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "A2", "=A1+1", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "A3", "=A2+1", None)
        .expect("formula");
    let request = |end: &str, limits: TargetCalculationLimitsDto| TargetCalculationRequestDto {
        targets: vec![CalculationTargetDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: Some(end.to_owned()),
        }],
        options: CalculationOptionsDto::default(),
        limits,
    };
    let error = session
        .calculate_targets(&request(
            "A3",
            TargetCalculationLimitsDto {
                max_result_cells: 2,
                ..TargetCalculationLimitsDto::default()
            },
        ))
        .expect_err("three cells exceed two results");
    assert_eq!(error.code(), "calculation.target.target_limit_exceeded");
    let limit = error.details().limit.as_ref().expect("limit figures");
    assert_eq!(limit.requested_cell_count, Some(3));
    assert_eq!(limit.evaluated_count, None);

    let error = session
        .calculate_targets(&request(
            "A3",
            TargetCalculationLimitsDto {
                max_evaluated_cells: 1,
                ..TargetCalculationLimitsDto::default()
            },
        ))
        .expect_err("the chain needs three evaluations");
    assert_eq!(error.code(), "calculation.target.evaluation_limit_exceeded");
    let limit = error.details().limit.as_ref().expect("limit figures");
    assert!(limit.evaluated_count.is_some());
    assert_eq!(limit.requested_cell_count, None);
}
