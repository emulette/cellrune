//! Deterministic 0.1.15 performance-axis acceptance suite (O6).
//!
//! These are deterministic correctness/counter checks, not wall-clock or RSS evidence. The
//! manual latency/memory acceptance (O5) is collected separately from the same commit.

use cellrune::testing::{
    WorkCounter, lock_work_counters, reset_work_counters, snapshot_work_counters,
};
use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SessionLimits, SheetId, WorkbookCalculationSession,
    WorkbookChange, calculate_workbook,
};

fn sheet() -> SheetId {
    SheetId::new(1).expect("valid default sheet ID")
}

fn address(a1: &str) -> CellAddress {
    CellAddress::from_a1(a1).expect("valid test address")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite test number"))
}

#[test]
fn public_source_and_result_iteration_remain_row_major() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("IW1"),
                    FormulaText::from_xlsx("1+1").expect("formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("A2"),
                    FormulaText::from_xlsx("2+2").expect("formula"),
                ),
            ]),
        )
        .expect("sparse workbook");
    assert_eq!(
        session.workbook().sheets()[0]
            .cells()
            .map(|cell| cell.address())
            .collect::<Vec<_>>(),
        vec![address("IW1"), address("A2")]
    );
    let calculation = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_eq!(
        calculation
            .cells()
            .map(|(cell, _)| cell.address())
            .collect::<Vec<_>>(),
        vec![address("IW1"), address("A2")]
    );
}

fn column_name(mut column: u32) -> String {
    let mut reversed = Vec::new();
    while column > 0 {
        column -= 1;
        reversed.push((b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    reversed.into_iter().rev().collect()
}

/// `acc_o1_no_payload_clone`: a single edit of a dense-wide (1 x 16384) sheet must deep-clone
/// zero unchanged Cell/CalculationCellResult payload bytes. Per-entry `Arc` sharing keeps the
/// deep-clone counter at zero even though the affected leaf is structurally rebuilt.
#[test]
fn dense_wide_edit_deep_clones_no_unchanged_payload() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut changes = Vec::with_capacity(16_384);
    for column in 1..=16_384 {
        changes.push(WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_indices(1, column).expect("valid dense-wide input"),
            number(1.0),
        ));
    }
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("dense-wide workbook");

    reset_work_counters();
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                CellAddress::from_indices(1, 8_192).expect("valid edit address"),
                number(2.0),
            )]),
        )
        .expect("single dense-wide edit");

    let snapshot = snapshot_work_counters();
    assert_eq!(
        snapshot.get(WorkCounter::CellStorePayloadBytesDeepCloned),
        0
    );
    assert_eq!(
        snapshot.get(WorkCounter::ResultStorePayloadBytesDeepCloned),
        0
    );
    assert_eq!(snapshot.get(WorkCounter::CellStoreLeavesRebuilt), 1);
    assert_eq!(snapshot.get(WorkCounter::CellStoreEntriesReindexed), 1);
    assert!(snapshot.get(WorkCounter::CellStoreNodesCopied) <= 17);
}

#[test]
fn cell_edit_work_is_width_independent_and_large_payload_is_shared() {
    let _guard = lock_work_counters();
    let measure = |width: u32, value: CellValue| {
        let sheet = sheet();
        let mut session = WorkbookCalculationSession::create();
        let changes = (1..=width).map(|column| {
            WorkbookChange::set_cell_value(
                sheet,
                CellAddress::from_indices(1, column).expect("dense address"),
                value.clone(),
            )
        });
        session
            .apply_changes(0, EditBatch::new(changes.collect::<Vec<_>>()))
            .expect("dense workbook");
        reset_work_counters();
        session
            .apply_changes(
                1,
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    CellAddress::from_indices(1, width / 2).expect("middle address"),
                    number(7.0),
                )]),
            )
            .expect("single edit");
        snapshot_work_counters()
    };

    let narrow = measure(1_024, number(1.0));
    let wide = measure(16_384, CellValue::Text("x".repeat(8_192)));
    for snapshot in [narrow, wide] {
        assert_eq!(snapshot.get(WorkCounter::CellStoreLeavesRebuilt), 1);
        assert_eq!(snapshot.get(WorkCounter::CellStoreEntriesReindexed), 1);
        assert_eq!(
            snapshot.get(WorkCounter::CellStorePayloadBytesDeepCloned),
            0
        );
        assert!(snapshot.get(WorkCounter::CellStoreNodesCopied) <= 17);
    }
}

