use super::super::coerce::compare;
use super::super::criteria::{
    CompiledCriteria, CompiledWildcardPattern, charge_value_comparison_work,
    compile_criteria_with_work,
};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::{ErrorKind, Value};
use super::database_criteria::CompiledDatabaseCriteria;
use super::util::fifteen_digit_rules;
use std::cmp::Ordering;

/// Formula-cell criteria execution state.
///
/// Preprocessing, text comparison, and wildcard state transitions all charge the formula cell's
/// cumulative function-work budget through this owner, which also polls cancellation before any
/// bounded allocation or linear text pass.
pub(super) struct CriteriaRuntime<'engine, 'workbook, 'scope> {
    engine: &'engine Engine<'workbook>,
    context: EvalContext<'scope>,
}

impl<'engine, 'workbook, 'scope> CriteriaRuntime<'engine, 'workbook, 'scope> {
    pub(super) fn new(engine: &'engine Engine<'workbook>, context: EvalContext<'scope>) -> Self {
        Self { engine, context }
    }

    pub(super) fn compile_criteria(
        &mut self,
        value: &Value,
    ) -> Result<CompiledCriteria, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        compile_criteria_with_work(value, fifteen_digit_rules(engine), |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compile_database_criteria(
        &mut self,
        value: &Value,
    ) -> Result<CompiledDatabaseCriteria, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        CompiledDatabaseCriteria::compile_with_work(value, fifteen_digit_rules(engine), |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compile_exact_equality(
        &mut self,
        value: &Value,
    ) -> Result<Option<CompiledCriteria>, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        CompiledCriteria::exact_equality_with_work(value, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compile_wildcard(
        &mut self,
        pattern: &str,
    ) -> Result<CompiledWildcardPattern, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        CompiledWildcardPattern::compile_with_work(pattern, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn matches(
        &mut self,
        criterion: &CompiledCriteria,
        value: &Value,
    ) -> Result<bool, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        criterion.matches_with_work(value, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn matches_database(
        &mut self,
        criterion: &CompiledDatabaseCriteria,
        value: &Value,
    ) -> Result<bool, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        criterion.matches_with_work(value, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn wildcard_matches(
        &mut self,
        pattern: &CompiledWildcardPattern,
        value: &str,
    ) -> Result<bool, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        pattern.matches_with_work(value, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compare(&mut self, left: &Value, right: &Value) -> Result<Ordering, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        charge_value_comparison_work(left, right, &mut |units| {
            Self::charge_cumulative(engine, context, units)
        })?;
        compare(left, right)
    }

    pub(super) fn charge_work(&mut self, units: u64) -> Result<(), ErrorKind> {
        Self::charge_cumulative(self.engine, self.context, units)
    }

    fn charge_cumulative(
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        units: u64,
    ) -> Result<(), ErrorKind> {
        super::array_common::poll_cancellation(context)?;
        engine.charge_function_iterations(context, units)
    }
}

/// How a conditional scan of SUMIFS, COUNTIFS, MAXIFS, or a sibling reports a failure.
///
/// A scan is charged for each cell it reads, and a row stops at its first unmatched criterion,
/// so a scan whose ranges together hold more than `max_array_cells` cells can still fit. Such a
/// scan calculates when it completes; if a calculation-engine issue stops it, it reports
/// `max_array_cells`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ConditionalScanBound {
    exceeds_array_cells: bool,
}

impl ConditionalScanBound {
    /// The bound of a scan over `ranges` ranges of `rows` by `width` cells.
    pub(super) fn new(engine: &Engine<'_>, rows: u64, width: u64, ranges: usize) -> Self {
        let cells = rows
            .checked_mul(width)
            .and_then(|cells| cells.checked_mul(ranges as u64));
        Self {
            exceeds_array_cells: cells
                .is_none_or(|cells| engine.ensure_array_cells(cells).is_err()),
        }
    }

    /// The error the scan reports when it stops with `kind`.
    pub(super) const fn error(self, kind: ErrorKind) -> ErrorKind {
        if self.exceeds_array_cells && kind.is_engine_issue() {
            ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells)
        } else {
            kind
        }
    }

    /// The value the scan reports when it returns `value`.
    pub(super) fn value(self, value: Value) -> Value {
        match value {
            Value::Error(kind) => Value::Error(self.error(kind)),
            value => value,
        }
    }
}
