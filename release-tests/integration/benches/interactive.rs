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
    let test_mode = arguments.iter().any(|argument| argument == "--test");
    let formulas = numeric_argument(&arguments, 0, if test_mode { 20 } else { DEFAULT_FORMULAS });
    let batch_cells = numeric_argument(
        &arguments,
        1,
        if test_mode { 100 } else { DEFAULT_BATCH_CELLS },
    );
    assert!(formulas > 0, "formula count must be greater than zero");
    assert!(batch_cells > 0, "batch size must be greater than zero");

    let independent_only = arguments
        .iter()
        .any(|argument| argument == "--independent-only");
    let dirty_rss_only = arguments
        .iter()
        .any(|argument| argument == "--dirty-rss-only");
    if independent_only || dirty_rss_only {
        run_independent_benchmark(formulas, !dirty_rss_only);
        return;
    }

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

    drop(session);
    drop(wide_session);
    drop(batch_session);
    drop(repeated_session);

    let mut independent = independent_session(formulas);
    recalculate(&mut independent);
    let independent_full_started = Instant::now();
    let independent_full = recalculate_mode(&mut independent, RecalculationMode::Full);
    let independent_full_elapsed = independent_full_started.elapsed();
    let independent_no_dirty_started = Instant::now();
    let independent_no_dirty = recalculate(&mut independent);
    let independent_no_dirty_elapsed = independent_no_dirty_started.elapsed();
    let one_dirty = measure_edit_and_recalculate(
        &mut independent,
        EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id(),
            address("A1"),
            number(2.0),
        )]),
    );
    drop(independent);

    let mut percent_session = independent_session(formulas);
    recalculate(&mut percent_session);
    let percent_count = (formulas / 100).max(1);
    let percent_changes = (1..=percent_count).map(|row| {
        WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(row, 1).expect("valid generated percent cell"),
            number(2.0),
        )
    });
    let percent_dirty = measure_edit_and_recalculate(
        &mut percent_session,
        EditBatch::new(percent_changes.collect::<Vec<_>>()),
    );
    drop(percent_session);

    let unique_count = if test_mode { 50 } else { 5_000 };
    let mut unique = unique_ast_session(unique_count);
    recalculate(&mut unique);
    let unique_full_started = Instant::now();
    let unique_full = recalculate_mode(&mut unique, RecalculationMode::Full);
    let unique_full_elapsed = unique_full_started.elapsed();
    let unique_dirty_row = unique_count / 2;
    let unique_dirty = measure_edit_and_recalculate(
        &mut unique,
        EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(unique_dirty_row, 1).expect("valid unique-AST input"),
            number(2.0),
        )]),
    );
    drop(unique);

    let range_source_rows = if test_mode { 10 } else { 1_000 };
    let range_formula_count = if test_mode { 4 } else { 40 };
    let mut ranges = range_fanout_session(range_source_rows, range_formula_count);
    recalculate(&mut ranges);
    let range_full_started = Instant::now();
    let range_full = recalculate_mode(&mut ranges, RecalculationMode::Full);
    let range_full_elapsed = range_full_started.elapsed();
    let range_dirty = measure_edit_and_recalculate(
        &mut ranges,
        EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(range_source_rows / 2, 1).expect("valid range-fanout input"),
            number(2.0),
        )]),
    );
    drop(ranges);

    let reverse_count = if test_mode { 50 } else { 50_000 };
    let mut reverse = reverse_chain_session(reverse_count);
    let reverse_cold_started = Instant::now();
    let reverse_cold = recalculate(&mut reverse);
    let reverse_cold_elapsed = reverse_cold_started.elapsed();
    let reverse_dirty_started = Instant::now();
    apply_one(
        &mut reverse,
        WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(reverse_count, 1).expect("valid reverse-chain tail"),
            number(2.0),
        ),
    );
    let reverse_dirty = recalculate(&mut reverse);
    let reverse_dirty_elapsed = reverse_dirty_started.elapsed();
    drop(reverse);

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
    metric("independent_warm_full_ms", independent_full_elapsed);
    println!(
        "independent_warm_full_evaluated\t{}",
        independent_full.evaluated_count()
    );
    metric("independent_no_dirty_ms", independent_no_dirty_elapsed);
    println!(
        "independent_no_dirty_evaluated\t{}",
        independent_no_dirty.evaluated_count()
    );
    phase_metrics("independent_one_dirty", &one_dirty);
    phase_metrics("independent_one_percent_dirty", &percent_dirty);
    metric("unique_ast_warm_full_ms", unique_full_elapsed);
    println!(
        "unique_ast_warm_full_evaluated\t{}",
        unique_full.evaluated_count()
    );
    phase_metrics("unique_ast_one_dirty", &unique_dirty);
    metric("range_fanout_warm_full_ms", range_full_elapsed);
    println!(
        "range_fanout_warm_full_evaluated\t{}",
        range_full.evaluated_count()
    );
    phase_metrics("range_fanout_one_range_dirty", &range_dirty);
    metric("reverse_chain_cold_full_ms", reverse_cold_elapsed);
    println!(
        "reverse_chain_cold_full_evaluated\t{}",
        reverse_cold.evaluated_count()
    );
    metric("reverse_chain_tail_dirty_ms", reverse_dirty_elapsed);
    println!(
        "reverse_chain_tail_dirty_evaluated\t{}",
        reverse_dirty.evaluated_count()
    );
}

