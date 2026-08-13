//! Manual O5 phase-isolated latency and memory evidence.
//!
//! Run the release binary on the baseline and candidate commits and retain stdout as JSON:
//! `cargo bench -p cellrune-integration-tests --bench v015_phase_memory -- --output evidence.json`.
//! Passing `--baseline baseline.json` additionally applies the 0.1.15 acceptance equations. This
//! is release evidence, not a CI required check. Formal evidence requires a clean worktree so the
//! recorded commit names the exact measured source. `--smoke` is the sole dirty-worktree exception.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
};
use serde::{Deserialize, Serialize};
const RECORDED_SAMPLES: usize = 10;
const FORMULAS: u32 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirtyMemorySample {
    peak_live_heap_bytes: usize,
    end_live_heap_bytes: usize,
    total_allocated_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Summary {
    median: f64,
    mad: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Evidence {
    schema: String,
    mode: String,
    commit: String,
    rustc: String,
    machine: String,
    target: String,
    profile: String,
    workload: String,
    warmup_samples: usize,
    recorded_samples: usize,
    raw_dirty_latency_ns: Vec<u128>,
    raw_dirty_memory_samples: Vec<DirtyMemorySample>,
    raw_cached_warm_full_latency_ns: Vec<u128>,
    raw_retained_rss_bytes: Vec<usize>,
    dirty_latency_ns: Summary,
    dirty_peak_live_heap_bytes: Summary,
    cached_warm_full_latency_ns: Summary,
    retained_rss_bytes: Summary,
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let child_mode = arguments
        .first()
        .is_some_and(|value| value.ends_with("-child"));
    let formulas = if child_mode {
        value_after(&arguments, "--formulas")
            .expect("measurement children require --formulas")
            .parse()
            .expect("numeric formula count")
    } else {
        FORMULAS
    };
    if arguments
        .first()
        .is_some_and(|value| value == "--dirty-latency-child")
    {
        println!("{}", measure_dirty_latency_child(formulas));
        return;
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--warm-full-child")
    {
        println!("{}", measure_warm_full_child(formulas));
        return;
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--retained-child")
    {
        retained_child(formulas);
        return;
    }

    let smoke = arguments.iter().any(|value| value == "--smoke");
    assert!(
        !(smoke && value_after(&arguments, "--baseline").is_some()),
        "smoke measurements cannot be used for baseline acceptance"
    );
    if !smoke {
        let status = command_output("git", &["status", "--porcelain"]);
        assert!(
            status.is_empty(),
            "formal release evidence requires a clean worktree; use --smoke only for orchestration verification"
        );
    }
    let formulas = if smoke { 100 } else { FORMULAS };
    let recorded_samples = if smoke { 1 } else { RECORDED_SAMPLES };
    let executable = std::env::current_exe().expect("resolve benchmark executable");
    let heap_executable = build_heap_helper();
    let _warmup_dirty = run_number_child(&executable, "--dirty-latency-child", formulas);
    let _warmup_heap =
        run_json_child::<DirtyMemorySample>(&heap_executable, "--dirty-memory-child", formulas);
    let _warmup_full = run_number_child(&executable, "--warm-full-child", formulas);
    let mut dirty_latency = Vec::with_capacity(recorded_samples);
    let mut dirty_memory = Vec::<DirtyMemorySample>::with_capacity(recorded_samples);
    let mut warm_full = Vec::with_capacity(recorded_samples);
    let mut retained = Vec::with_capacity(recorded_samples);
    for _ in 0..recorded_samples {
        dirty_latency.push(run_number_child(
            &executable,
            "--dirty-latency-child",
            formulas,
        ));
        dirty_memory.push(run_json_child(
            &heap_executable,
            "--dirty-memory-child",
            formulas,
        ));
        warm_full.push(run_number_child(&executable, "--warm-full-child", formulas));
        retained.push(measure_retained_child(&executable, formulas));
    }
    let evidence = Evidence {
        schema: "cellrune_0_1_15_phase_memory_v2".to_owned(),
        mode: if smoke { "smoke" } else { "formal" }.to_owned(),
        commit: command_output("git", &["rev-parse", "HEAD"]),
        rustc: command_output("rustc", &["--version"]),
        machine: machine_identity(),
        target: command_output("rustc", &["-vV"])
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .expect("rustc host target")
            .to_owned(),
        profile: "release-thin-lto".to_owned(),
        workload: format!("independent_{formulas}_single_dirty"),
        warmup_samples: 1,
        recorded_samples,
        dirty_latency_ns: summarize(
            &dirty_latency
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>(),
        ),
        dirty_peak_live_heap_bytes: summarize(
            &dirty_memory
                .iter()
                .map(|sample| sample.peak_live_heap_bytes as f64)
                .collect::<Vec<_>>(),
        ),
        cached_warm_full_latency_ns: summarize(
            &warm_full
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>(),
        ),
        retained_rss_bytes: summarize(
            &retained
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>(),
        ),
        raw_dirty_latency_ns: dirty_latency,
        raw_dirty_memory_samples: dirty_memory,
        raw_cached_warm_full_latency_ns: warm_full,
        raw_retained_rss_bytes: retained,
    };
    if let Some(path) = value_after(&arguments, "--baseline") {
        let baseline: Evidence =
            serde_json::from_slice(&fs::read(path).expect("read baseline evidence"))
                .expect("parse baseline evidence");
        assert_acceptance(&baseline, &evidence);
    }
    let json = serde_json::to_string_pretty(&evidence).expect("serialize evidence");
    if let Some(path) = value_after(&arguments, "--output") {
        fs::write(path, format!("{json}\n")).expect("write evidence");
    }
    println!("{json}");
}

fn measure_dirty_latency_child(formulas: u32) -> u128 {
    let mut session = independent_session(formulas);
    recalculate(&mut session, RecalculationMode::Full);
    let started = Instant::now();
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
    started.elapsed().as_nanos()
}

fn measure_warm_full_child(formulas: u32) -> u128 {
    let mut session = independent_session(formulas);
    recalculate(&mut session, RecalculationMode::Full);
    let started = Instant::now();
    recalculate(&mut session, RecalculationMode::Full);
    started.elapsed().as_nanos()
}

fn retained_child(formulas: u32) {
    let mut session = independent_session(formulas);
    recalculate(&mut session, RecalculationMode::Full);
    println!("READY");
    std::io::stdout().flush().expect("flush READY barrier");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("read release barrier");
    std::hint::black_box(session);
}

fn measure_retained_child(executable: &std::path::Path, formulas: u32) -> usize {
    let mut child = Command::new(executable)
        .arg("--retained-child")
        .arg("--formulas")
        .arg(formulas.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn retained-state child");
    let stdout = child.stdout.take().expect("retained child stdout");
    let mut reader = BufReader::new(stdout);
    let mut ready = String::new();
    reader.read_line(&mut ready).expect("read READY barrier");
    assert_eq!(ready.trim(), "READY");
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        samples.push(current_rss_bytes(child.id()));
        std::thread::sleep(Duration::from_millis(20));
    }
    child
        .stdin
        .as_mut()
        .expect("retained child stdin")
        .write_all(b"release\n")
        .expect("release retained child");
    assert!(child.wait().expect("wait retained child").success());
    median_usize(&mut samples)
}

fn current_rss_bytes(pid: u32) -> usize {
    if cfg!(target_os = "linux") {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).expect("read child status");
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|line| line.split_whitespace().next())
            .expect("VmRSS value")
            .parse::<usize>()
            .expect("numeric VmRSS");
        value * 1_024
    } else {
        command_output("ps", &["-o", "rss=", "-p", &pid.to_string()])
            .parse::<usize>()
            .expect("numeric ps RSS")
            * 1_024
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

fn run_json_child<T: for<'de> Deserialize<'de>>(
    executable: &std::path::Path,
    argument: &str,
    formulas: u32,
) -> T {
    serde_json::from_slice(&run_child(executable, argument, formulas)).expect("parse child JSON")
}

fn run_number_child(executable: &std::path::Path, argument: &str, formulas: u32) -> u128 {
    String::from_utf8(run_child(executable, argument, formulas))
        .expect("child UTF-8")
        .trim()
        .parse()
        .expect("child number")
}

fn run_child(executable: &std::path::Path, argument: &str, formulas: u32) -> Vec<u8> {
    let output = Command::new(executable)
        .arg(argument)
        .arg("--formulas")
        .arg(formulas.to_string())
        .output()
        .expect("spawn measurement child");
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn build_heap_helper() -> std::path::PathBuf {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("integration package is under the workspace root");
    let status = Command::new("cargo")
        .args([
            "build",
            "--profile",
            "bench",
            "--locked",
            "-p",
            "cellrune-integration-tests",
            "--bin",
            "v015_phase_memory_heap",
        ])
        .current_dir(workspace)
        .status()
        .expect("build heap measurement helper");
    assert!(status.success(), "heap measurement helper build failed");
    let mut executable = std::env::current_exe().expect("benchmark executable path");
    executable.pop();
    if executable.ends_with("deps") {
        executable.pop();
    }
    executable.push(format!(
        "v015_phase_memory_heap{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        executable.is_file(),
        "heap helper missing at {executable:?}"
    );
    executable
}

fn summarize(values: &[f64]) -> Summary {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let median = median_f64(&ordered);
    let mut deviations = ordered
        .iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    Summary {
        median,
        mad: median_f64(&deviations),
    }
}

fn median_f64(ordered: &[f64]) -> f64 {
    let middle = ordered.len() / 2;
    if ordered.len().is_multiple_of(2) {
        (ordered[middle - 1] + ordered[middle]) / 2.0
    } else {
        ordered[middle]
    }
}

fn median_usize(values: &mut [usize]) -> usize {
    values.sort_unstable();
    values[values.len() / 2]
}

fn assert_acceptance(baseline: &Evidence, candidate: &Evidence) {
    assert_eq!(baseline.schema, candidate.schema);
    assert_eq!(baseline.mode, "formal");
    assert_eq!(candidate.mode, "formal");
    assert_eq!(baseline.warmup_samples, 1);
    assert_eq!(candidate.warmup_samples, 1);
    assert_eq!(baseline.recorded_samples, RECORDED_SAMPLES);
    assert_eq!(candidate.recorded_samples, RECORDED_SAMPLES);
    assert_eq!(baseline.raw_dirty_latency_ns.len(), RECORDED_SAMPLES);
    assert_eq!(candidate.raw_dirty_latency_ns.len(), RECORDED_SAMPLES);
    assert_eq!(baseline.raw_dirty_memory_samples.len(), RECORDED_SAMPLES);
    assert_eq!(candidate.raw_dirty_memory_samples.len(), RECORDED_SAMPLES);
    assert_eq!(
        baseline.raw_cached_warm_full_latency_ns.len(),
        RECORDED_SAMPLES
    );
    assert_eq!(
        candidate.raw_cached_warm_full_latency_ns.len(),
        RECORDED_SAMPLES
    );
    assert_eq!(baseline.raw_retained_rss_bytes.len(), RECORDED_SAMPLES);
    assert_eq!(candidate.raw_retained_rss_bytes.len(), RECORDED_SAMPLES);
    assert_eq!(baseline.rustc, candidate.rustc);
    assert_eq!(baseline.machine, candidate.machine);
    assert_eq!(baseline.target, candidate.target);
    assert_eq!(baseline.profile, candidate.profile);
    assert_eq!(baseline.workload, candidate.workload);
    let required_heap_improvement = (0.05 * baseline.dirty_peak_live_heap_bytes.median).max(
        3.0 * baseline
            .dirty_peak_live_heap_bytes
            .mad
            .max(candidate.dirty_peak_live_heap_bytes.mad),
    );
    assert!(
        candidate.dirty_peak_live_heap_bytes.median
            <= baseline.dirty_peak_live_heap_bytes.median - required_heap_improvement
    );
    assert!(
        candidate.cached_warm_full_latency_ns.median
            <= 1.05 * baseline.cached_warm_full_latency_ns.median
    );
    assert!(candidate.retained_rss_bytes.median <= 1.05 * baseline.retained_rss_bytes.median);
}

fn machine_identity() -> String {
    if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "hw.model"])
    } else {
        let machine = command_output("uname", &["-m"]);
        let cpu = fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|contents| {
                contents.lines().find_map(|line| {
                    line.strip_prefix("model name")
                        .and_then(|value| value.split_once(':').map(|(_, model)| model.trim()))
                        .map(str::to_owned)
                })
            })
            .unwrap_or_else(|| "unknown-cpu".to_owned());
        format!("{machine}:{cpu}")
    }
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .expect("run metadata command");
    assert!(output.status.success(), "{program} failed");
    String::from_utf8(output.stdout)
        .expect("metadata UTF-8")
        .trim()
        .to_owned()
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
