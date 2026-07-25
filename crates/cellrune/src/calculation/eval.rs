use std::collections::{BTreeMap, BTreeSet};

use super::ast::Expr;
use super::convert::value_from_cell;
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
use crate::{CellContent, FiniteNumber, WorkbookSnapshot};

mod dependency;
mod expression;
mod materialization;
mod name_graph;
mod orchestration;
mod reference;

use materialization::ArrayRegion;
use reference::{cell_at, is_reference_returning_function};

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

#[derive(Debug)]
pub struct Engine<'workbook> {
    workbook: &'workbook WorkbookSnapshot,
    options: CalculationOptions,
    asts: BTreeMap<CellId, Expr>,
    defined_name_asts: Vec<Option<Expr>>,
    dependencies: DependencyGraph,
    results: BTreeMap<CellId, Value>,
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
    pub fn cell_value(&self, cell: CellId) -> Value {
        if let Some(value) = self.results.get(&cell) {
            return value.clone();
        }
        if self.cycle_cells.contains(&cell) {
            return Value::Error(ErrorKind::Ref);
        }
        if self.dependency_limit_exceeded {
            return Value::Error(ErrorKind::ResourceLimit(
                CalculationLimitKind::DependencyEdges,
            ));
        }
        if self.blocked_cells.contains(&cell) || self.parse_failures.contains_key(&cell) {
            return Value::Error(ErrorKind::Unsupported);
        }
        if let Some(owner) = self.array_owner(cell) {
            return self
                .results
                .get(&owner)
                .cloned()
                .unwrap_or(Value::Error(ErrorKind::Unsupported));
        }
        let Some(sheet) = self.workbook.sheets().get(cell.0) else {
            return Value::Error(ErrorKind::Ref);
        };
        let Some(source) = cell_at(sheet, cell.1, cell.2) else {
            return Value::Blank;
        };
        match source.content() {
            CellContent::Literal(value) => value_from_cell(value),
            CellContent::Formula(_) => Value::Error(ErrorKind::Unsupported),
        }
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

fn public_to_internal(workbook: &WorkbookSnapshot, cell: CalculationCellId) -> Option<CellId> {
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
