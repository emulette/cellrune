use cellrune_interop::{
    CalculationOptionsDto, CancellationToken, EditBatchV2Dto, PreviewCursorDto,
    RecalculationModeDto, TransactionDetailSectionDto, WorkbookChangeDto, WorkbookChangeV2Dto,
    WorkbookSession, WritableCellValueDto,
};

fn value_change(address: &str, value: f64) -> WorkbookChangeV2Dto {
    WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetValue {
        sheet: "Sheet1".to_owned(),
        address: address.to_owned(),
        value: WritableCellValueDto::Number { value },
    })
}

fn formula_change(address: &str, formula: &str) -> WorkbookChangeV2Dto {
    WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetFormula {
        sheet: "Sheet1".to_owned(),
        address: address.to_owned(),
        formula: formula.to_owned(),
        dynamic_range: None,
    })
}

fn batch(changes: impl IntoIterator<Item = WorkbookChangeV2Dto>) -> EditBatchV2Dto {
    EditBatchV2Dto {
        changes: changes.into_iter().collect(),
    }
}

fn calculated_session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session
        .apply_changes_v2(
            0,
            batch([
                value_change("A1", 1.0),
                formula_change("B1", "=A1+1"),
                formula_change("C1", "=B1+1"),
            ]),
        )
        .expect("initial edit");
    session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("initial calculation");
    session
}

#[test]
fn retained_preview_pages_with_lossless_cursor_and_commits_once() {
    let mut session = calculated_session();
    let preview = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 5.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("preview publishes");

    assert_eq!(preview.schema_version, 1);
    assert!(preview.report.base_calculation_reused);
    assert_eq!(
        preview.report.base_revision + 1,
        preview.report.result_revision
    );
    assert_eq!(
        preview.report.calculation_options.limits.max_array_cells,
        1_000_000
    );
    assert_eq!(preview.report.detail_counts.preview_results, 2);
    assert_eq!(preview.report.install_delta_count, 2);

    let first = session
        .preview_changes_page(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            1,
        )
        .expect("first detail page");
    assert_eq!(first.items.len(), 1);
    let cursor = first.next_cursor.clone().expect("second page cursor");
    let serialized = serde_json::to_string(&cursor).expect("cursor JSON");
    let round_trip: PreviewCursorDto = serde_json::from_str(&serialized).expect("cursor JSON");
    assert_eq!(round_trip, cursor);
    let second = session
        .preview_changes_page(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            Some(round_trip),
            1,
        )
        .expect("second detail page");
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());

    let first_page_bytes = serde_json::to_vec(&first)
        .expect("detail page serializes")
        .len();
    let exact_prefix = session
        .preview_changes_page_bounded(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            100,
            first_page_bytes,
        )
        .expect("byte bound selects the longest complete prefix");
    assert_eq!(exact_prefix, first);

    let bounded = session
        .preview_changes_page_bounded(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            100,
            1_024,
        )
        .expect("byte-bounded page");
    assert_eq!(bounded.items.len(), 2);
    assert!(bounded.next_cursor.is_none());

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let cancellation = session
        .commit_preview_cancellable(preview.preview_id, &cancelled)
        .expect_err("cancelled pre-commit preserves preview");
    assert_eq!(cancellation.code(), "session.cancelled");
    let expected_receipt = session
        .preview_commit_receipt(preview.preview_id)
        .expect("retained preview provides its exact pre-commit receipt");
    let receipt = session
        .commit_preview(preview.preview_id)
        .expect("explicit retry commits preview");
    assert_eq!(receipt, expected_receipt);
    assert_eq!(receipt.edit.result_revision, preview.report.result_revision);
    assert_eq!(receipt.calculation_delta.changed_cells.len(), 2);
    assert_eq!(
        session
            .preview_changes_page(
                preview.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
            )
            .expect_err("commit consumes preview")
            .code(),
        "interop.preview.not_found"
    );
}

#[test]
fn two_phase_publish_preserves_prior_preview_until_replacement_and_mutations_invalidate() {
    let mut session = calculated_session();
    let first = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 4.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("first preview");
    let prepared = session
        .prepare_preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 7.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("second preview stages");
    let request_id = prepared.request_id();
    let completed = prepared.run().expect("second preview calculates");
    assert_ne!(completed.preview_id(), first.preview_id);
    assert_eq!(completed.summary().preview_id, completed.preview_id());

    session.abandon_recalculation(request_id);
    assert!(
        session
            .preview_changes_page(
                first.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
            )
            .is_ok(),
        "unpublished candidate must not replace the existing preview"
    );

    let prepared = session
        .prepare_preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 7.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("replacement preview stages");
    let completed = prepared.run().expect("replacement preview calculates");
    let replacement = session
        .publish_preview(completed)
        .expect("publish replacement");
    assert_eq!(
        session
            .preview_changes_page(
                first.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
            )
            .expect_err("replacement consumes previous preview")
            .code(),
        "interop.preview.not_found"
    );

    session
        .apply_changes_v2(
            session.summary().semantic_revision,
            batch([value_change("A1", 9.0)]),
        )
        .expect("successful workbook mutation");
    assert_eq!(
        session
            .discard_preview(replacement.preview_id)
            .expect_err("mutation invalidates preview")
            .code(),
        "interop.preview.not_found"
    );

    let recalculation_preview = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 10.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("preview after edit publishes");
    session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("regular recalculation installs a new calculation generation");
    assert_eq!(
        session
            .discard_preview(recalculation_preview.preview_id)
            .expect_err("recalculation invalidates the captured calculation basis")
            .code(),
        "interop.preview.not_found"
    );
}