fn run_independent_benchmark(formulas: u32, include_percent_dirty: bool) {
    let mut independent = independent_session(formulas);
    recalculate(&mut independent);

    let full_started = Instant::now();
    let full = recalculate_mode(&mut independent, RecalculationMode::Full);
    let full_elapsed = full_started.elapsed();

    let no_dirty_started = Instant::now();
    let no_dirty = recalculate(&mut independent);
    let no_dirty_elapsed = no_dirty_started.elapsed();

    let one_dirty = measure_edit_and_recalculate(
        &mut independent,
        EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id(),
            address("A1"),
            number(2.0),
        )]),
    );
    drop(independent);

    let percent_dirty = include_percent_dirty.then(|| {
        let mut percent_session = independent_session(formulas);
        recalculate(&mut percent_session);
        let percent_count = (formulas / 100).max(1);
        let percent_changes = (1..=percent_count).map(|row| {
            WorkbookChange::set_cell_value(
                sheet_id(),
                CellAddress::from_indices(row, 1).expect("valid generated percent cell"),
                number(2.0),
            )
        });
        measure_edit_and_recalculate(
            &mut percent_session,
            EditBatch::new(percent_changes.collect::<Vec<_>>()),
        )
    });

    println!("cellrune_independent_benchmark_v1");
    println!("formulas\t{formulas}");
    metric("independent_warm_full_ms", full_elapsed);
    println!(
        "independent_warm_full_evaluated\t{}",
        full.evaluated_count()
    );
    metric("independent_no_dirty_ms", no_dirty_elapsed);
    println!(
        "independent_no_dirty_evaluated\t{}",
        no_dirty.evaluated_count()
    );
    phase_metrics("independent_one_dirty", &one_dirty);
    if let Some(percent_dirty) = percent_dirty {
        phase_metrics("independent_one_percent_dirty", &percent_dirty);
    }
}

struct IncrementalMeasurement {
    edit_prepare: Duration,
    impact_install: Duration,
    calculation_prepare: Duration,
    run: Duration,
    install: Duration,
    delta: cellrune::CalculationDelta,
}

