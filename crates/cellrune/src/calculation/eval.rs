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
use super::syntax::ParsedFormula;
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
    asts: BTreeMap<CellId, ParsedFormula>,
    defined_name_asts: Vec<Option<ParsedFormula>>,
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
    reference_cells: Cell<u64>,
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

    fn charge_reference_cells(
        &self,
        limits: CalculationLimits,
        cells: u64,
    ) -> Result<(), ErrorKind> {
        let total = self
            .reference_cells
            .get()
            .checked_add(cells)
            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
        if total > limits.max_array_cells() {
            return Err(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
        }
        self.reference_cells.set(total);
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
    charge_reference_work: bool,
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
            charge_reference_work: true,
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
            charge_reference_work: true,
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
            charge_reference_work: self.charge_reference_work,
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

    pub(super) const fn without_reference_work_charge(self) -> Self {
        Self {
            charge_reference_work: false,
            ..self
        }
    }

    pub(super) const fn charges_reference_work(self) -> bool {
        self.charge_reference_work
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

    pub(super) fn charge_reference_cells(
        self,
        limits: CalculationLimits,
        cells: u64,
    ) -> Result<(), ErrorKind> {
        self.budget.charge_reference_cells(limits, cells)
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
    asts: BTreeMap<CellId, ParsedFormula>,
    defined_name_asts: Vec<Option<ParsedFormula>>,
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
        self.asts.get(&cell).map(ParsedFormula::root)
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

    pub(super) fn charge_reference_cells(
        &self,
        context: EvalContext<'_>,
        cells: u64,
    ) -> Result<(), ErrorKind> {
        context.charge_reference_cells(self.options.limits(), cells)
    }

    pub(super) fn read_reference_cell(
        &self,
        context: EvalContext<'_>,
        cell: CellId,
    ) -> Result<Value, ErrorKind> {
        if context.is_cancelled() {
            return Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ));
        }
        self.charge_reference_cells(context, 1)?;
        Ok(self.cell_value(cell))
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
        CalculationCellId, CalculationCellResult, CalculationHints, CalculationIssueCode,
        CellAddress, CellRange, CellValue, DateSystem, ExcelError, FormulaCell, FormulaDialect,
        FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId,
        SheetName, SheetVisibility, Table, TableColumn, TableId, TableName, WorkbookSource,
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

    #[test]
    fn structured_references_and_areas_share_the_multi_area_reference_model() {
        let sheet_id = SheetId::new(1).expect("valid sheet ID");
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("Calculations").expect("valid sheet name"),
            SheetVisibility::Visible,
        );
        for (address, value) in [
            ("A1", CellValue::Text("Region".to_owned())),
            ("B1", CellValue::Text("Amount".to_owned())),
            ("C1", CellValue::Text("Echo".to_owned())),
            ("A2", CellValue::Text("North".to_owned())),
            ("B2", CellValue::number(10.0).expect("finite number")),
            ("A3", CellValue::Text("South".to_owned())),
            ("B3", CellValue::number(20.0).expect("finite number")),
            ("A4", CellValue::Text("West".to_owned())),
            ("B4", CellValue::number(30.0).expect("finite number")),
            ("A5", CellValue::Text("Total".to_owned())),
            ("B5", CellValue::number(60.0).expect("finite number")),
            ("E1", CellValue::Text("Key".to_owned())),
            ("F1", CellValue::Text("#OfItems".to_owned())),
            ("E2", CellValue::Text("A".to_owned())),
            ("F2", CellValue::number(1.0).expect("finite number")),
            ("E3", CellValue::Text("B".to_owned())),
            ("F3", CellValue::number(2.0).expect("finite number")),
            ("J1", CellValue::number(7.0).expect("finite number")),
            ("K1", CellValue::number(8.0).expect("finite number")),
            ("J2", CellValue::number(9.0).expect("finite number")),
            ("K2", CellValue::number(10.0).expect("finite number")),
            ("P1", CellValue::Text("Label".to_owned())),
            ("Q1", CellValue::Text("Amount".to_owned())),
            ("P2", CellValue::Text("Total".to_owned())),
            ("Q2", CellValue::number(5.0).expect("finite number")),
            ("S1", CellValue::Text("Label".to_owned())),
            ("T1", CellValue::Text("Amount".to_owned())),
            ("S2", CellValue::Text("Only".to_owned())),
            ("T2", CellValue::number(11.0).expect("finite number")),
        ] {
            sheet
                .insert_cell(
                    CellAddress::from_a1(address).expect("valid literal address"),
                    CellContent::Literal(value),
                )
                .expect("unique literal cell");
        }
        for (address, formula) in [
            ("C2", "Sales[@Amount]"),
            ("C3", "[@Amount]"),
            ("C4", "[@Amount]"),
            ("C5", "[@Amount]"),
            ("H1", "SUM(Sales[Amount])"),
            ("H2", "SUM(Sales[[#Headers],[#Data],[Amount]])"),
            ("H3", "SUM(Sales[[#Data],[#Totals],[Amount]])"),
            ("H4", "AREAS((A1:A2,C1:C2,A1:A2))"),
            ("H5", "AREAS(A1:C3 A2:B4)"),
            ("H6", "AREAS(NoTotals[#Totals])"),
            ("H7", "AREAS(Sales[#Totals])"),
            ("H8", "AREAS((NoTotals[#Totals],A1))"),
            ("H9", "AREAS((A1:A2,A2:A3) A2)"),
            ("H10", "AREAS(Sales[])"),
            ("H11", "SUM((A2:A3,B2:B3))"),
            ("H12", "SUM(Missing[Amount])"),
            ("H13", "SUM(Sales[Missing])"),
            ("H14", "[@Amount]"),
            ("H15", "AREAS(A1 B1)"),
            ("H16", "AREAS()"),
            ("H17", "AREAS(A1,B1)"),
            ("H18", "AREAS(NoTotals[[#Totals],[Missing]])"),
            ("H19", "ISREF((A1,B1))"),
            ("H20", "ISREF(NoTotals[#Totals])"),
            ("H21", "SUM(NoTotals['#OfItems])"),
            ("H22", "SUM(Sales[aMoUnT])"),
            ("H23", "SUM(Sales[[Region]:[Amount]])"),
            ("H24", "SUM(Sales[[Amount]:[Region]])"),
            ("H25", "SUM(Sales[[#All],[Amount]])"),
            ("H26", "AREAS(EmptyData[#Data])"),
            ("H27", "SUM(EmptyData[[#All],[Amount]])"),
            ("H28", "AREAS((A1,B1) (A1,C1))"),
            ("H29", "AREAS(1:100)"),
            ("H30", "ISREF(1:100)"),
            ("H31", "SUM(A2:A3)+SUM(B2:B3)"),
            ("H32", "SUM(SingleRow[Amount])"),
            ("H33", "SUM(Remote[Amount])"),
            ("H34", "COUNTBLANK(A6:A7)+COUNTBLANK(B6:B7)"),
            ("H35", "AREAS((A1,RemoteData!A1))"),
            ("H36", "ISREF((A1,RemoteData!A1))"),
            ("H37", "AREAS(A1 RemoteData!A1)"),
            ("H38", "AREAS(Calculations:RemoteData!A1)"),
            ("H39", "ISREF(Calculations:RemoteData!A1)"),
            ("H40", "SUM((A1,B1):C3)"),
            ("H41", "AREAS(#REF!)"),
            ("H42", "AREAS(A1:#REF!)"),
            ("H43", "AREAS((A1,#REF!))"),
            ("H44", "SUM(NoTotals[#Totals])"),
            ("H45", "ISREF(LET(x,NoTotals[#Totals],x))"),
            ("H46", "AREAS(LET(x,NoTotals[#Totals],x))"),
            ("H47", "AREAS(NoTotals[#Totals]:C3)"),
            ("H48", "AREAS(NoTotals[#Totals] A1)"),
            ("H49", "ISREF(NoSuchName)"),
            ("H50", "NoTotals[#Totals]"),
            ("H51", "LET(x,NoTotals[#Totals],x)"),
            ("H52", "COUNTBLANK((A6,B6) A6)"),
            ("H53", "SUM(((A6,B6) A6)+0)"),
            ("H54", "INDEX((J1,K1),1)"),
            ("H55", "INDEX((J1,K1),1,1,2)"),
            ("H56", "INDEX((J1,K1),1,1,3)"),
            ("H57", "OFFSET((J1,K1),0,0)"),
            ("H58", "ISREF(LET(x,NoSuchName,x))"),
            ("H59", "COUNTA(Sales[#Headers])"),
            ("M1", "AREAS(Headerless[#Headers])"),
            ("M2", "SUM(Headerless[#Data])"),
        ] {
            sheet
                .insert_cell(
                    CellAddress::from_a1(address).expect("valid formula address"),
                    CellContent::Formula(FormulaCell::new(
                        FormulaDialect::ExcelA1,
                        FormulaText::from_xlsx(formula).expect("valid formula"),
                        SavedResult::Missing,
                        FormulaMetadata::Normal,
                    )),
                )
                .expect("unique formula cell");
        }
        let sales = Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Sales").expect("table name"),
            TableName::new("Sales").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("A1").expect("table start"),
                CellAddress::from_a1("C5").expect("table end"),
            )
            .expect("table range"),
            1,
            1,
            vec![
                TableColumn::new(1, "Region", None).expect("column"),
                TableColumn::new(2, "Amount", None).expect("column"),
                TableColumn::new(3, "Echo", None).expect("column"),
            ],
        )
        .expect("valid table");
        let no_totals = Table::new(
            TableId::new(2).expect("table ID"),
            TableName::new("NoTotals").expect("table name"),
            TableName::new("NoTotals").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("E1").expect("table start"),
                CellAddress::from_a1("F3").expect("table end"),
            )
            .expect("table range"),
            1,
            0,
            vec![
                TableColumn::new(1, "Key", None).expect("column"),
                TableColumn::new(2, "#OfItems", None).expect("column"),
            ],
        )
        .expect("valid table");
        let headerless = Table::new(
            TableId::new(3).expect("table ID"),
            TableName::new("Headerless").expect("table name"),
            TableName::new("Headerless").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("J1").expect("table start"),
                CellAddress::from_a1("K2").expect("table end"),
            )
            .expect("table range"),
            0,
            0,
            vec![
                TableColumn::new(1, "Left", None).expect("column"),
                TableColumn::new(2, "Right", None).expect("column"),
            ],
        )
        .expect("valid table");
        let empty_data = Table::new(
            TableId::new(4).expect("table ID"),
            TableName::new("EmptyData").expect("table name"),
            TableName::new("EmptyData").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("P1").expect("table start"),
                CellAddress::from_a1("Q2").expect("table end"),
            )
            .expect("table range"),
            1,
            1,
            vec![
                TableColumn::new(1, "Label", None).expect("column"),
                TableColumn::new(2, "Amount", None).expect("column"),
            ],
        )
        .expect("valid table");
        let single_row = Table::new(
            TableId::new(5).expect("table ID"),
            TableName::new("SingleRow").expect("table name"),
            TableName::new("SingleRow").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("S1").expect("table start"),
                CellAddress::from_a1("T2").expect("table end"),
            )
            .expect("table range"),
            1,
            0,
            vec![
                TableColumn::new(1, "Label", None).expect("column"),
                TableColumn::new(2, "Amount", None).expect("column"),
            ],
        )
        .expect("valid table");
        sheet.set_tables(vec![sales, no_totals, headerless, empty_data, single_row]);
        let remote_sheet_id = SheetId::new(2).expect("valid remote sheet ID");
        let mut remote_sheet = Sheet::new(
            remote_sheet_id,
            SheetName::new("RemoteData").expect("valid remote sheet name"),
            SheetVisibility::Visible,
        );
        for (address, value) in [
            ("A1", CellValue::Text("Label".to_owned())),
            ("B1", CellValue::Text("Amount".to_owned())),
            ("A2", CellValue::Text("First".to_owned())),
            ("B2", CellValue::number(4.0).expect("finite number")),
            ("A3", CellValue::Text("Second".to_owned())),
            ("B3", CellValue::number(5.0).expect("finite number")),
        ] {
            remote_sheet
                .insert_cell(
                    CellAddress::from_a1(address).expect("valid remote address"),
                    CellContent::Literal(value),
                )
                .expect("unique remote literal cell");
        }
        remote_sheet.set_tables(vec![
            Table::new(
                TableId::new(6).expect("table ID"),
                TableName::new("Remote").expect("table name"),
                TableName::new("Remote").expect("display name"),
                CellRange::new(
                    CellAddress::from_a1("A1").expect("table start"),
                    CellAddress::from_a1("B3").expect("table end"),
                )
                .expect("table range"),
                1,
                0,
                vec![
                    TableColumn::new(1, "Label", None).expect("column"),
                    TableColumn::new(2, "Amount", None).expect("column"),
                ],
            )
            .expect("valid table"),
        ]);
        let workbook = WorkbookSnapshot::new(
            vec![sheet, remote_sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("reference-value-test", "1")
                    .expect("valid provider identity"),
                None,
            ),
        )
        .expect("valid workbook");

        assert!(crate::calculation::scan_formula_capabilities(&workbook).is_supported());
        let calculation =
            crate::calculation::calculate_workbook(&workbook, crate::CalculationOptions::default());
        let number = |address: &str| {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("result address"),
            );
            let Some(CalculationCellResult::Value(CellValue::Number(value))) = calculation.cell(id)
            else {
                panic!(
                    "numeric result expected at {address}, got {:?}",
                    calculation.cell(id)
                );
            };
            value.get()
        };
        for (address, expected) in [
            ("C2", 10.0),
            ("C3", 20.0),
            ("C4", 30.0),
            ("H1", 60.0),
            ("H2", 60.0),
            ("H3", 120.0),
            ("H4", 3.0),
            ("H5", 1.0),
            ("H7", 1.0),
            ("H9", 2.0),
            ("H10", 1.0),
            ("H11", 30.0),
            ("M2", 34.0),
            ("H21", 3.0),
            ("H22", 60.0),
            ("H23", 60.0),
            ("H25", 120.0),
            ("H27", 5.0),
            ("H28", 1.0),
            ("H29", 1.0),
            ("H31", 30.0),
            ("H32", 11.0),
            ("H33", 9.0),
            ("H34", 4.0),
            ("H40", 60.0),
            ("H24", 60.0),
            ("H52", 1.0),
            ("H53", 0.0),
            ("H54", 7.0),
            ("H55", 8.0),
            ("H59", 3.0),
        ] {
            assert_eq!(number(address), expected, "{address}");
        }
        for (address, expected) in [
            ("H12", ExcelError::Name),
            ("H13", ExcelError::Reference),
            ("H14", ExcelError::Value),
            ("M1", ExcelError::Reference),
            ("H15", ExcelError::Null),
            ("H16", ExcelError::Value),
            ("H17", ExcelError::Value),
            ("H18", ExcelError::Reference),
            ("C5", ExcelError::Value),
            ("H6", ExcelError::Reference),
            ("H8", ExcelError::Reference),
            ("H26", ExcelError::Reference),
            ("H35", ExcelError::Value),
            ("H37", ExcelError::Value),
            ("H38", ExcelError::Value),
            ("H41", ExcelError::Reference),
            ("H42", ExcelError::Reference),
            ("H43", ExcelError::Reference),
            ("H44", ExcelError::Reference),
            ("H46", ExcelError::Reference),
            ("H47", ExcelError::Reference),
            ("H48", ExcelError::Reference),
            ("H50", ExcelError::Reference),
            ("H51", ExcelError::Reference),
            ("H56", ExcelError::Reference),
            ("H57", ExcelError::Value),
        ] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("error result address"),
            );
            assert_eq!(
                calculation.cell(id),
                Some(&CalculationCellResult::Value(CellValue::Error(expected))),
                "{address}"
            );
        }
        for address in ["H19", "H30"] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("logical result address"),
            );
            assert_eq!(
                calculation.cell(id),
                Some(&CalculationCellResult::Value(CellValue::Logical(true))),
                "{address}"
            );
        }
        for address in ["H20", "H36", "H39", "H45", "H49", "H58"] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("false logical result address"),
            );
            assert_eq!(
                calculation.cell(id),
                Some(&CalculationCellResult::Value(CellValue::Logical(false))),
                "{address}"
            );
        }

        let limits = crate::CalculationLimits::default()
            .with_max_reference_areas(2)
            .expect("non-zero reference-area limit");
        let limited = crate::calculation::calculate_workbook(
            &workbook,
            crate::CalculationOptions::default().with_limits(limits),
        );
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1("H4").expect("limited result address"),
        );
        let Some(CalculationCellResult::Unavailable(issue)) = limited.cell(id) else {
            panic!("three-area reference must exceed a two-area limit");
        };
        assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
        assert_eq!(issue.detail(), Some("max_reference_areas"));

        let limits = crate::CalculationLimits::default()
            .with_max_function_iterations(2)
            .expect("non-zero function-iteration limit");
        let limited = crate::calculation::calculate_workbook(
            &workbook,
            crate::CalculationOptions::default().with_limits(limits),
        );
        for (address, expected) in [("H52", 1.0), ("H53", 0.0)] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("lookahead budget result address"),
            );
            assert_eq!(
                limited.cell(id),
                Some(&CalculationCellResult::Value(
                    CellValue::number(expected).expect("finite lookahead result")
                )),
                "{address}"
            );
        }
        let intersection_id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1("H28").expect("intersection result address"),
        );
        let Some(CalculationCellResult::Unavailable(issue)) = limited.cell(intersection_id) else {
            panic!("four pairwise intersection checks must exceed a two-iteration limit");
        };
        assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
        assert_eq!(issue.detail(), Some("max_function_iterations"));

        let limits = crate::CalculationLimits::default()
            .with_max_array_cells(3)
            .expect("non-zero array-cell limit");
        let limited = crate::calculation::calculate_workbook(
            &workbook,
            crate::CalculationOptions::default().with_limits(limits),
        );
        for address in ["H31", "H34"] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("limited result address"),
            );
            let Some(CalculationCellResult::Unavailable(issue)) = limited.cell(id) else {
                panic!(
                    "two individually bounded references must share one cumulative cell budget at {address}"
                );
            };
            assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
            assert_eq!(issue.detail(), Some("max_array_cells"));
        }
        for address in ["H29", "H30"] {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("metadata-only result address"),
            );
            let expected = if address == "H29" {
                CalculationCellResult::Value(
                    CellValue::number(1.0).expect("finite metadata result"),
                )
            } else {
                CalculationCellResult::Value(CellValue::Logical(true))
            };
            assert_eq!(limited.cell(id), Some(&expected), "{address}");
        }

        let limits = crate::CalculationLimits::default()
            .with_max_reference_areas(1)
            .expect("non-zero reference-area limit");
        let limited = crate::calculation::calculate_workbook(
            &workbook,
            crate::CalculationOptions::default().with_limits(limits),
        );
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1("H19").expect("ISREF engine-issue address"),
        );
        let Some(CalculationCellResult::Unavailable(issue)) = limited.cell(id) else {
            panic!("ISREF must propagate reference resource limits");
        };
        assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
        assert_eq!(issue.detail(), Some("max_reference_areas"));
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
