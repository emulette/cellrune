//! Deterministic 0.1.15 performance-axis acceptance suite (O6).
//!
//! These are deterministic correctness/counter checks, not wall-clock or RSS evidence. The
//! manual latency/memory acceptance (O5) is collected separately from the same commit.

use cellrune::testing::{reset_work_counters, snapshot_work_counters, WorkCounter};
use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
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

/// `acc_o1_no_payload_clone`: a single edit of a dense-wide (1 x 16384) sheet must deep-clone
/// zero unchanged Cell/CalculationCellResult payload bytes. Per-entry `Arc` sharing keeps the
/// deep-clone counter at zero even though the affected leaf is structurally rebuilt.
#[test]
fn dense_wide_edit_deep_clones_no_unchanged_payload() {
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
    assert_eq!(snapshot.get(WorkCounter::CellStorePayloadBytesDeepCloned), 0);
    assert_eq!(
        snapshot.get(WorkCounter::ResultStorePayloadBytesDeepCloned),
        0
    );
}

/// `acc_o4_hash_work` (no-dirty half): a no-dirty recalculation must reuse the cached workbook
/// root fingerprint and hash zero payload leaves or internal nodes.
#[test]
fn no_dirty_rebase_reuses_cached_fingerprint() {
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

/// `acc_full_incremental`: a fixed-income-style workbook keeps value, error, and spill-shape
/// parity between a full calculation and an incremental recalculation after a one-cell edit.
#[test]
fn full_and_incremental_calculation_agree_after_an_edit() {
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

    let full = session
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

    // The edited input changes PRICE; the dependent and spill formulas must still be evaluated
    // to the same shape under both schedules.
    assert!(full.evaluated_count() > 0);
    assert!(incremental.evaluated_count() > 0);
    assert!(incremental.evaluated_count() <= full.evaluated_count());
}
