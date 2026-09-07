use std::sync::{Arc, Mutex, OnceLock};

use super::error::MESSAGE_ANALYSIS_CACHE_POISONED;
use super::{CalculationOptions, FormulaCapabilityReport, FunctionUsageReport, pipeline};
use crate::WorkbookSnapshot;

#[derive(Debug)]
struct CachedAnalysis {
    options: CalculationOptions,
    capabilities: OnceLock<FormulaCapabilityReport>,
    usage: OnceLock<FunctionUsageReport>,
}

/// One option set per immutable source; calculated values and parsed engines are not retained.
#[derive(Debug, Default)]
pub(crate) struct WorkbookAnalysisCache {
    current: Mutex<Option<Arc<CachedAnalysis>>>,
}

impl WorkbookAnalysisCache {
    pub(super) fn capabilities(
        &self,
        workbook: &WorkbookSnapshot,
        options: CalculationOptions,
    ) -> FormulaCapabilityReport {
        self.for_options(options)
            .capabilities
            .get_or_init(|| pipeline::scan_formula_capabilities(workbook, options))
            .clone()
    }

    pub(super) fn usage(
        &self,
        workbook: &WorkbookSnapshot,
        options: CalculationOptions,
    ) -> FunctionUsageReport {
        self.for_options(options)
            .usage
            .get_or_init(|| pipeline::scan_function_usage(workbook, options))
            .clone()
    }

    fn for_options(&self, options: CalculationOptions) -> Arc<CachedAnalysis> {
        let mut current = self.current.lock().expect(MESSAGE_ANALYSIS_CACHE_POISONED);
        if let Some(cached) = current.as_ref()
            && cached.options == options
        {
            return Arc::clone(cached);
        }
        let cached = Arc::new(CachedAnalysis {
            options,
            capabilities: OnceLock::new(),
            usage: OnceLock::new(),
        });
        *current = Some(Arc::clone(&cached));
        // Each report initializes outside this lock, independently of the other report.
        cached
    }
}
