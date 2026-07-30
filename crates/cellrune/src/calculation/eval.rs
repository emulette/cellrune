use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use super::ast::Expr;
use super::convert::value_from_cell;
use super::decimal::DecimalTrace;
use super::graph::DependencyGraph;
use super::limits::CalculationLimitKind;
use super::parser::ParseError;
use super::runtime::{CellId, Rect, RectSpan};
use super::scope::{ScopeEntry, ScopeValue, scope_value};
use super::value::{ErrorKind, Value};
use super::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions,
};
use crate::{
    CellContent, CellValue, DefinedNameScope, FiniteNumber, FormulaMetadata, WorkbookSnapshot,
};

mod dependency;
mod expression;
mod materialization;
mod name_graph;
mod orchestration;
mod reference;

use materialization::ArrayRegion;
use reference::{ColumnExtents, cell_at};

pub(super) fn clone_map_cancellable<K, V>(
    source: &BTreeMap<K, V>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<K, V>, ()>
where
    K: Clone + Ord,
    V: Clone,
{
    let mut cloned = BTreeMap::new();
    for (key, value) in source {
        if cancelled() {
            return Err(());
        }
        cloned.insert(key.clone(), value.clone());
    }
    Ok(cloned)
}

pub(super) fn clone_vec_map_cancellable<K, V>(
    source: &BTreeMap<K, Vec<V>>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<K, Vec<V>>, ()>
where
    K: Clone + Ord,
    V: Clone,
{
    let mut cloned = BTreeMap::new();
    for (key, values) in source {
        if cancelled() {
            return Err(());
        }
        let mut cloned_values = Vec::with_capacity(values.len());
        for value in values {
            if cancelled() {
                return Err(());
            }
            cloned_values.push(value.clone());
        }
        cloned.insert(key.clone(), cloned_values);
    }
    Ok(cloned)
}

pub(super) fn clone_set_cancellable<T>(
    source: &BTreeSet<T>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<T>, ()>
where
    T: Clone + Ord,
{
    let mut cloned = BTreeSet::new();
    for value in source {
        if cancelled() {
            return Err(());
        }
        cloned.insert(value.clone());
    }
    Ok(cloned)
}

pub(super) fn clone_vec_cancellable<T>(
    source: &[T],
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<T>, ()>
where
    T: Clone,
{
    let mut cloned = Vec::with_capacity(source.len());
    for value in source {
        if cancelled() {
            return Err(());
        }
        cloned.push(value.clone());
    }
    Ok(cloned)
}

fn collect_workbook_layout(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<(Vec<ArrayRegion>, Vec<ColumnExtents>), ()> {
    let mut array_regions = Vec::new();
    let mut column_extents = Vec::with_capacity(workbook.sheets().len());
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        if cancelled() {
            return Err(());
        }
        let mut extents = ColumnExtents::default();
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            let address = cell.address();
            extents.record(address.column().get(), address.row().get());
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let range = match formula.metadata() {
                FormulaMetadata::Array { range, .. } => *range,
                FormulaMetadata::DynamicArray {
                    range: Some(range), ..
                } => *range,
                FormulaMetadata::Normal
                | FormulaMetadata::Shared { .. }
                | FormulaMetadata::DynamicArray { range: None, .. }
                | FormulaMetadata::DataTable { .. } => continue,
            };
            if range.start() != address || (range.height() == 1 && range.width() == 1) {
                continue;
            }
            array_regions.push(ArrayRegion {
                anchor: (sheet_index, address.row().get(), address.column().get()),
                rect: Rect {
                    sheet: sheet_index,
                    row_start: range.start().row().get(),
                    col_start: range.start().column().get(),
                    row_end: range.end().row().get(),
                    col_end: range.end().column().get(),
                    whole_rows: false,
                },
                provisional: false,
            });
        }
        column_extents.push(extents);
    }
    Ok((array_regions, column_extents))
}

#[derive(Debug, Clone)]
pub(super) struct CompiledWorkbook {
    asts: BTreeMap<CellId, Expr>,
    defined_name_asts: Vec<Option<Expr>>,
    dependencies: DependencyGraph,
    dependency_rectangles: BTreeMap<CellId, Vec<RectSpan>>,
    parse_failures: BTreeMap<CellId, ParseError>,
    name_cycle_cells: BTreeSet<CellId>,
    name_limit_cells: BTreeSet<CellId>,
    dependency_limit_exceeded: bool,
    cycle_cells: BTreeSet<CellId>,
    blocked_cells: BTreeSet<CellId>,
    limits: CalculationLimits,
    incremental_safe: bool,
}

impl CompiledWorkbook {
    pub(super) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        Ok(Self {
            asts: clone_map_cancellable(&self.asts, cancelled)?,
            defined_name_asts: clone_vec_cancellable(&self.defined_name_asts, cancelled)?,
            dependencies: clone_vec_map_cancellable(&self.dependencies, cancelled)?,
            dependency_rectangles: clone_vec_map_cancellable(
                &self.dependency_rectangles,
                cancelled,
            )?,
            parse_failures: clone_map_cancellable(&self.parse_failures, cancelled)?,
            name_cycle_cells: clone_set_cancellable(&self.name_cycle_cells, cancelled)?,
            name_limit_cells: clone_set_cancellable(&self.name_limit_cells, cancelled)?,
            dependency_limit_exceeded: self.dependency_limit_exceeded,
            cycle_cells: clone_set_cancellable(&self.cycle_cells, cancelled)?,
            blocked_cells: clone_set_cancellable(&self.blocked_cells, cancelled)?,
            limits: self.limits,
            incremental_safe: self.incremental_safe,
        })
    }

    pub(super) fn dependencies(&self) -> &DependencyGraph {
        &self.dependencies
    }

    pub(super) fn dependency_rectangles(&self) -> &BTreeMap<CellId, Vec<RectSpan>> {
        &self.dependency_rectangles
    }

    pub(super) const fn limits(&self) -> CalculationLimits {
        self.limits
    }

    pub(super) const fn incremental_safe(&self) -> bool {
        self.incremental_safe
    }

    pub(super) fn formula_count(&self) -> usize {
        self.asts.len() + self.parse_failures.len()
    }
}

#[derive(Debug, Default)]
pub(super) struct EvaluationBudget {
    lambda_depth: Cell<u64>,
    lambda_invocations: Cell<u64>,
    function_iterations: Cell<u64>,
}

impl EvaluationBudget {
    fn enter_lambda(&self, limits: CalculationLimits) -> Result<ActiveLambda<'_>, ErrorKind> {
        let depth = self
            .lambda_depth
            .get()
            .checked_add(1)
            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::LambdaDepth))?;
        if depth > limits.max_lambda_depth() {
            return Err(ErrorKind::ResourceLimit(CalculationLimitKind::LambdaDepth));
        }
        let invocations =
            self.lambda_invocations
                .get()
                .checked_add(1)
                .ok_or(ErrorKind::ResourceLimit(
                    CalculationLimitKind::LambdaInvocations,
                ))?;
        if invocations > limits.max_lambda_invocations() {
            return Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::LambdaInvocations,
            ));
        }
        self.lambda_depth.set(depth);
        self.lambda_invocations.set(invocations);
        Ok(ActiveLambda { budget: self })
    }

    fn charge_function_iterations(
        &self,
        limits: CalculationLimits,
        iterations: u64,
    ) -> Result<(), ErrorKind> {
        let total = self
            .function_iterations
            .get()
            .checked_add(iterations)
            .ok_or(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ))?;
        if total > limits.max_function_iterations() {
            return Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ));
        }
        self.function_iterations.set(total);
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct ActiveLambda<'budget> {
    budget: &'budget EvaluationBudget,
}

