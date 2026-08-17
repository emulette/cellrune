use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PhaseSample {
    pub(super) edit_only_prepare_ns: u128,
    pub(super) transaction_prepare_clone_rewrite_ns: u128,
    pub(super) base_calculation_ns: u128,
    pub(super) candidate_planning_ns: u128,
    pub(super) candidate_calculation_ns: u128,
    pub(super) preview_difference_ns: u128,
    pub(super) install_difference_ns: u128,
    pub(super) report_construction_ns: u128,
    pub(super) paging_interop_dto_serialization_ns: u128,
    pub(super) install_ns: u128,
    pub(super) serialized_dto_bytes: usize,
    pub(super) base_calculation_reused: bool,
    pub(super) base_execution_mode: String,
    pub(super) base_decision_reason: String,
    pub(super) candidate_execution_mode: String,
    pub(super) candidate_decision_reason: String,
    pub(super) preview_delta_cells: usize,
    pub(super) install_delta_cells: usize,
    pub(super) retained_detail_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PhaseMedians {
    pub(super) edit_only_prepare_ns: f64,
    pub(super) transaction_prepare_clone_rewrite_ns: f64,
    pub(super) base_calculation_ns: f64,
    pub(super) candidate_planning_ns: f64,
    pub(super) candidate_calculation_ns: f64,
    pub(super) preview_difference_ns: f64,
    pub(super) install_difference_ns: f64,
    pub(super) report_construction_ns: f64,
    pub(super) paging_interop_dto_serialization_ns: f64,
    pub(super) install_ns: f64,
}

impl PhaseMedians {
    pub(super) fn from_samples(samples: &[PhaseSample]) -> Self {
        Self {
            edit_only_prepare_ns: median(samples, |sample| sample.edit_only_prepare_ns),
            transaction_prepare_clone_rewrite_ns: median(samples, |sample| {
                sample.transaction_prepare_clone_rewrite_ns
            }),
            base_calculation_ns: median(samples, |sample| sample.base_calculation_ns),
            candidate_planning_ns: median(samples, |sample| sample.candidate_planning_ns),
            candidate_calculation_ns: median(samples, |sample| sample.candidate_calculation_ns),
            preview_difference_ns: median(samples, |sample| sample.preview_difference_ns),
            install_difference_ns: median(samples, |sample| sample.install_difference_ns),
            report_construction_ns: median(samples, |sample| sample.report_construction_ns),
            paging_interop_dto_serialization_ns: median(samples, |sample| {
                sample.paging_interop_dto_serialization_ns
            }),
            install_ns: median(samples, |sample| sample.install_ns),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RetainedMemorySample {
    pub(super) base_session_rss_bytes: usize,
    pub(super) completed_transaction_rss_bytes: usize,
    pub(super) retained_completed_delta_rss_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ScenarioEvidence {
    pub(super) scenario: String,
    pub(super) raw_phase_samples: Vec<PhaseSample>,
    pub(super) phase_medians: PhaseMedians,
    pub(super) raw_retained_memory_samples: Vec<RetainedMemorySample>,
    pub(super) retained_completed_delta_rss_bytes_median: f64,
}

impl ScenarioEvidence {
    pub(super) fn new(
        scenario: String,
        raw_phase_samples: Vec<PhaseSample>,
        raw_retained_memory_samples: Vec<RetainedMemorySample>,
    ) -> Self {
        let phase_medians = PhaseMedians::from_samples(&raw_phase_samples);
        let retained_completed_delta_rss_bytes_median = median_i64(
            &raw_retained_memory_samples
                .iter()
                .map(|sample| sample.retained_completed_delta_rss_bytes)
                .collect::<Vec<_>>(),
        );
        Self {
            scenario,
            raw_phase_samples,
            phase_medians,
            raw_retained_memory_samples,
            retained_completed_delta_rss_bytes_median,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Evidence {
    pub(super) schema: String,
    pub(super) mode: String,
    pub(super) commit: String,
    pub(super) rustc: String,
    pub(super) machine: String,
    pub(super) target: String,
    pub(super) profile: String,
    pub(super) workload: String,
    pub(super) formula_count: u32,
    pub(super) warmup_samples: usize,
    pub(super) recorded_samples: usize,
    pub(super) exclusive_run_phase_order: Vec<String>,
    pub(super) scenarios: Vec<ScenarioEvidence>,
}

fn median(samples: &[PhaseSample], value: impl Fn(&PhaseSample) -> u128) -> f64 {
    let mut values = samples
        .iter()
        .map(|sample| value(sample) as f64)
        .collect::<Vec<_>>();
    median_f64(&mut values)
}

fn median_i64(samples: &[i64]) -> f64 {
    let mut values = samples
        .iter()
        .map(|value| *value as f64)
        .collect::<Vec<_>>();
    median_f64(&mut values)
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}