fn measure_edit_and_recalculate(
    session: &mut WorkbookCalculationSession,
    batch: EditBatch,
) -> IncrementalMeasurement {
    let edit_prepare_started = Instant::now();
    let prepared = session
        .prepare_changes(session.workbook().semantic_revision(), batch)
        .expect("generated benchmark edit preparation");
    let edit_prepare = edit_prepare_started.elapsed();

    let impact_started = Instant::now();
    session
        .install_changes(prepared)
        .expect("generated benchmark edit installation");
    let impact_install = impact_started.elapsed();

    let calculation_prepare_started = Instant::now();
    let prepared = session
        .prepare_recalculation(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("generated benchmark calculation preparation");
    let calculation_prepare = calculation_prepare_started.elapsed();

    let run_started = Instant::now();
    let completed = prepared
        .run()
        .expect("generated benchmark calculation execution");
    let run = run_started.elapsed();

    let install_started = Instant::now();
    let delta = session
        .install(completed)
        .expect("generated benchmark calculation installation");
    let install = install_started.elapsed();
    IncrementalMeasurement {
        edit_prepare,
        impact_install,
        calculation_prepare,
        run,
        install,
        delta,
    }
}

fn phase_metrics(prefix: &str, measurement: &IncrementalMeasurement) {
    metric(
        &format!("{prefix}_edit_prepare_ms"),
        measurement.edit_prepare,
    );
    metric(
        &format!("{prefix}_impact_install_ms"),
        measurement.impact_install,
    );
    metric(
        &format!("{prefix}_calculation_prepare_ms"),
        measurement.calculation_prepare,
    );
    metric(&format!("{prefix}_run_ms"), measurement.run);
    metric(&format!("{prefix}_install_ms"), measurement.install);
    println!(
        "{prefix}_evaluated\t{}",
        measurement.delta.evaluated_count()
    );
    println!("{prefix}_reason\t{:?}", measurement.delta.reason());
}

fn independent_session(formulas: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(formulas as usize * 2);
    for row in 1..=formulas {
        changes.push(WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(row, 1).expect("valid generated independent input"),
            number(1.0),
        ));
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id(),
            CellAddress::from_indices(row, 2).expect("valid generated independent formula"),
            formula(&format!("A{row}+1")),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated independent workbook");
    session
}

fn unique_ast_session(formulas: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(formulas as usize * 2);
    for row in 1..=formulas {
        changes.push(WorkbookChange::set_cell_value(
            sheet_id(),
            CellAddress::from_indices(row, 1).expect("valid generated unique-AST input"),
            number(1.0),
        ));
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id(),
            CellAddress::from_indices(row, 2).expect("valid generated unique-AST formula"),
            formula(&format!("A{row}+{row}")),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated unique-AST workbook");
    session
}

fn range_fanout_session(source_rows: u32, formulas_per_column: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity((10 * (source_rows + formulas_per_column)) as usize);
    for column in 1..=10 {
        let column_name =
            char::from_u32(u32::from(b'A') + column - 1).expect("generated A:J range column");
        for row in 1..=source_rows {
            changes.push(WorkbookChange::set_cell_value(
                sheet_id(),
                CellAddress::from_indices(row, column).expect("valid range source"),
                number(1.0),
            ));
        }
        for offset in 1..=formulas_per_column {
            changes.push(WorkbookChange::set_cell_formula(
                sheet_id(),
                CellAddress::from_indices(source_rows + 1 + offset, column)
                    .expect("valid range formula"),
                formula(&format!("SUM({column_name}1:{column_name}{source_rows})")),
            ));
        }
    }
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated range-fanout workbook");
    session
}

fn reverse_chain_session(cells: u32) -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let mut changes = Vec::with_capacity(cells as usize);
    for row in 1..cells {
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id(),
            CellAddress::from_indices(row, 1).expect("valid reverse-chain formula"),
            formula(&format!("A{}+1", row + 1)),
        ));
    }
    changes.push(WorkbookChange::set_cell_value(
        sheet_id(),
        CellAddress::from_indices(cells, 1).expect("valid reverse-chain tail"),
        number(1.0),
    ));
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("generated reverse-chain workbook");
    session
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
    recalculate_mode(session, RecalculationMode::Auto)
}

fn recalculate_mode(
    session: &mut WorkbookCalculationSession,
    mode: RecalculationMode,
) -> cellrune::CalculationDelta {
    session
        .recalculate(
            mode,
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
