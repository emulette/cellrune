use cellrune_interop::{EditBatchV2Dto, SheetPageRequestDto, WorkbookSession};
use serde_json::{Value, json};

fn fixture() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../binding-contract/sheet-metadata.xlsx"),
    )
    .expect("shared sheet metadata fixture")
}

fn rules(session: &WorkbookSession, sheet: &str) -> Value {
    serde_json::to_value(
        session
            .data_validations(&SheetPageRequestDto {
                sheet: sheet.to_owned(),
                offset: 0,
                limit: 0,
            })
            .expect("data validations")
            .entries,
    )
    .expect("serialize")
}

fn batch(changes: Value) -> EditBatchV2Dto {
    serde_json::from_value(json!({ "changes": changes })).expect("v2 batch")
}

fn roles_rule() -> Value {
    json!({
        "ranges": ["F2:F10"],
        "validation_type": "list",
        "operator": "between",
        "formula1": "Lists!$A$1:$A$2",
        "formula2": null,
        "allow_blank": true,
        "in_cell_dropdown": true,
        "show_input_message": true,
        "show_error_message": true,
        "error_style": "warning",
        "error_title": "Unknown role",
        "error_message": "Pick a listed role",
        "prompt_title": "Role",
        "prompt_message": "Choose a role"
    })
}

#[test]
fn rule_edits_keep_the_semantic_revision_and_survive_both_saves() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes_v2(
            revision,
            batch(json!([
                {"kind": "set_data_validation", "sheet": "data", "rule": roles_rule()},
                {"kind": "clear_data_validations", "sheet": "Data", "ranges": ["A3:A5"]},
                {"kind": "remove_data_validation", "sheet": "Data", "index": 1}
            ])),
        )
        .expect("rule edits");
    assert_eq!(session.summary().semantic_revision, revision);
    assert!(receipt.receipt.calculation_changed_cells.is_empty());
    let edited = rules(&session, "Data");
    assert_eq!(edited[0]["ranges"], json!(["A1:A2", "C1"]));
    assert_eq!(
        edited[edited.as_array().expect("rules").len() - 1],
        roles_rule()
    );

    session.calculate(Default::default()).expect("calculate");
    let (saved, _) = session.save_bytes(Default::default()).expect("save");
    let (edits, _) = session.save_edits_bytes().expect("save edits");
    for bytes in [saved, edits] {
        let reopened = WorkbookSession::open_bytes(&bytes).expect("reopen");
        let mut reread = rules(&reopened, "Data").as_array().expect("rules").clone();
        let mut expected = edited.as_array().expect("rules").clone();
        reread.sort_by_key(Value::to_string);
        expected.sort_by_key(Value::to_string);
        assert_eq!(reread, expected);
    }
}

#[test]
fn the_receipt_inverse_sets_the_rules_back() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let original = rules(&session, "Data");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes_v2(
            revision,
            batch(json!([{"kind": "set_data_validation", "sheet": "Data", "rule": roles_rule()}])),
        )
        .expect("rule edit");
    let inverse = receipt.receipt.inverse.expect("an inverse");
    assert!(inverse.unrestored.is_empty());
    assert_eq!(
        serde_json::to_value(&inverse.changes).expect("serialize")[0]["kind"],
        "set_data_validations"
    );
    session
        .apply_changes_v2(
            session.summary().semantic_revision,
            EditBatchV2Dto {
                changes: inverse.changes,
            },
        )
        .expect("undo");
    assert_eq!(rules(&session, "Data"), original);
}

#[test]
fn invalid_rule_edits_fail_with_stable_codes() {
    let mut session = WorkbookSession::open_bytes(&fixture()).expect("open");
    let mut missing_formula = roles_rule();
    missing_formula["formula1"] = Value::Null;
    let mut unknown_type = roles_rule();
    unknown_type["validation_type"] = json!("bogus");
    let mut long_title = roles_rule();
    long_title["prompt_title"] = json!("x".repeat(33));
    for (change, code) in [
        (
            json!({"kind": "set_data_validation", "sheet": "Data", "rule": missing_formula}),
            "validation.data_validation_formula_missing",
        ),
        (
            json!({"kind": "set_data_validation", "sheet": "Data", "rule": unknown_type}),
            "interop.change.invalid",
        ),
        (
            json!({"kind": "set_data_validation", "sheet": "Data", "rule": long_title}),
            "validation.data_validation_text_too_long",
        ),
        (
            json!({"kind": "remove_data_validation", "sheet": "Data", "index": 9}),
            "validation.data_validation_index_out_of_range",
        ),
    ] {
        let revision = session.summary().semantic_revision;
        let error = session
            .apply_changes_v2(revision, batch(json!([change])))
            .expect_err("invalid rule edit");
        assert_eq!(error.code(), code);
    }
}
