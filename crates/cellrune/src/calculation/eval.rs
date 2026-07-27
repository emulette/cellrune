use std::collections::{BTreeMap, BTreeSet};

use super::ast::Expr;
use super::convert::value_from_cell;
use super::decimal::DecimalTrace;
use super::graph::DependencyGraph;
use super::lambda::{LambdaBinding, binding_value};
use super::limits::CalculationLimitKind;
use super::parser::ParseError;
use super::runtime::{CellId, Rect};
use super::value::{ErrorKind, Value};
use super::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions,
};
use crate::{CellContent, CellValue, FiniteNumber, WorkbookSnapshot};

mod dependency;
mod expression;
mod materialization;
mod name_graph;
mod orchestration;
mod reference;

use materialization::ArrayRegion;
use reference::cell_at;

#[derive(Debug, Clone)]
pub(super) struct CompiledWorkbook {
    asts: BTreeMap<CellId, Expr>,
    defined_name_asts: Vec<Option<Expr>>,
    dependencies: DependencyGraph,
    dependency_rectangles: BTreeMap<CellId, Vec<Rect>>,
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
    pub(super) fn dependencies(&self) -> &DependencyGraph {
        &self.dependencies
    }

    pub(super) fn dependency_rectangles(&self) -> &BTreeMap<CellId, Vec<Rect>> {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EvalContext<'bindings> {
    cell: CellId,
    bindings: &'bindings [LambdaBinding],
}

impl<'bindings> EvalContext<'bindings> {
    pub(super) const fn for_cell(cell: CellId) -> Self {
        Self {
            cell,
            bindings: &[],
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

    pub(super) fn binding(self, name: &str) -> Option<&'bindings Value> {
        binding_value(self.bindings, name)
    }

    pub(super) const fn bindings(self) -> &'bindings [LambdaBinding] {
        self.bindings
    }

    pub(super) const fn with_bindings<'next>(
        self,
        bindings: &'next [LambdaBinding],
    ) -> EvalContext<'next> {
        EvalContext {
            cell: self.cell,
            bindings,
        }
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