#[test]
fn one_dirty_result_patch_copies_one_bounded_path() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(8_192);
    for column in 1..=4_096 {
        changes.push(WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_indices(2, column).expect("input address"),
            number(1.0),
        ));
        changes.push(WorkbookChange::set_cell_formula(
            sheet,
            CellAddress::from_indices(1, column).expect("formula address"),
            FormulaText::from_xlsx(format!("{}2+1", column_name(column)))
                .expect("generated independent formula"),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("dense result workbook");
    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial result calculation");

    reset_work_counters();
    session
        .apply_changes(
            1,
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address("A2"),
                number(2.0),
            )]),
        )
        .expect("one dirty input");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("one dirty result patch");
    let snapshot = snapshot_work_counters();
    assert_eq!(snapshot.get(WorkCounter::ResultStoreLeavesRebuilt), 1);
    assert_eq!(snapshot.get(WorkCounter::ResultStoreEntriesReindexed), 1);
    // The compressed branch plus one bounded packed leaf are rebuilt.
    assert_eq!(snapshot.get(WorkCounter::ResultStoreNodesCopied), 2);
    assert_eq!(
        snapshot.get(WorkCounter::ResultStorePayloadBytesDeepCloned),
        0
    );
}

/// `acc_o4_hash_work` (no-dirty half): a no-dirty recalculation must reuse the cached workbook
/// root fingerprint and hash zero payload leaves or internal nodes.
#[test]
fn no_dirty_rebase_reuses_cached_fingerprint() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("B1"),
                    FormulaText::from_xlsx("A1+1").expect("valid formula"),
                ),
            ]),
        )
        .expect("fingerprint workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    reset_work_counters();
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("no-dirty calculation");

    let snapshot = snapshot_work_counters();
    assert_eq!(snapshot.get(WorkCounter::FingerprintPayloadLeavesHashed), 0);
    assert_eq!(snapshot.get(WorkCounter::FingerprintInternalNodesHashed), 0);
    assert!(snapshot.get(WorkCounter::FingerprintRootCacheHits) > 0);
}

#[test]
fn single_edit_hashes_one_payload_and_one_bounded_radix_path() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    let changes = (1..=4_096).map(|column| {
        WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_indices(1, column).expect("dense address"),
            number(1.0),
        )
    });
    session
        .apply_changes(0, EditBatch::new(changes.collect::<Vec<_>>()))
        .expect("dense fingerprint workbook");
    calculate_workbook(session.workbook(), CalculationOptions::default());

    session
        .apply_changes(
            1,
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("changed fingerprint workbook");
    reset_work_counters();
    calculate_workbook(session.workbook(), CalculationOptions::default());
    let snapshot = snapshot_work_counters();
    assert_eq!(snapshot.get(WorkCounter::FingerprintPayloadLeavesHashed), 1);
    // One changed cell rehashes the compressed cell-radix branch, the sheet envelope, and the
    // workbook envelope. The one-sheet identity stays in a packed leaf. No count depends on width.
    assert_eq!(snapshot.get(WorkCounter::FingerprintInternalNodesHashed), 3);
    assert!(snapshot.get(WorkCounter::FingerprintCachedNodesReused) > 0);
}

/// `acc_full_incremental`: a fixed-income-style workbook keeps value, error, and spill-shape
/// parity between a full calculation and an incremental recalculation after a one-cell edit.
#[test]
fn full_and_incremental_calculation_agree_after_an_edit() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("B1"),
                    FormulaText::from_xlsx("A1+1").expect("valid dependent formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("C1"),
                    FormulaText::from_xlsx("B1*2").expect("valid transitive formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("D1"),
                    FormulaText::from_xlsx("SEQUENCE(1,3,1,1)").expect("valid spill formula"),
                ),
            ]),
        )
        .expect("fixed-income workbook");

    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full calculation");

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("one-cell edit");

    let incremental = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");

    let incremental_cells = session
        .calculation()
        .expect("incremental state")
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect::<Vec<_>>();
    let fresh = calculate_workbook(session.workbook(), CalculationOptions::default());
    let fresh_cells = fresh
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect::<Vec<_>>();
    assert_eq!(incremental_cells, fresh_cells);
    assert!(incremental.evaluated_count() > 0);
    assert!(incremental.evaluated_count() < fresh.len());
}

#[test]
fn large_spill_owner_patch_shares_unchanged_results() {
    let _guard = lock_work_counters();
    let sheet = sheet();
    let limits =
        SessionLimits::new(100_000, 1_000_000, 1_000_000, 256, 100).expect("session limits");
    let mut session = WorkbookCalculationSession::with_limits(Default::default(), limits);
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("A1"),
                    FormulaText::from_xlsx("SEQUENCE(1,8192)").expect("large spill"),
                ),
                WorkbookChange::set_cell_value(sheet, address("A2"), number(1.0)),
            ]),
        )
        .expect("spill workbook");
    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("spill calculation");
    reset_work_counters();
    session
        .apply_changes(
            1,
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address("A2"),
                number(2.0),
            )]),
        )
        .expect("unrelated edit");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("no-dirty spill rebase");
    let snapshot = snapshot_work_counters();
    assert_eq!(
        snapshot.get(WorkCounter::ResultStorePayloadBytesDeepCloned),
        0
    );
    assert_eq!(snapshot.get(WorkCounter::ResultStoreLeavesRebuilt), 0);
}
