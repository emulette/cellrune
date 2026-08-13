//! Tracking-allocator child for the manual 0.1.15 phase-memory benchmark.

use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
};
use serde::Serialize;
use system_alloc_stats::SystemWithStats;

#[global_allocator]
static ALLOCATOR: SystemWithStats = SystemWithStats;

#[derive(Serialize)]
struct DirtyMemorySample {
    peak_live_heap_bytes: usize,
    end_live_heap_bytes: usize,
    total_allocated_bytes: usize,
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(
        arguments
            .first()
            .is_some_and(|value| value == "--dirty-memory-child"),
        "the heap helper only supports --dirty-memory-child"
    );
    let formulas = value_after(&arguments, "--formulas")
        .expect("--formulas is required")
        .parse()
        .expect("numeric formula count");
    println!(
        "{}",
        serde_json::to_string(&measure(formulas)).expect("serialize dirty memory sample")
    );
}

fn measure(formulas: u32) -> DirtyMemorySample {
    let mut session = independent_session(formulas);
    recalculate(&mut session, RecalculationMode::Full);
    ALLOCATOR.reset();
    let initial_live = ALLOCATOR.use_curr();
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet(),
                address(1, 1),
                number(2.0),
            )]),
        )
        .expect("single dirty edit");
    recalculate(&mut session, RecalculationMode::Auto);
    let stats = ALLOCATOR.stats();
    DirtyMemorySample {
        peak_live_heap_bytes: stats.use_max.saturating_sub(initial_live),
        end_live_heap_bytes: stats.use_curr.saturating_sub(initial_live),
        total_allocated_bytes: ALLOCATOR
            .alloc_sum()
            .saturating_add(ALLOCATOR.realloc_growth_sum()),
    }
}

fn independent_session(formulas: u32) -> WorkbookCalculationSession {
    let mut changes = Vec::with_capacity(formulas as usize * 2);
    for row in 1..=formulas {
        changes.push(WorkbookChange::set_cell_value(
            sheet(),
            address(row, 1),
            number(1.0),
        ));
        changes.push(WorkbookChange::set_cell_formula(
            sheet(),
            address(row, 2),
            FormulaText::from_xlsx(format!("A{row}+1")).expect("generated formula"),
        ));
    }
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("independent workload");
    session
}

fn recalculate(session: &mut WorkbookCalculationSession, mode: RecalculationMode) {
    session
        .recalculate(
            mode,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("benchmark calculation");
}

fn value_after<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

fn sheet() -> SheetId {
    SheetId::new(1).expect("default sheet")
}

fn address(row: u32, column: u32) -> CellAddress {
    CellAddress::from_indices(row, column).expect("benchmark address")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite benchmark number"))
}
