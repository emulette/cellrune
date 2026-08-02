use super::super::criteria::{
    CompiledCriteria, CompiledWildcardPattern, WildcardStepBudget, compile_criteria_with_work,
};
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};

/// Per-function-call criteria execution state.
///
/// Wildcard matching has two deliberately independent limits: `wildcard_budget` preserves the
/// hard cap for one criteria/lookup function call, while `EvalContext` accumulates the same work
/// across every function nested in the formula cell. Preprocessing and each state transition go
/// through this owner so neither boundary can be bypassed.
pub(super) struct CriteriaRuntime<'engine, 'workbook, 'scope> {
    engine: &'engine Engine<'workbook>,
    context: EvalContext<'scope>,
    wildcard_budget: WildcardStepBudget,
}

impl<'engine, 'workbook, 'scope> CriteriaRuntime<'engine, 'workbook, 'scope> {
    pub(super) fn new(engine: &'engine Engine<'workbook>, context: EvalContext<'scope>) -> Self {
        Self {
            engine,
            context,
            wildcard_budget: WildcardStepBudget::new(engine.max_function_iterations()),
        }
    }

    pub(super) fn compile_criteria(
        &mut self,
        value: &Value,
    ) -> Result<CompiledCriteria, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        compile_criteria_with_work(value, &mut self.wildcard_budget, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compile_exact_equality(
        &mut self,
        value: &Value,
    ) -> Result<Option<CompiledCriteria>, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        CompiledCriteria::exact_equality_with_work(value, &mut self.wildcard_budget, |units| {
            Self::charge_cumulative(engine, context, units)
        })
    }

    pub(super) fn compile_wildcard(
        &mut self,
        pattern: &str,
    ) -> Result<CompiledWildcardPattern, ErrorKind> {
        let engine = self.engine;
        let context = self.context;
        CompiledWildcardPattern::compile_with_work(pattern, &mut self.wildcard_budget, |units| {
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
        criterion.matches_with_work(value, &mut self.wildcard_budget, |units| {
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
        pattern.matches_with_work(value, &mut self.wildcard_budget, |units| {
            Self::charge_cumulative(engine, context, units)
        })
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