impl Drop for ActiveLambda<'_> {
    fn drop(&mut self) {
        self.budget
            .lambda_depth
            .set(self.budget.lambda_depth.get() - 1);
    }
}

#[derive(Clone, Copy)]
pub(super) struct EvalContext<'scope> {
    cell: CellId,
    bindings: &'scope [ScopeEntry],
    defined_name_scope: Option<DefinedNameScope>,
    budget: &'scope EvaluationBudget,
    cancelled: &'scope dyn Fn() -> bool,
}

#[cfg(test)]
fn never_cancelled() -> bool {
    false
}

impl<'scope> EvalContext<'scope> {
    #[cfg(test)]
    pub(super) const fn for_evaluation(cell: CellId, budget: &'scope EvaluationBudget) -> Self {
        Self {
            cell,
            bindings: &[],
            defined_name_scope: None,
            budget,
            cancelled: &never_cancelled,
        }
    }

    pub(super) const fn for_cancellable(
        cell: CellId,
        budget: &'scope EvaluationBudget,
        cancelled: &'scope dyn Fn() -> bool,
    ) -> Self {
        Self {
            cell,
            bindings: &[],
            defined_name_scope: None,
            budget,
            cancelled,
        }
    }

    pub(super) const fn sheet(self) -> usize {
        self.cell.0
    }