#[test]
fn preview_rejects_cross_preview_cursor_and_discard_consumes_id() {
    let mut session = calculated_session();
    let first = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 2.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("first preview");
    let first_page = session
        .preview_changes_page(
            first.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            1,
        )
        .expect("first page");
    let cursor = first_page.next_cursor.expect("cursor");
    let second = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 3.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("second preview");
    assert_eq!(
        session
            .preview_changes_page(
                second.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                Some(cursor),
                1,
            )
            .expect_err("cursor binds its issuing preview")
            .code(),
        "interop.preview.cursor_invalid"
    );
    session
        .discard_preview(second.preview_id)
        .expect("discard published preview");
    assert_eq!(
        session
            .discard_preview(second.preview_id)
            .expect_err("discard is terminal")
            .code(),
        "interop.preview.not_found"
    );
}

#[test]
fn failed_replacement_and_identity_preserving_edit_keep_the_published_preview() {
    let mut session = calculated_session();
    let published = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 4.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("first preview publishes");

    let error = session
        .preview_changes(
            session.summary().semantic_revision,
            batch(Vec::<WorkbookChangeV2Dto>::new()),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect_err("an empty replacement is rejected");
    assert_eq!(error.code(), "session.edit_batch_empty");
    assert!(
        session
            .preview_changes_page(
                published.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
            )
            .is_ok(),
        "failed replacement must preserve the published preview"
    );

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        session
            .prepare_preview_changes_cancellable(
                session.summary().semantic_revision,
                batch([value_change("A1", 8.0)]),
                RecalculationModeDto::Auto,
                CalculationOptionsDto::default(),
                &cancelled,
            )
            .expect_err("cancelled candidate conversion must not publish")
            .code(),
        "session.cancelled"
    );
    assert!(
        session
            .preview_changes_page(
                published.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
            )
            .is_ok(),
        "cancelled preparation must preserve the published preview"
    );

    let no_op = session
        .apply_changes_v2(
            session.summary().semantic_revision,
            batch([value_change("A1", 1.0)]),
        )
        .expect("identity-preserving edit succeeds");
    assert_eq!(
        no_op.receipt.base_revision, no_op.receipt.result_revision,
        "setting the existing value must preserve semantic identity"
    );
    session
        .commit_preview(published.preview_id)
        .expect("identity-preserving edit must not invalidate the preview");
}

#[test]
fn superseded_runs_cancel_and_live_mutation_rejects_stale_publication() {
    let mut session = calculated_session();
    let first = session
        .prepare_preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 2.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("first preview prepares");
    let second = session
        .prepare_preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 3.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("newer preview prepares");
    assert_eq!(
        first
            .run()
            .expect_err("newer request cancels the older active run")
            .code(),
        "session.cancelled"
    );

    let completed = second.run().expect("newer preview calculates");
    session
        .apply_changes_v2(
            session.summary().semantic_revision,
            batch([value_change("A1", 9.0)]),
        )
        .expect("concurrent live mutation succeeds");
    let preview_id = completed.preview_id();
    assert_eq!(
        session
            .publish_preview(completed)
            .expect_err("changed live basis rejects stale publication")
            .code(),
        "session.stale_result"
    );
    assert!(!session.preview_or_calculation_active());
    assert_eq!(
        session
            .discard_preview(preview_id)
            .expect_err("stale candidate was never retained")
            .code(),
        "interop.preview.not_found"
    );
}

#[test]
fn cancelled_publish_and_too_small_page_preserve_the_existing_preview() {
    let mut session = calculated_session();
    let published = session
        .preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 4.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("first preview publishes");
    let prepared = session
        .prepare_preview_changes(
            session.summary().semantic_revision,
            batch([value_change("A1", 5.0)]),
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("replacement prepares");
    let request_id = prepared.request_id();
    let completed = prepared.run().expect("replacement calculates");
    assert!(session.cancel_recalculation(request_id));
    assert_eq!(
        session
            .publish_preview(completed)
            .expect_err("cancelled active request cannot publish")
            .code(),
        "session.cancelled"
    );
    assert!(!session.preview_or_calculation_active());

    assert_eq!(
        session
            .preview_changes_page_bounded(
                published.preview_id,
                TransactionDetailSectionDto::PreviewResults,
                None,
                1,
                1,
            )
            .expect_err("one item cannot fit in one byte")
            .code(),
        "interop.preview.response_limit_exceeded"
    );
    let page = session
        .preview_changes_page(
            published.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            1,
        )
        .expect("failed bounded page must not advance the cursor");
    assert_eq!(page.items.len(), 1);
}
