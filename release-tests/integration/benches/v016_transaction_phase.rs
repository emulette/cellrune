//! Manual 0.1.16 transaction phase latency and retained-memory evidence.
//!
//! Run the release-profile smoke check with:
//! `cargo bench -p cellrune-integration-tests --bench v016_transaction_phase -- --smoke`.
//! Run the recorded workload with:
//! `cargo bench -p cellrune-integration-tests --bench v016_transaction_phase -- --output evidence.json`.
//! Optional `--formulas` and `--samples` overrides are intended only to validate the harness. The
//! default measurement uses 10,000 formulas, one warmup, and ten recorded child-process samples.
//! Measurements are descriptive only: this target has no threshold and never gates CI.

use std::fs;
use std::hint::black_box;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cellrune::{
    CalculationDecisionReason, CalculationExecutionMode, CalculationOptions, CancellationToken,
    RecalculationMode, TransactionDetailSection,
};
use cellrune_interop::TransactionDetailSectionDto;

#[path = "v016_transaction_phase/model.rs"]
mod model;
#[path = "support/platform.rs"]
mod platform;
#[path = "v016_transaction_phase/workload.rs"]
mod workload;

use model::{Evidence, PhaseSample, RetainedMemorySample, ScenarioEvidence};
use workload::Scenario;

