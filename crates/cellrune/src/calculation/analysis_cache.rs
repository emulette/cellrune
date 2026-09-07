use std::sync::Mutex;

use super::error::MESSAGE_ANALYSIS_CACHE_POISONED;
use super::{CalculationOptions, FormulaCapabilityReport, FunctionUsageReport, pipeline};
use crate::WorkbookSnapshot;

#[derive(Debug, Clone)]
pub(super) struct AnalysisReports {
    pub(super) capabilities: FormulaCapabilityReport,
    pub(super) usage: FunctionUsageReport,
}

#[derive(Debug)]
struct CachedAnalysis {
    options: CalculationOptions,
    reports: AnalysisReports,
}

/// One option set per immutable source; calculated values and parsed engines are not retained.
#[derive(Debug, Default)]
pub(crate) struct WorkbookAnalysisCache {
    current: Mutex<Option<CachedAnalysis>>,
}

impl WorkbookAnalysisCache {
    pub(super) fn reports(
        &self,
        workbook: &WorkbookSnapshot,
        options: CalculationOptions,
    ) -> AnalysisReports {
        let mut current = self.current.lock().expect(MESSAGE_ANALYSIS_CACHE_POISONED);
        if let Some(cached) = current.as_ref()
            && cached.options == options
        {
            return cached.reports.clone();
        }
        let reports = pipeline::scan_analysis_reports(workbook, options);
        *current = Some(CachedAnalysis {
            options,
            reports: reports.clone(),
        });
        reports
    }
}
