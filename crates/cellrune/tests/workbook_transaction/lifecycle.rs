use super::*;

#[test]
fn transaction_runs_off_lock_and_installs_the_reported_generation() {
    let mut session = calculated_session();
    let base_revision = session.workbook().semantic_revision();
    let base_fingerprint = session.workbook().fingerprint();
    let base_cursor = session
        .changes_since(0, 100)
        .expect("initial history page")
        .deltas()[0]
        .cursor();
    let base_b1 = session
        .calculation()
        .and_then(|calculation| calculation.cell(CalculationCellId::new(sheet(), address("B1"))))
        .expect("base formula result")
        .clone();

    let prepared = session
        .prepare_transaction(
            base_revision,
            EditBatch::new([set_value("A1", 5.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("transaction prepares");
    assert_eq!(prepared.base_workbook().fingerprint(), base_fingerprint);
    assert_ne!(
        prepared.candidate_workbook().fingerprint(),
        base_fingerprint
    );
    let mut completed = prepared.run().expect("transaction calculates");

    assert_eq!(session.workbook().semantic_revision(), base_revision);
    assert_eq!(session.workbook().fingerprint(), base_fingerprint);
    assert_eq!(
        session
            .calculation()
            .and_then(|calculation| {
                calculation.cell(CalculationCellId::new(sheet(), address("B1")))
            })
            .expect("live calculation remains installed"),
        &base_b1
    );
    assert!(completed.report().base_calculation_reused());
    assert_eq!(completed.report().base_evaluated_count(), 0);
    assert_eq!(completed.report().candidate_evaluated_count(), 2);
    assert_eq!(completed.report().direct_affected_count(), 1);
    assert_eq!(completed.report().transitive_affected_count(), 1);
    assert_eq!(completed.report().conservative_affected_count(), 0);
    assert_eq!(
        completed.report().impact_coverage(),
        TransactionImpactCoverage::Exact
    );
    assert_eq!(completed.report().preview_changed_count(), 2);
    assert_eq!(completed.report().install_delta().changed_cells().len(), 2);
    assert!(
        !completed
            .report()
            .install_delta_basis_differs_from_preview_base()
    );

    let reported_edit = completed.report().edit_receipt().clone();
    let reported_delta = completed.report().install_delta().clone();
    let receipt = session
        .install_transaction(&mut completed)
        .expect("transaction installs");
    assert_eq!(receipt.edit(), &reported_edit);
    assert_eq!(receipt.calculation_delta(), &reported_delta);
    assert_eq!(receipt.calculation_delta().cursor(), base_cursor + 1);
    assert_eq!(
        session.workbook().fingerprint(),
        receipt.result_fingerprint()
    );
    assert_eq!(
        session
            .calculation()
            .expect("candidate calculation installed")
            .source_fingerprint(),
        receipt.result_fingerprint()
    );
    let history = session
        .changes_since(base_cursor, 100)
        .expect("transaction history page");
    assert_eq!(history.deltas(), &[reported_delta]);
    assert_eq!(
        completed
            .page(TransactionDetailSection::Affected, None, 1)
            .expect_err("installed transaction is no longer pageable")
            .code(),
        SessionErrorCode::TransactionConsumed
    );
    assert_eq!(
        session
            .install_transaction(&mut completed)
            .expect_err("transaction is one-shot")
            .code(),
        SessionErrorCode::TransactionConsumed
    );
}

#[test]
fn pending_edits_are_in_preview_base_but_remain_in_install_delta_basis() {
    let mut session = calculated_session();
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 2.0)]),
        )
        .expect("pending base edit installs");
    let prepared = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 3.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("transaction prepares");
    let completed = prepared.run().expect("transaction calculates");

    assert!(!completed.report().base_calculation_reused());
    assert!(completed.report().base_evaluated_count() > 0);
    assert_eq!(
        completed.report().install_delta_basis_reasons(),
        &[InstallDeltaBasisReason::PriorPendingEdits]
    );
    let preview = completed
        .page(TransactionDetailSection::PreviewResults, None, 100)
        .expect("preview page");
    let b1 = preview
        .items()
        .iter()
        .find_map(|item| match item {
            TransactionDetailItem::PreviewResult(change)
                if change.cell() == CalculationCellId::new(sheet(), address("B1")) =>
            {
                Some(change)
            }
            _ => None,
        })
        .expect("B1 preview change");
    assert_eq!(
        result_number(b1.previous_result().expect("base result")),
        3.0
    );
    assert_eq!(result_number(b1.result().expect("candidate result")), 4.0);
    let install_b1 = completed
        .report()
        .install_delta()
        .changed_cells()
        .iter()
        .find(|change| change.cell() == CalculationCellId::new(sheet(), address("B1")))
        .expect("B1 install change");
    assert_eq!(result_number(install_b1.result()), 4.0);
}

