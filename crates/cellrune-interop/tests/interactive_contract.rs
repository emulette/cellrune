use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellValueDto, EditBatchDto, InteropErrorKind,
    RangeRequestDto, RecalculationModeDto, WorkbookChangeDto, WorkbookSession,
    WritableCellValueDto,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    initial_changes: Vec<WorkbookChangeDto>,
    incremental_changes: Vec<WorkbookChangeDto>,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    initial_revision: u64,
    incremental_revision: u64,
    incremental_mode: String,
    incremental_evaluated_count: u64,
    b1: f64,
    c1: f64,
    revision_error: String,
}

#[test]
fn versioned_interactive_corpus_matches_batch_delta_and_error_contract() {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../binding-contract/interactive-v1.json"
    ))
    .expect("valid interactive corpus");
    assert_eq!(corpus.schema_version, 1);
    let mut session = WorkbookSession::create();

    let receipt = session
        .apply_changes(
            0,
            EditBatchDto {
                changes: corpus.initial_changes,
            },
        )
        .expect("initial batch");
    assert_eq!(receipt.result_revision, corpus.expected.initial_revision);
    assert_eq!(receipt.applied_change_count, 4);
    assert_eq!(receipt.calculation_changed_cells.len(), 4);
    assert!(!receipt.calculation_metadata_changed);

    let initial = session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("initial calculation");
    assert_eq!(initial.mode, "full");
    assert_eq!(initial.result_revision, corpus.expected.initial_revision);

    let receipt = session
        .apply_changes(
            receipt.result_revision,
            EditBatchDto {
                changes: corpus.incremental_changes,
            },
        )
        .expect("incremental edit");
    assert_eq!(
        receipt.result_revision,
        corpus.expected.incremental_revision
    );
    assert_eq!(receipt.calculation_changed_cells.len(), 1);
    let delta = session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("incremental calculation");
    assert_eq!(delta.mode, corpus.expected.incremental_mode);
    assert_eq!(
        delta.evaluated_count,
        corpus.expected.incremental_evaluated_count
    );
    assert_eq!(delta.parsed_formula_count, 0);

    let page = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "B1".to_owned(),
            end: "C1".to_owned(),
            offset: 0,
            limit: 2,
        })
        .expect("calculated range");
    assert_number(
        page.cells[0].calculated.as_ref().expect("B1 calculation"),
        corpus.expected.b1,
    );
    assert_number(
        page.cells[1].calculated.as_ref().expect("C1 calculation"),
        corpus.expected.c1,
    );

    let history = session.changes_since(0, 1).expect("first delta page");
    assert_eq!(history.deltas.len(), 1);
    assert!(history.next_cursor.is_some());
    let next = session
        .changes_since(history.next_cursor.expect("next cursor"), 1)
        .expect("second delta page");
    assert_eq!(next.deltas.len(), 1);
    assert!(next.next_cursor.is_none());

    let error = session
        .apply_changes(
            0,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::SetValue {
                    sheet: "Sheet1".to_owned(),
                    address: "A1".to_owned(),
                    value: WritableCellValueDto::Number { value: 99.0 },
                }],
            },
        )
        .expect_err("stale edit rejected");
    assert_eq!(error.kind(), InteropErrorKind::State);
    assert_eq!(error.code(), corpus.expected.revision_error);
}

#[test]
fn prepared_interop_calculation_rejects_stale_install_and_supports_cancel() {
    let mut session = WorkbookSession::create();
    session
        .apply_changes(
            0,
            EditBatchDto {
                changes: vec![
                    WorkbookChangeDto::SetValue {
                        sheet: "Sheet1".to_owned(),
                        address: "A1".to_owned(),
                        value: WritableCellValueDto::Number { value: 1.0 },
                    },
                    WorkbookChangeDto::SetFormula {
                        sheet: "Sheet1".to_owned(),
                        address: "B1".to_owned(),
                        formula: "=A1+1".to_owned(),
                        dynamic_range: None,
                    },
                ],
            },
        )
        .expect("initial batch");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("initial calculation");
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 2.0 })
        .expect("first edit");
    let prepared = session
        .prepare_recalculation(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("prepared work");
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 3.0 })
        .expect("newer edit");
    let completed = prepared.run().expect("older work completes");
    let error = session
        .install_recalculation(completed)
        .expect_err("stale result rejected");
    assert_eq!(error.code(), "session.stale_result");

    let prepared = session
        .prepare_recalculation(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .expect("prepared cancellable work");
    let request_id = prepared.request_id();
    assert!(session.calculation_active());
    assert!(session.cancel_calculation());
    let error = prepared.run().expect_err("cancelled calculation");
    assert_eq!(error.code(), "session.cancelled");
    session.abandon_recalculation(request_id);
    assert!(!session.calculation_active());
}

#[test]
fn rejected_recalculation_request_does_not_cancel_the_active_request() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=1+1", None)
        .expect("formula must be accepted");
    let active = session
        .prepare_recalculation(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .expect("full request must be prepared");

    let unsafe_error = session
        .prepare_recalculation(
            RecalculationModeDto::Incremental,
            CalculationOptionsDto::default(),
        )
        .expect_err("incremental calculation requires initialized state");
    assert_eq!(unsafe_error.code(), "session.calculation_uninitialized");
    let invalid_error = session
        .prepare_recalculation(
            RecalculationModeDto::Full,
            CalculationOptionsDto {
                today_serial: Some(f64::NAN),
                now_serial: None,
            },
        )
        .expect_err("non-finite options must be rejected");
    assert_eq!(invalid_error.code(), "validation.non_finite_number");

    let completed = active
        .run()
        .expect("rejected newer requests must not cancel active work");
    let delta = session
        .install_recalculation(completed)
        .expect("the original active request must remain installable");
    assert_eq!(delta.result_revision, 1);
}

#[test]
fn request_scoped_cancellation_cannot_cancel_a_newer_request() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=1+1", None)
        .expect("formula must be accepted");
    let older = session
        .prepare_recalculation(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .expect("older request must be prepared");
    let older_request_id = older.request_id();
    let newer = session
        .prepare_recalculation(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .expect("newer request must supersede older work");

    assert!(
        !session.cancel_recalculation(older_request_id),
        "late cancellation for an older request must not reach the active token"
    );
    assert_eq!(
        older.run().expect_err("superseded work must stop").code(),
        "session.cancelled"
    );
    let completed = newer
        .run()
        .expect("the newer request must remain unaffected");
    session
        .install_recalculation(completed)
        .expect("the newer request must remain installable");
}

fn assert_number(result: &CalculationResultDto, expected: f64) {
    assert_eq!(
        result,
        &CalculationResultDto::Value {
            value: CellValueDto::Number { value: expected },
        }
    );
}
