use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CalculationTarget,
    CancellationToken, CellAddress, CellValue, EditBatch, FormulaText, RecalculationMode, SheetId,
    TargetCalculationLimits, WorkbookCalculationSession, WorkbookChange, WorkbookDraft,
};
use std::time::Instant;

const ROWS: u32 = 10_000;

fn id(row: u32, column: u32) -> CalculationCellId {
    CalculationCellId::new(
        SheetId::new(1).expect("sheet"),
        CellAddress::from_indices(row, column).expect("address"),
    )
}

fn workbook() -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    draft
        .apply_changes(EditBatch::new((1..=ROWS).flat_map(|row| {
            [
                WorkbookChange::set_cell_value(
                    id(row, 1).sheet_id(),
                    id(row, 1).address(),
                    CellValue::number(f64::from(row)).expect("number"),
                ),
                WorkbookChange::set_cell_formula(
                    id(row, 2).sheet_id(),
                    id(row, 2).address(),
                    FormulaText::from_xlsx(format!("A{row}+1")).expect("formula"),
                ),
                WorkbookChange::set_cell_formula(
                    id(row, 3).sheet_id(),
                    id(row, 3).address(),
                    FormulaText::from_xlsx(format!("B{row}*2")).expect("formula"),
                ),
            ]
        })))
        .expect("workload");
    draft
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let options = CalculationOptions::default();
    let limits = TargetCalculationLimits::default();
    let samples = if std::env::args().any(|arg| arg == "--test") {
        1
    } else {
        5
    };
    for requested in [1, 100, 10_000] {
        let mut full_times = Vec::new();
        let mut partial_times = Vec::new();
        let mut repeated_times = Vec::new();
        let mut cached_times = Vec::new();
        let targets = [CalculationTarget::new(
            id(1, 3).sheet_id(),
            cellrune::CellRange::new(id(1, 3).address(), id(requested, 3).address())
                .expect("output range"),
        )];
        let mut evaluated = 0;
        for _ in 0..samples {
            // Independently constructed snapshots include initial identity preparation on both paths.
            let partial = WorkbookCalculationSession::new(workbook());
            let mut full = WorkbookCalculationSession::new(workbook());
            let started = Instant::now();
            let result = partial
                .calculate_targets(&targets, options, limits, CancellationToken::new())
                .expect("first request");
            partial_times.push(started.elapsed().as_secs_f64() * 1_000.0);
            evaluated = result.evaluated_count();
            let started = Instant::now();
            let repeated = partial
                .calculate_targets(&targets, options, limits, CancellationToken::new())
                .expect("repeated request without a full cache");
            repeated_times.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(repeated.evaluated_count(), result.evaluated_count());
            assert_eq!(
                repeated.parsed_formula_count(),
                result.parsed_formula_count()
            );
            assert_eq!(
                repeated.cells().collect::<Vec<_>>(),
                result.cells().collect::<Vec<_>>()
            );
            let started = Instant::now();
            full.recalculate(RecalculationMode::Full, options, CancellationToken::new())
                .expect("full calculation");
            full_times.push(started.elapsed().as_secs_f64() * 1_000.0);
            for (cell, value) in result.cells() {
                assert_eq!(
                    Some(value),
                    full.calculation().expect("complete").cell(cell)
                );
                assert!(matches!(
                    value,
                    CalculationCellResult::Value(CellValue::Number(_))
                ));
            }
            let started = Instant::now();
            let cached = full
                .calculate_targets(&targets, options, limits, CancellationToken::new())
                .expect("current cache");
            cached_times.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(cached.evaluated_count(), 0);
            assert_eq!(cached.reused_count(), requested as usize);
        }
        println!(
            "formulas={} targets={requested} samples={samples} full_ms={:.3} first_partial_ms={:.3} repeated_partial_ms={:.3} cached_partial_ms={:.3} partial_evaluations={evaluated}",
            ROWS * 2,
            median(full_times),
            median(partial_times),
            median(repeated_times),
            median(cached_times)
        );
    }
}