    pub(super) const fn row(self) -> u32 {
        self.cell.1
    }

    pub(super) const fn column(self) -> u32 {
        self.cell.2
    }

    pub(super) fn binding(self, name: &str) -> Option<&'scope ScopeValue> {
        scope_value(self.bindings, name)
    }

    pub(super) const fn bindings(self) -> &'scope [ScopeEntry] {
        self.bindings
    }

    pub(super) const fn defined_name_scope(self) -> Option<DefinedNameScope> {
        self.defined_name_scope
    }

    pub(super) const fn with_bindings<'next>(
        self,
        bindings: &'next [ScopeEntry],
    ) -> EvalContext<'next>
    where
        'scope: 'next,
    {
        EvalContext {
            cell: self.cell,
            bindings,
            defined_name_scope: self.defined_name_scope,
            budget: self.budget,
            cancelled: self.cancelled,
        }
    }

    pub(super) const fn without_bindings(self) -> Self {
        Self {
            bindings: &[],
            ..self
        }
    }

    pub(super) const fn with_defined_name_scope(
        self,
        defined_name_scope: Option<DefinedNameScope>,
    ) -> Self {
        Self {
            defined_name_scope,
            ..self
        }
    }

    pub(super) fn enter_lambda(
        self,
        limits: CalculationLimits,
    ) -> Result<ActiveLambda<'scope>, ErrorKind> {
        self.budget.enter_lambda(limits)
    }

    pub(super) fn charge_function_iterations(
        self,
        limits: CalculationLimits,
        iterations: u64,
    ) -> Result<(), ErrorKind> {
        self.budget.charge_function_iterations(limits, iterations)
    }

    pub(super) fn is_cancelled(self) -> bool {
        (self.cancelled)()
    }
}

/// Which of a cell's competing sources `Engine::value_source` selected.
enum ValueSource<'engine> {
    Calculated(&'engine Value),
    Literal(&'engine CellValue),
    Blank,
    Error(ErrorKind),
}

#[derive(Debug)]
pub struct Engine<'workbook> {
    workbook: &'workbook WorkbookSnapshot,
    options: CalculationOptions,
    asts: BTreeMap<CellId, Expr>,
    defined_name_asts: Vec<Option<Expr>>,
    dependencies: DependencyGraph,
    results: BTreeMap<CellId, Value>,
    numeric_decimal_traces: BTreeMap<CellId, DecimalTrace>,
    retained_results: BTreeMap<CellId, CalculationCellResult>,
    array_regions: Vec<ArrayRegion>,
    column_extents: Vec<ColumnExtents>,
    dynamic_spills: BTreeMap<CellId, Rect>,
    parse_failures: BTreeMap<CellId, ParseError>,
    name_cycle_cells: BTreeSet<CellId>,
    name_limit_cells: BTreeSet<CellId>,
    dependency_limit_exceeded: bool,
    pub(super) cycle_cells: BTreeSet<CellId>,
    pub(super) blocked_cells: BTreeSet<CellId>,
    evaluated_cell_count: usize,
}

