use std::hint::black_box;
use std::time::{Duration, Instant};

use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
};

const DEFAULT_FORMULAS: u32 = 10_000;
const DEFAULT_BATCH_CELLS: u32 = 10_000;

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let test_mode = cfg!(test) || arguments.iter().any(|argument| argument == "--test");
    let formulas = numeric_argument(&arguments, 0, if test_mode { 20 } else { DEFAULT_FORMULAS });
    let batch_cells = numeric_argument(
        &arguments,
        1,
        if test_mode { 100 } else { DEFAULT_BATCH_CELLS },
    );
    assert!(formulas > 0, "formula count must be greater than zero");
    assert!(batch_cells > 0, "batch size must be greater than zero");

    let mut session = chain_session(formulas);
    let cold_started = Instant::now();
    let cold = recalculate(&mut session);
    let cold_elapsed = cold_started.elapsed();

    let warm_started = Instant::now();
    let warm = recalculate(&mut session);
    let warm_elapsed = warm_started.elapsed();

    let unrelated_started = Instant::now();
    apply_one(
        &mut session,
        WorkbookChange::set_cell_value(sheet_id(), address("C1"), number(1.0)),
    );
    let unrelated = recalculate(&mut session);
    let unrelated_elapsed = unrelated_started.elapsed();

    let chain_started = Instant::now();
    apply_one(
        &mut session,
        WorkbookChange::set_cell_value(sheet_id(), address("A1"), number(2.0)),
    );
    let chain = recalculate(&mut session);
    let chain_elapsed = chain_started.elapsed();

    let mut wide_session = wide_session(formulas);
    recalculate(&mut wide_session);
    let wide_started = Instant::now();
    apply_one(
        &mut wide_session,
        WorkbookChange::set_cell_value(sheet_id(), address("A1"), number(2.0)),
    );
    let wide = recalculate(&mut wide_session);
    let wide_elapsed = wide_started.elapsed();

    let topology_started = Instant::now();
    apply_one(
        &mut session,
        WorkbookChange::set_cell_formula(sheet_id(), address("B1"), formula("A1+2")),
    );
    let topology = recalculate(&mut session);
    let topology_elapsed = topology_started.elapsed();

    let batch_started = Instant::now();
    let mut batch_session = WorkbookCalculationSession::create();
    let batch = generated_values(batch_cells);
    black_box(
        batch_session
            .apply_changes(0, EditBatch::new(batch))
            .expect("generated benchmark batch"),
    );
    let batch_elapsed = batch_started.elapsed();

    let repeated_started = Instant::now();
    let mut repeated_session = WorkbookCalculationSession::create();
    for change in generated_values(batch_cells) {
        apply_one(&mut repeated_session, change);
    }
    let repeated_elapsed = repeated_started.elapsed();

    println!("cellrune_interactive_benchmark_v1");
    println!("formulas\t{formulas}");
    println!("batch_cells\t{batch_cells}");
    metric("cold_full_ms", cold_elapsed);
    println!("cold_evaluated\t{}", cold.evaluated_count());
    println!("cold_reason\t{:?}", cold.reason());
    metric("warm_ms", warm_elapsed);
    println!("warm_evaluated\t{}", warm.evaluated_count());
    println!("warm_reason\t{:?}", warm.reason());
    metric("unrelated_literal_ms", unrelated_elapsed);
    println!("unrelated_evaluated\t{}", unrelated.evaluated_count());
    println!("unrelated_reason\t{:?}", unrelated.reason());
    metric("chain_incremental_ms", chain_elapsed);
    println!("chain_evaluated\t{}", chain.evaluated_count());
    println!("chain_reason\t{:?}", chain.reason());
    metric("wide_incremental_ms", wide_elapsed);
    println!("wide_evaluated\t{}", wide.evaluated_count());
    println!("wide_reason\t{:?}", wide.reason());
    metric("topology_full_ms", topology_elapsed);
    println!("topology_evaluated\t{}", topology.evaluated_count());
    println!("topology_reason\t{:?}", topology.reason());
    metric("batch_edit_ms", batch_elapsed);
    metric("repeated_edit_ms", repeated_elapsed);
    println!(
        "batch_speedup_ratio\t{:.3}",
        repeated_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64()
    );
}

fn wide_session(formulas: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(formulas as usize + 2);
    changes.push(WorkbookChange::set_cell_value(
        sheet_id(),
        address("A1"),
        number(1.0),
    ));
    for row in 1..=formulas {
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id(),
            CellAddress::from_indices(row, 2).expect("valid generated wide formula cell"),
            formula(&format!("A1+{row}")),
        ));
    }
    changes.push(WorkbookChange::set_cell_formula(
        sheet_id(),
        address("D1"),
        formula("1+1"),
    ));
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated wide workbook");
    session
}

fn chain_session(formulas: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(formulas as usize + 2);
    changes.push(WorkbookChange::set_cell_value(
        sheet_id(),
        address("A1"),
        number(1.0),
    ));
    for row in 1..=formulas {
        let dependency = if row == 1 {
            "A1".to_owned()
        } else {
            format!("B{}", row - 1)
        };
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id(),
            CellAddress::from_indices(row, 2).expect("valid generated formula cell"),
            formula(&format!("{dependency}+1")),
        ));
    }
    changes.push(WorkbookChange::set_cell_formula(
        sheet_id(),
        address("D1"),
        formula("1+1"),
    ));
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated benchmark workbook");
    session
}

fn generated_values(cells: u32) -> Vec<WorkbookChange> {
    (1..=cells)
        .map(|row| {
            WorkbookChange::set_cell_value(
                sheet_id(),
                CellAddress::from_indices(row, 1).expect("valid generated batch cell"),
                number(f64::from(row)),
            )
        })
        .collect()
}

fn apply_one(session: &mut WorkbookCalculationSession, change: WorkbookChange) {
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([change]),
        )
        .expect("generated single edit");
}

fn recalculate(session: &mut WorkbookCalculationSession) -> cellrune::CalculationDelta {
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("generated benchmark calculation")
}

fn numeric_argument(arguments: &[String], index: usize, default: u32) -> u32 {
    arguments
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .nth(index)
        .map(|argument| {
            argument
                .parse::<u32>()
                .expect("benchmark arguments must be u32")
        })
        .unwrap_or(default)
}

fn metric(name: &str, duration: Duration) {
    println!("{name}\t{:.3}", duration.as_secs_f64() * 1_000.0);
}

fn sheet_id() -> SheetId {
    SheetId::new(1).expect("constant sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("constant address")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("generated formula")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite generated number"))
}
