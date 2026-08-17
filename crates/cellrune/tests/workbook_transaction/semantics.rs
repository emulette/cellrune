use super::*;

#[test]
fn uncached_base_uses_requested_options_and_transaction_cumulative_work_limits() {
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                set_value("A1", 1.0),
                WorkbookChange::set_cell_formula(
                    sheet(),
                    address("B1"),
                    formula("SEQUENCE(1,1,A1)"),
                ),
            ]),
        )
        .expect("uncalculated base edit installs");

    let boundary_limits = CalculationLimits::default()
        .with_max_function_iterations(2)
        .expect("non-zero function limit");
    let boundary = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 2.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default().with_limits(boundary_limits),
            CancellationToken::new(),
        )
        .expect("boundary transaction prepares")
        .run()
        .expect("base and candidate fit the cumulative boundary");
    assert!(!boundary.report().base_calculation_reused());
    assert_eq!(boundary.report().base_evaluated_count(), 1);
    assert_eq!(boundary.report().candidate_evaluated_count(), 1);
    assert_eq!(boundary.report().function_iteration_count(), 2);
    assert_eq!(boundary.report().reference_cell_count(), 2);
    assert_eq!(
        boundary.report().install_delta_basis_reasons(),
        &[InstallDeltaBasisReason::NoInstalledCalculation]
    );

    let exceeded_limits = CalculationLimits::default()
        .with_max_function_iterations(1)
        .expect("non-zero function limit");
    let error = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 2.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default().with_limits(exceeded_limits),
            CancellationToken::new(),
        )
        .expect("over-budget transaction prepares")
        .run()
        .expect_err("two individually valid passes cannot evade the cumulative limit");
    assert_eq!(
        error.code(),
        SessionErrorCode::TransactionResourceLimitExceeded
    );
    let stage_error = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                sheet(),
                address("B1"),
                formula("SEQUENCE(1,2,A1)"),
            )]),
            RecalculationMode::Auto,
            CalculationOptions::default().with_limits(exceeded_limits),
            CancellationToken::new(),
        )
        .expect("per-pass limit transaction prepares")
        .run()
        .expect_err("a candidate evaluator resource issue cannot become a completed preview");
    assert_eq!(
        stage_error.code(),
        SessionErrorCode::TransactionResourceLimitExceeded
    );
    assert!(session.calculation().is_none());
    assert_eq!(
        session
            .workbook()
            .sheet_by_id(sheet())
            .and_then(|sheet| sheet.cell(address("A1")))
            .map(|cell| cell.content()),
        Some(&CellContent::Literal(number(1.0)))
    );
}

#[test]
fn options_mismatch_builds_the_base_and_topology_reports_conservative_impact() {
    let session = calculated_session();
    let alternate_limits = CalculationLimits::default()
        .with_max_text_bytes(1_024)
        .expect("non-zero text limit");
    let options_completed = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 4.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default().with_limits(alternate_limits),
            CancellationToken::new(),
        )
        .expect("options transaction prepares")
        .run()
        .expect("options transaction calculates an immutable base");
    assert!(!options_completed.report().base_calculation_reused());
    assert!(options_completed.report().base_evaluated_count() > 0);
    assert_eq!(
        options_completed.report().install_delta_basis_reasons(),
        &[InstallDeltaBasisReason::CalculationOptionsChanged]
    );

    let topology_edit = EditBatch::new([WorkbookChange::set_cell_formula(
        sheet(),
        address("D1"),
        formula("A1+3"),
    )]);
    let incremental_error = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            topology_edit.clone(),
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("forced incremental transaction prepares")
        .run()
        .expect_err("topology changes are not forced through incremental calculation");
    assert_eq!(
        incremental_error.code(),
        SessionErrorCode::IncrementalUnsafe
    );

    let topology_completed = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            topology_edit,
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("topology transaction prepares")
        .run()
        .expect("auto selects a safe full calculation");
    assert_eq!(
        topology_completed.report().impact_coverage(),
        TransactionImpactCoverage::ConservativeFull
    );
    assert_eq!(topology_completed.report().direct_affected_count(), 0);
    assert_eq!(topology_completed.report().transitive_affected_count(), 0);
    assert!(topology_completed.report().conservative_affected_count() >= 3);
    let affected = topology_completed
        .page(TransactionDetailSection::Affected, None, 0)
        .expect("complete affected page");
    assert!(affected.items().iter().all(|item| matches!(
        item,
        TransactionDetailItem::Affected(detail)
            if detail.cause() == TransactionImpactCause::Conservative
    )));

    let mut dynamic = WorkbookCalculationSession::create();
    dynamic
        .apply_changes(
            0,
            EditBatch::new([
                set_value("A1", 1.0),
                WorkbookChange::set_cell_formula(
                    sheet(),
                    address("B1"),
                    formula("INDIRECT(\"A1\")"),
                ),
            ]),
        )
        .expect("dynamic reference fixture installs");
    dynamic
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("dynamic reference fixture calculates");
    let dynamic_completed = dynamic
        .prepare_transaction(
            dynamic.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 2.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("dynamic transaction prepares")
        .run()
        .expect("dynamic transaction falls back safely");
    assert_eq!(
        dynamic_completed.report().impact_coverage(),
        TransactionImpactCoverage::ConservativeFull
    );
    assert!(dynamic_completed.report().conservative_affected_count() >= 1);
}

#[test]
fn issue_differences_and_empty_transactions_are_exact() {
    let mut session = calculated_session();
    let empty_error = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::default(),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("empty transactions retain the edit API's explicit rejection");
    assert!(matches!(
        empty_error,
        cellrune::ApplyChangesError::Session(error)
            if error.code() == SessionErrorCode::EmptyBatch
    ));
    let empty = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 1.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("semantic no-op transaction prepares")
        .run()
        .expect("semantic no-op transaction completes");
    assert!(empty.report().base_calculation_reused());
    assert_eq!(empty.report().candidate_evaluated_count(), 0);
    assert_eq!(empty.report().preview_changed_count(), 0);
    assert_eq!(empty.report().preview_removed_count(), 0);

    let mut completed = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                sheet(),
                address("B1"),
                formula("MISSINGFUNCTION(A1)"),
            )]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("issue transaction prepares")
        .run()
        .expect("unsupported function is reported as a calculation issue");
    assert_eq!(completed.report().introduced_issue_count(), 2);
    assert_eq!(completed.report().resolved_issue_count(), 0);
    assert_eq!(completed.report().changed_issue_count(), 0);
    let issues = completed
        .page(TransactionDetailSection::PreviewIssues, None, 0)
        .expect("issue details page");
    assert_eq!(issues.items().len(), 2);
    assert!(issues.items().iter().all(|item| matches!(
        item,
        TransactionDetailItem::PreviewIssue(change)
            if change.kind() == TransactionIssueChangeKind::Introduced
    )));

    session
        .install_transaction(&mut completed)
        .expect("issue-producing transaction installs");
    let resolved = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                sheet(),
                address("B1"),
                formula("A1+1"),
            )]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("issue-resolving transaction prepares")
        .run()
        .expect("issue-resolving transaction completes");
    assert_eq!(resolved.report().introduced_issue_count(), 0);
    assert_eq!(resolved.report().resolved_issue_count(), 2);
    assert_eq!(resolved.report().changed_issue_count(), 0);
}
