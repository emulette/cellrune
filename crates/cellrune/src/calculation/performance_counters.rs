//! Always-compiled performance-axis work counters for 0.1.15.
//!
//! The 0.1.14 counters in [`super::work_counter`] are test-gated and thread-local. These counters
//! are global relaxed atomics and are compiled into every build so the deterministic adversarial
//! suite and the phase-isolated benches can reset and read them from outside the crate through the
//! hidden `cellrune::testing` module. They separate structural copies from actual payload deep
//! clones and cover area-index, impact-preparation, and fingerprint-hashing work.

// The types and functions here are re-exported through the hidden `cellrune::testing` module for
// integration tests and benches; they are intentionally exempt from the crate's public-API
// documentation requirement.
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum WorkCounter {
    CellStoreNodesCopied,
    CellStoreLeavesRebuilt,
    CellStoreEntriesReindexed,
    CellStorePayloadBytesDeepCloned,
    ResultStoreNodesCopied,
    ResultStoreLeavesRebuilt,
    ResultStoreEntriesReindexed,
    ResultStorePayloadBytesDeepCloned,
    AreaSourceRectangles,
    AreaPayloadRefsRetained,
    AreaNodesRetained,
    AreaBuildPayloadVisits,
    AreaQueryNodesVisited,
    AreaQueryCandidatesExamined,
    AreaQueryMatchesEmitted,
    ImpactChangedCellsVisited,
    ImpactDirectCandidatesVisited,
    ImpactReverseEdgesVisited,
    ImpactUniqueDirtyInserted,
    ImpactCancellationPolls,
    FingerprintPayloadLeavesHashed,
    FingerprintInternalNodesHashed,
    FingerprintCachedNodesReused,
    FingerprintRootCacheHits,
}

const COUNTER_COUNT: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkCounterSnapshot {
    values: [u64; COUNTER_COUNT],
}

impl WorkCounterSnapshot {
    pub const fn get(self, counter: WorkCounter) -> u64 {
        self.values[counter as usize]
    }
}

struct GlobalWorkCounters {
    values: [AtomicU64; COUNTER_COUNT],
}

static GLOBAL_WORK_COUNTERS: GlobalWorkCounters = GlobalWorkCounters::new();
static COUNTERS_ACTIVE: AtomicBool = AtomicBool::new(false);
static COUNTER_TEST_LOCK: Mutex<()> = Mutex::new(());

impl GlobalWorkCounters {
    const fn new() -> Self {
        #[allow(clippy::declare_interior_mutable_const)]
        const ZERO: AtomicU64 = AtomicU64::new(0);
        Self {
            values: [ZERO; COUNTER_COUNT],
        }
    }

    fn add(&self, counter: WorkCounter, delta: u64) {
        self.values[counter as usize].fetch_add(delta, Ordering::Relaxed);
    }

    fn store(&self, counter: WorkCounter, value: u64) {
        self.values[counter as usize].store(value, Ordering::Relaxed);
    }

    fn load(&self, counter: WorkCounter) -> u64 {
        self.values[counter as usize].load(Ordering::Relaxed)
    }
}

// Forward-declared mutation API wired up by the O1-O4 storage/index/impact/identity integrations.
#[allow(dead_code)]
pub(crate) fn work_counter_add(counter: WorkCounter, delta: u64) {
    if COUNTERS_ACTIVE.load(Ordering::Relaxed) {
        GLOBAL_WORK_COUNTERS.add(counter, delta);
    }
}

#[allow(dead_code)]
pub(crate) fn work_counter_store(counter: WorkCounter, value: u64) {
    if COUNTERS_ACTIVE.load(Ordering::Relaxed) {
        GLOBAL_WORK_COUNTERS.store(counter, value);
    }
}

pub fn lock_work_counters() -> MutexGuard<'static, ()> {
    COUNTER_TEST_LOCK
        .lock()
        .expect("work-counter lock poisoned")
}

pub fn reset_work_counters() {
    COUNTERS_ACTIVE.store(true, Ordering::Relaxed);
    for counter in ALL_COUNTERS {
        GLOBAL_WORK_COUNTERS.store(counter, 0);
    }
}

pub fn snapshot_work_counters() -> WorkCounterSnapshot {
    let mut values = [0_u64; COUNTER_COUNT];
    for counter in ALL_COUNTERS {
        values[counter as usize] = GLOBAL_WORK_COUNTERS.load(counter);
    }
    COUNTERS_ACTIVE.store(false, Ordering::Relaxed);
    WorkCounterSnapshot { values }
}

const ALL_COUNTERS: [WorkCounter; COUNTER_COUNT] = [
    WorkCounter::CellStoreNodesCopied,
    WorkCounter::CellStoreLeavesRebuilt,
    WorkCounter::CellStoreEntriesReindexed,
    WorkCounter::CellStorePayloadBytesDeepCloned,
    WorkCounter::ResultStoreNodesCopied,
    WorkCounter::ResultStoreLeavesRebuilt,
    WorkCounter::ResultStoreEntriesReindexed,
    WorkCounter::ResultStorePayloadBytesDeepCloned,
    WorkCounter::AreaSourceRectangles,
    WorkCounter::AreaPayloadRefsRetained,
    WorkCounter::AreaNodesRetained,
    WorkCounter::AreaBuildPayloadVisits,
    WorkCounter::AreaQueryNodesVisited,
    WorkCounter::AreaQueryCandidatesExamined,
    WorkCounter::AreaQueryMatchesEmitted,
    WorkCounter::ImpactChangedCellsVisited,
    WorkCounter::ImpactDirectCandidatesVisited,
    WorkCounter::ImpactReverseEdgesVisited,
    WorkCounter::ImpactUniqueDirtyInserted,
    WorkCounter::ImpactCancellationPolls,
    WorkCounter::FingerprintPayloadLeavesHashed,
    WorkCounter::FingerprintInternalNodesHashed,
    WorkCounter::FingerprintCachedNodesReused,
    WorkCounter::FingerprintRootCacheHits,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_reset_snapshot_and_increment_independently() {
        let _guard = lock_work_counters();
        reset_work_counters();
        work_counter_add(WorkCounter::CellStoreNodesCopied, 3);
        work_counter_add(WorkCounter::FingerprintRootCacheHits, 1);
        let snapshot = snapshot_work_counters();
        assert_eq!(snapshot.get(WorkCounter::CellStoreNodesCopied), 3);
        assert_eq!(snapshot.get(WorkCounter::FingerprintRootCacheHits), 1);
        assert_eq!(snapshot.get(WorkCounter::ResultStoreNodesCopied), 0);

        reset_work_counters();
        work_counter_store(WorkCounter::AreaSourceRectangles, 7);
        assert_eq!(
            snapshot_work_counters().get(WorkCounter::AreaSourceRectangles),
            7
        );
        reset_work_counters();
        assert_eq!(
            snapshot_work_counters().get(WorkCounter::CellStoreNodesCopied),
            0
        );
    }
}