const DEFAULT_FORMULAS: u32 = 10_000;
const DEFAULT_SAMPLES: usize = 10;
const SMOKE_FORMULAS: u32 = 100;
const PAGE_ITEMS: u32 = 1_000;

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|value| value == "--phase-child")
    {
        let scenario = child_scenario(&arguments);
        let formulas = child_formulas(&arguments);
        let sample = measure_phase_sample(scenario, formulas);
        println!(
            "{}",
            serde_json::to_string(&sample).expect("serialize phase sample")
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--retained-child")
    {
        retained_child(child_scenario(&arguments), child_formulas(&arguments));
        return;
    }

    let smoke = arguments.iter().any(|value| value == "--smoke");
    let formulas = value_after(&arguments, "--formulas")
        .map(|value| value.parse().expect("numeric formula count"))
        .unwrap_or(if smoke {
            SMOKE_FORMULAS
        } else {
            DEFAULT_FORMULAS
        });
    let recorded_samples = value_after(&arguments, "--samples")
        .map(|value| value.parse().expect("numeric sample count"))
        .unwrap_or(if smoke { 1 } else { DEFAULT_SAMPLES });
    assert!(formulas >= 2, "the workload requires at least two formulas");
    assert!(recorded_samples > 0, "at least one sample is required");

    let executable = std::env::current_exe().expect("resolve benchmark executable");
    let mut scenarios = Vec::with_capacity(Scenario::ALL.len());
    for scenario in Scenario::ALL {
        let _warmup_phase = run_phase_child(&executable, scenario, formulas);
        let _warmup_memory = measure_retained_child(&executable, scenario, formulas);
        let mut phase_samples = Vec::with_capacity(recorded_samples);
        let mut memory_samples = Vec::with_capacity(recorded_samples);
        for _ in 0..recorded_samples {
            phase_samples.push(run_phase_child(&executable, scenario, formulas));
            memory_samples.push(measure_retained_child(&executable, scenario, formulas));
        }
        scenarios.push(ScenarioEvidence::new(
            scenario.as_str().to_owned(),
            phase_samples,
            memory_samples,
        ));
    }

    let evidence = Evidence {
        schema: "cellrune_0_1_16_transaction_phase_v1".to_owned(),
        mode: if smoke { "smoke" } else { "measurement" }.to_owned(),
        commit: platform::command_output("git", &["rev-parse", "HEAD"]),
        rustc: platform::command_output("rustc", &["--version"]),
        machine: platform::machine_identity(),
        target: platform::command_output("rustc", &["-vV"])
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .expect("rustc host target")
            .to_owned(),
        profile: "release-thin-lto".to_owned(),
        workload: "two_input_fanout_transaction".to_owned(),
        formula_count: formulas,
        warmup_samples: 1,
        recorded_samples,
        exclusive_run_phase_order: [
            "base_calculation",
            "candidate_impact_planning",
            "candidate_calculation",
            "preview_difference",
            "install_difference",
            "report_construction",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        scenarios,
    };
    let json = serde_json::to_string_pretty(&evidence).expect("serialize benchmark evidence");
    if let Some(path) = value_after(&arguments, "--output") {
        fs::write(path, format!("{json}\n")).expect("write benchmark evidence");
    }
    println!("{json}");
}

fn measure_phase_sample(scenario: Scenario, formulas: u32) -> PhaseSample {
    let mut session = workload::core_session(formulas, scenario);
    let revision = session.workbook().semantic_revision();
    let batch = workload::core_transaction_batch(scenario);

    let started = Instant::now();
    let edit_only = session
        .prepare_changes(revision, batch.clone())
        .expect("edit-only prepare");
    let edit_only_prepare_ns = started.elapsed().as_nanos();
    black_box(edit_only.receipt());
    drop(edit_only);

    let started = Instant::now();
    let prepared = session
        .prepare_transaction(
            revision,
            batch,
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("transaction prepare");
    let transaction_prepare_clone_rewrite_ns = started.elapsed().as_nanos();

    let (mut completed, run_phases) = prepared
        .run_with_benchmark_phases()
        .expect("transaction run");
    assert_scenario_report(scenario, &completed);
    let report = completed.report();
    let preview_delta_cells = report
        .preview_changed_count()
        .saturating_add(report.preview_removed_count());
    let install_delta_cells = report
        .install_delta()
        .changed_cells()
        .len()
        .saturating_add(report.install_delta().removed_materialized_cells().len());
    let retained_detail_items = detail_sections()
        .into_iter()
        .map(|section| report.detail_count(section))
        .sum();

    let started = Instant::now();
    let receipt = session
        .install_transaction(&mut completed)
        .expect("transaction install");
    let install_ns = started.elapsed().as_nanos();
    let receipt_delta_cells = receipt
        .calculation_delta()
        .changed_cells()
        .len()
        .saturating_add(
            receipt
                .calculation_delta()
                .removed_materialized_cells()
                .len(),
        );
    assert_eq!(receipt_delta_cells, install_delta_cells);

    let (paging_interop_dto_serialization_ns, serialized_dto_bytes) =
        measure_interop_paging(scenario, formulas);
    PhaseSample {
        edit_only_prepare_ns,
        transaction_prepare_clone_rewrite_ns,
        base_calculation_ns: run_phases[0].as_nanos(),
        candidate_planning_ns: run_phases[1].as_nanos(),
        candidate_calculation_ns: run_phases[2].as_nanos(),
        preview_difference_ns: run_phases[3].as_nanos(),
        install_difference_ns: run_phases[4].as_nanos(),
        report_construction_ns: run_phases[5].as_nanos(),
        paging_interop_dto_serialization_ns,
        install_ns,
        serialized_dto_bytes,
        base_calculation_reused: report_base_reused(scenario),
        base_execution_mode: expected_base_mode(scenario).to_owned(),
        base_decision_reason: expected_base_reason(scenario).to_owned(),
        candidate_execution_mode: expected_candidate_mode(scenario).to_owned(),
        candidate_decision_reason: expected_candidate_reason(scenario).to_owned(),
        preview_delta_cells,
        install_delta_cells,
        retained_detail_items,
    }
}

fn measure_interop_paging(scenario: Scenario, formulas: u32) -> (u128, usize) {
    let mut session = workload::interop_session(formulas, scenario);
    let preview = session
        .preview_changes(
            session.summary().semantic_revision,
            workload::interop_transaction_batch(scenario),
            cellrune_interop::RecalculationModeDto::Auto,
            cellrune_interop::CalculationOptionsDto::default(),
        )
        .expect("interop preview");
    let started = Instant::now();
    let mut serialized_bytes = serde_json::to_vec(&preview)
        .expect("serialize preview summary")
        .len();
    for section in dto_sections() {
        let mut cursor = None;
        loop {
            let page = session
                .preview_changes_page(preview.preview_id, section, cursor, PAGE_ITEMS)
                .expect("interop preview page");
            serialized_bytes = serialized_bytes.saturating_add(
                serde_json::to_vec(&page)
                    .expect("serialize preview page")
                    .len(),
            );
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
    }
    let elapsed = started.elapsed().as_nanos();
    black_box(serialized_bytes);
    session
        .discard_preview(preview.preview_id)
        .expect("discard measured interop preview");
    (elapsed, serialized_bytes)
}

fn retained_child(scenario: Scenario, formulas: u32) {
    let session = workload::core_session(formulas, scenario);
    println!("BASE_READY");
    std::io::stdout().flush().expect("flush base RSS barrier");
    read_barrier();

    let prepared = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            workload::core_transaction_batch(scenario),
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("retained transaction prepare");
    let completed = prepared.run().expect("retained transaction run");
    assert_scenario_report(scenario, &completed);
    println!("COMPLETED_READY");
    std::io::stdout()
        .flush()
        .expect("flush completed RSS barrier");
    read_barrier();
    black_box((&session, &completed));
}

fn measure_retained_child(
    executable: &Path,
    scenario: Scenario,
    formulas: u32,
) -> RetainedMemorySample {
    let mut child = Command::new(executable)
        .arg("--retained-child")
        .arg("--scenario")
        .arg(scenario.as_str())
        .arg("--formulas")
        .arg(formulas.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn retained transaction child");
    let stdout = child.stdout.take().expect("retained child stdout");
    let mut reader = BufReader::new(stdout);
    expect_barrier(&mut reader, "BASE_READY");
    let base_session_rss_bytes = median_child_rss(child.id());
    write_barrier(&mut child, b"complete\n");
    expect_barrier(&mut reader, "COMPLETED_READY");
    let completed_transaction_rss_bytes = median_child_rss(child.id());
    write_barrier(&mut child, b"release\n");
    assert!(child.wait().expect("wait retained child").success());
    let delta = completed_transaction_rss_bytes as i128 - base_session_rss_bytes as i128;
    RetainedMemorySample {
        base_session_rss_bytes,
        completed_transaction_rss_bytes,
        retained_completed_delta_rss_bytes: i64::try_from(delta).expect("RSS delta fits i64"),
    }
}

fn run_phase_child(executable: &Path, scenario: Scenario, formulas: u32) -> PhaseSample {
    let output = Command::new(executable)
        .arg("--phase-child")
        .arg("--scenario")
        .arg(scenario.as_str())
        .arg("--formulas")
        .arg(formulas.to_string())
        .output()
        .expect("spawn transaction phase child");
    assert!(
        output.status.success(),
        "phase child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse phase child JSON")
}

fn assert_scenario_report(scenario: Scenario, completed: &cellrune::CompletedWorkbookTransaction) {
    let report = completed.report();
    assert_eq!(
        report.base_calculation_reused(),
        report_base_reused(scenario)
    );
    assert_eq!(
        report.base_execution_mode(),
        expected_base_execution(scenario)
    );
    assert_eq!(
        report.base_decision_reason(),
        expected_base_decision(scenario)
    );
    assert_eq!(
        report.candidate_execution_mode(),
        expected_candidate_execution(scenario)
    );
    assert_eq!(
        report.candidate_decision_reason(),
        expected_candidate_decision(scenario)
    );
}

fn report_base_reused(scenario: Scenario) -> bool {
    scenario != Scenario::PendingUncalculatedEdit
}

fn expected_base_execution(_scenario: Scenario) -> CalculationExecutionMode {
    CalculationExecutionMode::Incremental
}

fn expected_base_decision(scenario: Scenario) -> CalculationDecisionReason {
    if scenario == Scenario::PendingUncalculatedEdit {
        CalculationDecisionReason::DirtySubset
    } else {
        CalculationDecisionReason::NoDirtyFormulas
    }
}

fn expected_candidate_execution(scenario: Scenario) -> CalculationExecutionMode {
    if scenario == Scenario::TopologyFullFallback {
        CalculationExecutionMode::Full
    } else {
        CalculationExecutionMode::Incremental
    }
}

fn expected_candidate_decision(scenario: Scenario) -> CalculationDecisionReason {
    if scenario == Scenario::TopologyFullFallback {
        CalculationDecisionReason::TopologyChanged
    } else {
        CalculationDecisionReason::DirtySubset
    }
}

fn expected_base_mode(scenario: Scenario) -> &'static str {
    match expected_base_execution(scenario) {
        CalculationExecutionMode::Incremental => "incremental",
        CalculationExecutionMode::Full => "full",
    }
}

fn expected_base_reason(scenario: Scenario) -> &'static str {
    decision_reason_name(expected_base_decision(scenario))
}

fn expected_candidate_mode(scenario: Scenario) -> &'static str {
    match expected_candidate_execution(scenario) {
        CalculationExecutionMode::Incremental => "incremental",
        CalculationExecutionMode::Full => "full",
    }
}

fn expected_candidate_reason(scenario: Scenario) -> &'static str {
    decision_reason_name(expected_candidate_decision(scenario))
}

fn decision_reason_name(reason: CalculationDecisionReason) -> &'static str {
    match reason {
        CalculationDecisionReason::InitialCalculation => "initial_calculation",
        CalculationDecisionReason::FullRequested => "full_requested",
        CalculationDecisionReason::IncrementalRequested => "incremental_requested",
        CalculationDecisionReason::DirtySubset => "dirty_subset",
        CalculationDecisionReason::NoDirtyFormulas => "no_dirty_formulas",
        CalculationDecisionReason::TopologyChanged => "topology_changed",
        CalculationDecisionReason::OptionsChanged => "options_changed",
        CalculationDecisionReason::DynamicTopology => "dynamic_topology",
        CalculationDecisionReason::DirtySetCoversWorkbook => "dirty_set_covers_workbook",
        _ => "unknown",
    }
}