impl<'workbook> Engine<'workbook> {
    /// Resolves which of a cell's competing sources actually supplies its value.
    ///
    /// A cell can hold a literal and still read as something else — an array formula or a dynamic
    /// spill materializes over it, a cycle or a blocked upstream replaces it with an error. Both
    /// the value and its decimal trace are derived from this one answer, because a trace taken
    /// from a literal that something else overrode would let a sum snap to zero on terms that
    /// never cancelled.
    fn value_source(&self, cell: CellId) -> ValueSource<'_> {
        if let Some(value) = self.results.get(&cell) {
            return ValueSource::Calculated(value);
        }
        if self.cycle_cells.contains(&cell) {
            return ValueSource::Error(ErrorKind::Ref);
        }
        if self.dependency_limit_exceeded {
            return ValueSource::Error(ErrorKind::ResourceLimit(
                CalculationLimitKind::DependencyEdges,
            ));
        }
        if self.blocked_cells.contains(&cell) || self.parse_failures.contains_key(&cell) {
            return ValueSource::Error(ErrorKind::Unsupported);
        }
        if let Some(owner) = self.array_owner(cell) {
            return self.results.get(&owner).map_or(
                ValueSource::Error(ErrorKind::Unsupported),
                ValueSource::Calculated,
            );
        }
        let Some(sheet) = self.workbook.sheets().get(cell.0) else {
            return ValueSource::Error(ErrorKind::Ref);
        };
        let Some(source) = cell_at(sheet, cell.1, cell.2) else {
            return ValueSource::Blank;
        };
        match source.content() {
            CellContent::Literal(value) => ValueSource::Literal(value),
            CellContent::Formula(_) => ValueSource::Error(ErrorKind::Unsupported),
        }
    }

    pub fn cell_value(&self, cell: CellId) -> Value {
        match self.value_source(cell) {
            ValueSource::Calculated(value) => value.clone(),
            ValueSource::Literal(value) => value_from_cell(value),
            ValueSource::Blank => Value::Blank,
            ValueSource::Error(kind) => Value::Error(kind),
        }
    }

    /// Exact decimal behind this cell's number, when the configured policy can act on one.
    ///
    /// Under `Ieee754` nothing consults the trace, so it is not computed: the literal path costs a
    /// `f64::to_string` and an allocation per cell, which would otherwise be charged to every
    /// range read on a policy that discards the result.
    pub(super) fn numeric_decimal_trace(&self, cell: CellId) -> Option<DecimalTrace> {
        if !matches!(
            self.arithmetic_semantics(),
            crate::ArithmeticSemantics::ExcelNearZero
        ) {
            return None;
        }
        match self.value_source(cell) {
            ValueSource::Calculated(_) => self.numeric_decimal_traces.get(&cell).copied(),
            ValueSource::Literal(CellValue::Number(number)) => {
                DecimalTrace::from_number(number.get())
            }
            ValueSource::Literal(_) | ValueSource::Blank | ValueSource::Error(_) => None,
        }
    }

    pub(super) fn calculated_decimal_trace(&self, cell: CellId) -> Option<DecimalTrace> {
        self.numeric_decimal_traces.get(&cell).copied()
    }

    pub(super) fn has_unavailable_dependency(
        &self,
        cell: CellId,
        direct_unavailable: &BTreeSet<CellId>,
    ) -> bool {
        self.dependencies.get(&cell).is_some_and(|dependencies| {
            dependencies.iter().any(|dependency| {
                direct_unavailable.contains(dependency)
                    || self.cycle_cells.contains(dependency)
                    || self.blocked_cells.contains(dependency)
                    || matches!(
                        self.results.get(dependency),
                        Some(Value::Error(kind)) if kind.is_engine_issue()
                    )
            })
        })
    }

    pub fn today_serial(&self) -> Option<f64> {
        self.options.today_serial().map(FiniteNumber::get)
    }

    pub fn now_serial(&self) -> Option<f64> {
        self.options.now_serial().map(FiniteNumber::get)
    }

    pub(super) fn parsed_expr(&self, cell: CellId) -> Option<&Expr> {
        self.asts.get(&cell)
    }

    pub(super) fn parse_failure(&self, cell: CellId) -> Option<&ParseError> {
        self.parse_failures.get(&cell)
    }

    pub(super) const fn dependency_limit_exceeded(&self) -> bool {
        self.dependency_limit_exceeded
    }

    pub(super) const fn arithmetic_semantics(&self) -> crate::ArithmeticSemantics {
        self.options.arithmetic_semantics()
    }

    pub(super) const fn calculation_limits(&self) -> CalculationLimits {
        self.options.limits()
    }

    pub(super) const fn financial_solver_semantics(&self) -> crate::FinancialSolverSemantics {
        self.options.financial_solver_semantics()
    }

    pub(super) fn max_array_cells(&self) -> u64 {
        self.options.limits().max_array_cells()
    }

    pub(super) fn ensure_array_cells(&self, cells: u64) -> Result<(), ErrorKind> {
        if cells > self.max_array_cells() {
            Err(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_text_bytes(&self, bytes: usize) -> Result<(), ErrorKind> {
        if bytes as u64 > self.options.limits().max_text_bytes() {
            Err(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes))
        } else {
            Ok(())
        }
    }

    pub(super) fn bounded_text(&self, text: String) -> Value {
        match self.ensure_text_bytes(text.len()) {
            Ok(()) => Value::Text(text),
            Err(kind) => Value::Error(kind),
        }
    }

    pub(super) fn ensure_function_iterations(&self, iterations: u64) -> Result<(), ErrorKind> {
        if iterations > self.options.limits().max_function_iterations() {
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn charge_function_iterations(
        &self,
        context: EvalContext<'_>,
        iterations: u64,
    ) -> Result<(), ErrorKind> {
        context.charge_function_iterations(self.options.limits(), iterations)
    }

    pub(super) fn max_function_iterations(&self) -> u64 {
        self.options.limits().max_function_iterations()
    }

    pub fn date_system(&self) -> crate::DateSystem {
        self.workbook.date_system()
    }
}

pub(super) fn public_to_internal(
    workbook: &WorkbookSnapshot,
    cell: CalculationCellId,
) -> Option<CellId> {
    let sheet = workbook
        .sheets()
        .iter()
        .position(|candidate| candidate.id() == cell.sheet_id())?;
    Some((
        sheet,
        cell.address().row().get(),
        cell.address().column().get(),
    ))
}

fn value_from_calculation_result(result: &CalculationCellResult) -> Value {
    match result {
        CalculationCellResult::Value(value) => value_from_cell(value),
        CalculationCellResult::Unavailable(issue) => {
            let kind = if issue.code() == CalculationIssueCode::ResourceLimitExceeded {
                issue
                    .detail()
                    .and_then(CalculationLimitKind::from_detail)
                    .map_or(ErrorKind::Unsupported, ErrorKind::ResourceLimit)
            } else if issue.code() == CalculationIssueCode::CircularReference {
                ErrorKind::Ref
            } else {
                ErrorKind::Unsupported
            };
            Value::Error(kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CalculationHints, CellAddress, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata,
        FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName,
        SheetVisibility, WorkbookSource,
    };

    #[test]
    fn capability_analysis_does_not_retain_or_schedule_the_dependency_graph() {
        let workbook = generated_analysis_workbook();

        let engine = Engine::analyze(&workbook, CalculationOptions::default());

        assert!(!engine.asts.is_empty());
        assert!(!engine.dependency_limit_exceeded);
        assert!(engine.dependencies.is_empty());
        assert!(engine.cycle_cells.is_empty());
        assert!(engine.blocked_cells.is_empty());
    }

    #[test]
    fn workbook_layout_collection_polls_cancellation_between_sparse_cells() {
        let workbook = generated_analysis_workbook();
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 3
        };

        assert!(collect_workbook_layout(&workbook, &cancelled).is_err());
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn nested_vector_map_clone_polls_cancellation_between_values() {
        let source = BTreeMap::from([("dependencies", vec![1_u32, 2, 3])]);
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 3
        };

        assert!(clone_vec_map_cancellable(&source, &cancelled).is_err());
        assert_eq!(polls.get(), 3);
    }

    fn generated_analysis_workbook() -> WorkbookSnapshot {
        let mut sheet = Sheet::new(
            SheetId::new(1).expect("valid sheet ID"),
            SheetName::new("Calculations").expect("valid sheet name"),
            SheetVisibility::Visible,
        );
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx("SUM(1,2)").expect("valid formula"),
            SavedResult::Missing,
            FormulaMetadata::Normal,
        );
        sheet
            .insert_cell(
                CellAddress::from_a1("A1").expect("valid cell address"),
                CellContent::Formula(formula),
            )
            .expect("unique formula cell");
        sheet
            .insert_cell(
                CellAddress::from_a1("A2").expect("valid cell address"),
                CellContent::Literal(CellValue::number(1.0).expect("finite number")),
            )
            .expect("unique literal cell");

        WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("calculation-unit-test", "1")
                    .expect("valid provider identity"),
                None,
            ),
        )
        .expect("valid generated workbook")
    }
}
