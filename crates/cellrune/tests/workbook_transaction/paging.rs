use super::*;

#[test]
fn detail_paging_is_report_bound_and_total_detail_is_bounded() {
    let session = calculated_session();
    let make_completed = || {
        session
            .prepare_transaction(
                session.workbook().semantic_revision(),
                EditBatch::new([set_value("A1", 11.0)]),
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("transaction prepares")
            .run()
            .expect("transaction calculates")
    };
    let first = make_completed();
    let second = make_completed();
    let first_page = first
        .page(TransactionDetailSection::Affected, None, 1)
        .expect("first affected page");
    let cursor = first_page.next_cursor().expect("second affected page");
    let second_page = first
        .page(TransactionDetailSection::Affected, Some(cursor), 1)
        .expect("cursor continues same report and section");
    assert_eq!(second_page.items().len(), 1);
    assert!(second_page.next_cursor().is_none());
    let token = cursor.to_token();
    assert_eq!(token.len(), 54);
    assert_eq!(
        first
            .page_from_token(TransactionDetailSection::Affected, Some(token.as_str()), 1,)
            .expect("opaque cursor token round trips"),
        second_page
    );
    assert_eq!(
        second
            .page_from_token(TransactionDetailSection::Affected, Some(token.as_str()), 1,)
            .expect_err("opaque token cannot cross reports")
            .code(),
        SessionErrorCode::TransactionCursorInvalid
    );
    assert_eq!(
        first
            .page_from_token(TransactionDetailSection::Evaluated, Some(token.as_str()), 1,)
            .expect_err("opaque token cannot cross sections")
            .code(),
        SessionErrorCode::TransactionCursorInvalid
    );
    let mut tampered = token.into_bytes();
    let last = tampered.last_mut().expect("cursor token is non-empty");
    *last = if *last == b'0' { b'1' } else { b'0' };
    let tampered = String::from_utf8(tampered).expect("tamper remains ASCII");
    assert_eq!(
        first
            .page_from_token(
                TransactionDetailSection::Affected,
                Some(tampered.as_str()),
                1,
            )
            .expect_err("tampered cursor token is rejected")
            .code(),
        SessionErrorCode::TransactionCursorInvalid
    );
    assert_eq!(
        second
            .page(TransactionDetailSection::Affected, Some(cursor), 1)
            .expect_err("cursor cannot cross reports")
            .code(),
        SessionErrorCode::TransactionCursorInvalid
    );
    assert_eq!(
        first
            .page(TransactionDetailSection::Evaluated, Some(cursor), 1)
            .expect_err("cursor cannot cross sections")
            .code(),
        SessionErrorCode::TransactionCursorInvalid
    );
    assert_eq!(
        first
            .page(TransactionDetailSection::Affected, None, 1_001)
            .expect_err("page limit is enforced")
            .code(),
        SessionErrorCode::PageLimitExceeded
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        first
            .page_cancellable(TransactionDetailSection::Affected, None, 1, &cancelled,)
            .expect_err("cancelled page clone is retryable")
            .code(),
        SessionErrorCode::Cancelled
    );
    assert_eq!(
        first
            .page(TransactionDetailSection::Affected, None, 1)
            .expect("report remains pageable after cancellation")
            .items()
            .len(),
        1
    );

    for section in [
        TransactionDetailSection::Affected,
        TransactionDetailSection::Evaluated,
        TransactionDetailSection::PreviewResults,
        TransactionDetailSection::PreviewIssues,
        TransactionDetailSection::InstallResults,
    ] {
        let collect = |transaction: &cellrune::CompletedWorkbookTransaction| {
            let mut cursor = None;
            let mut cells = Vec::new();
            loop {
                let page = transaction
                    .page(section, cursor.as_ref(), 1)
                    .expect("every detail page remains complete");
                cells.extend(page.items().iter().filter_map(detail_cell));
                cursor = page.next_cursor().cloned();
                if cursor.is_none() {
                    break;
                }
            }
            cells
        };
        let first_cells = collect(&first);
        let second_cells = collect(&second);
        assert_eq!(first_cells.len(), first.report().detail_count(section));
        assert_eq!(first_cells, second_cells);
        assert!(first_cells.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    let limits = SessionLimits::default()
        .with_transaction_detail_limits(1, 1)
        .expect("valid transaction limits");
    let mut limited =
        WorkbookCalculationSession::with_limits(cellrune::WorkbookDraft::new(), limits);
    limited
        .apply_changes(
            0,
            EditBatch::new([
                set_value("A1", 1.0),
                WorkbookChange::set_cell_formula(sheet(), address("B1"), formula("A1+1")),
            ]),
        )
        .expect("limited initial edit");
    limited
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("limited initial calculation");
    let error = limited
        .prepare_transaction(
            limited.workbook().semantic_revision(),
            EditBatch::new([set_value("A1", 2.0)]),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("limited transaction prepares")
        .run()
        .expect_err("total detail limit rejects partial report");
    assert_eq!(
        error.code(),
        SessionErrorCode::TransactionDetailLimitExceeded
    );
}