fn detail_sections() -> [TransactionDetailSection; 5] {
    [
        TransactionDetailSection::Affected,
        TransactionDetailSection::Evaluated,
        TransactionDetailSection::PreviewResults,
        TransactionDetailSection::PreviewIssues,
        TransactionDetailSection::InstallResults,
    ]
}

fn dto_sections() -> [TransactionDetailSectionDto; 5] {
    [
        TransactionDetailSectionDto::Affected,
        TransactionDetailSectionDto::Evaluated,
        TransactionDetailSectionDto::PreviewResults,
        TransactionDetailSectionDto::PreviewIssues,
        TransactionDetailSectionDto::InstallResults,
    ]
}

fn median_child_rss(pid: u32) -> usize {
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        samples.push(platform::current_rss_bytes(pid));
        std::thread::sleep(Duration::from_millis(20));
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn expect_barrier(reader: &mut impl BufRead, expected: &str) {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read child barrier");
    assert_eq!(line.trim(), expected);
}

fn write_barrier(child: &mut std::process::Child, value: &[u8]) {
    child
        .stdin
        .as_mut()
        .expect("retained child stdin")
        .write_all(value)
        .expect("write child barrier");
}

fn read_barrier() {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("read parent barrier");
}

fn child_scenario(arguments: &[String]) -> Scenario {
    Scenario::parse(value_after(arguments, "--scenario").expect("child scenario"))
}

fn child_formulas(arguments: &[String]) -> u32 {
    value_after(arguments, "--formulas")
        .expect("child formula count")
        .parse()
        .expect("numeric child formula count")
}

fn value_after<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}
