use std::cell::Cell;

thread_local! {
    static DEEP_CLONED_CELLS: Cell<usize> = const { Cell::new(0) };
    static DEEP_CLONED_ASTS: Cell<usize> = const { Cell::new(0) };
    static DEEP_CLONED_RESULTS: Cell<usize> = const { Cell::new(0) };
    static DEPENDENCY_TARGET_SCANS: Cell<usize> = const { Cell::new(0) };
    static SCHEDULE_BUILDS: Cell<usize> = const { Cell::new(0) };
    static SCHEDULE_VISITS: Cell<usize> = const { Cell::new(0) };
    static FORMULA_SNAPSHOT_SCANS: Cell<usize> = const { Cell::new(0) };
    static AREA_DEPENDENCY_VISITS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorkCounters {
    pub(super) deep_cloned_cells: usize,
    pub(super) deep_cloned_asts: usize,
    pub(super) deep_cloned_results: usize,
    pub(super) dependency_target_scans: usize,
    pub(super) schedule_builds: usize,
    pub(super) schedule_visits: usize,
    pub(super) formula_snapshot_scans: usize,
    pub(super) area_dependency_visits: usize,
}

pub(super) fn reset() {
    DEEP_CLONED_CELLS.set(0);
    DEEP_CLONED_ASTS.set(0);
    DEEP_CLONED_RESULTS.set(0);
    DEPENDENCY_TARGET_SCANS.set(0);
    SCHEDULE_BUILDS.set(0);
    SCHEDULE_VISITS.set(0);
    FORMULA_SNAPSHOT_SCANS.set(0);
    AREA_DEPENDENCY_VISITS.set(0);
}

pub(super) fn snapshot() -> WorkCounters {
    WorkCounters {
        deep_cloned_cells: DEEP_CLONED_CELLS.get(),
        deep_cloned_asts: DEEP_CLONED_ASTS.get(),
        deep_cloned_results: DEEP_CLONED_RESULTS.get(),
        dependency_target_scans: DEPENDENCY_TARGET_SCANS.get(),
        schedule_builds: SCHEDULE_BUILDS.get(),
        schedule_visits: SCHEDULE_VISITS.get(),
        formula_snapshot_scans: FORMULA_SNAPSHOT_SCANS.get(),
        area_dependency_visits: AREA_DEPENDENCY_VISITS.get(),
    }
}

pub(crate) fn deep_cloned_asts(count: usize) {
    DEEP_CLONED_ASTS.set(DEEP_CLONED_ASTS.get() + count);
}

pub(super) fn dependency_target_scan() {
    DEPENDENCY_TARGET_SCANS.set(DEPENDENCY_TARGET_SCANS.get() + 1);
}

pub(super) fn schedule_build() {
    SCHEDULE_BUILDS.set(SCHEDULE_BUILDS.get() + 1);
}

pub(super) fn schedule_visit() {
    SCHEDULE_VISITS.set(SCHEDULE_VISITS.get() + 1);
}

pub(super) fn formula_snapshot_scan() {
    FORMULA_SNAPSHOT_SCANS.set(FORMULA_SNAPSHOT_SCANS.get() + 1);
}

pub(super) fn area_dependency_visit() {
    AREA_DEPENDENCY_VISITS.set(AREA_DEPENDENCY_VISITS.get() + 1);
}