#[test]
fn cancellation_is_retryable_while_stale_and_discard_are_terminal() {
    let mut session = calculated_session();
    let run_cancellation = CancellationToken::new();
    let run_prepared = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 6.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            run_cancellation.clone(),
        )
        .expect("cancellable transaction prepares");
    let live_fingerprint = session.workbook().fingerprint();
    let live_calculation = session.calculation().expect("live calculation exists") as *const _;
    let live_history = session
        .changes_since(0, 100)
        .expect("live history snapshot")
        .deltas()
        .to_vec();
    run_cancellation.cancel();
    assert_eq!(
        run_prepared
            .run()
            .expect_err("cancelled run does not complete")
            .code(),
        SessionErrorCode::Cancelled
    );
    assert_eq!(session.workbook().fingerprint(), live_fingerprint);
    assert!(std::ptr::eq(
        session
            .calculation()
            .expect("calculation remains installed"),
        live_calculation
    ));
    assert_eq!(
        session
            .changes_since(0, 100)
            .expect("history remains readable")
            .deltas(),
        live_history
    );

    let mut completed = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 7.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("transaction prepares")
        .run()
        .expect("transaction calculates");
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let before_commit_fingerprint = session.workbook().fingerprint();
    let before_commit_calculation = session
        .calculation()
        .expect("live calculation exists before commit")
        as *const _;
    let before_commit_history = session
        .changes_since(0, 100)
        .expect("history before commit")
        .deltas()
        .to_vec();
    assert_eq!(
        session
            .install_transaction_cancellable(&mut completed, &cancelled)
            .expect_err("cancelled commit does not install")
            .code(),
        SessionErrorCode::Cancelled
    );
    assert_eq!(session.workbook().fingerprint(), before_commit_fingerprint);
    assert!(std::ptr::eq(
        session
            .calculation()
            .expect("calculation remains installed after cancelled commit"),
        before_commit_calculation
    ));
    assert_eq!(
        session
            .changes_since(0, 100)
            .expect("history after cancelled commit")
            .deltas(),
        before_commit_history
    );
    session
        .install_transaction(&mut completed)
        .expect("explicit retry installs");

    let mut discarded = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 8.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("discard transaction prepares")
        .run()
        .expect("discard transaction calculates");
    discarded.discard().expect("first discard succeeds");
    assert_eq!(
        discarded
            .page(TransactionDetailSection::Affected, None, 1)
            .expect_err("discarded transaction is no longer pageable")
            .code(),
        SessionErrorCode::TransactionConsumed
    );
    assert_eq!(
        discarded
            .discard()
            .expect_err("second discard is terminal")
            .code(),
        SessionErrorCode::TransactionConsumed
    );

    let mut stale = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 9.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("stale transaction prepares")
        .run()
        .expect("stale transaction calculates");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 10.0)]),
        )
        .expect("intervening edit installs");
    assert_eq!(
        session
            .install_transaction(&mut stale)
            .expect_err("intervening edit makes transaction stale")
            .code(),
        SessionErrorCode::StaleResult
    );
    assert_eq!(
        stale
            .page(TransactionDetailSection::Affected, None, 1)
            .expect_err("stale transaction is no longer pageable")
            .code(),
        SessionErrorCode::TransactionConsumed
    );
    assert_eq!(
        session
            .install_transaction(&mut stale)
            .expect_err("stale transaction is terminal")
            .code(),
        SessionErrorCode::TransactionConsumed
    );
}
