use std::collections::{BTreeMap, BTreeSet};

use super::materialization::collect_array_regions;
use super::reference::collect_column_extents;
use super::{CompiledWorkbook, Engine, EvalContext, EvaluationBudget, public_to_internal};
use crate::calculation::graph::{DependencyGraph, schedule};
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::runtime::CellId;
use crate::calculation::value::{ErrorKind, Value};
use crate::calculation::{
    CalculationCellId, CalculationCellResult, CalculationLimitKind, CalculationOptions,
    CalculationSnapshot,
};
use crate::{CellContent, WorkbookSnapshot};

impl<'workbook> Engine<'workbook> {
    fn parsed(workbook: &'workbook WorkbookSnapshot, options: CalculationOptions) -> Self {
        Self::parsed_cancellable(workbook, options, &|| false)
            .expect("non-cancellable parsing cannot be cancelled")
    }

    fn parsed_cancellable(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut engine = Self {
            workbook,
            options,
            asts: BTreeMap::new(),
            defined_name_asts: Vec::new(),
            dependencies: BTreeMap::new(),
            results: BTreeMap::new(),
            numeric_decimal_traces: BTreeMap::new(),
            retained_results: BTreeMap::new(),
            array_regions: collect_array_regions(workbook),
            column_extents: collect_column_extents(workbook),
            dynamic_spills: BTreeMap::new(),
            parse_failures: BTreeMap::new(),
            name_cycle_cells: BTreeSet::new(),
            name_limit_cells: BTreeSet::new(),
            dependency_limit_exceeded: false,
            cycle_cells: BTreeSet::new(),
            blocked_cells: BTreeSet::new(),
            evaluated_cell_count: 0,
        };
        engine.parse_all_cancellable(cancelled)?;
        if cancelled() {
            return Err(());
        }
        engine.classify_name_graphs();
        if cancelled() {
            return Err(());
        }
        Ok(engine)
    }

    pub(in crate::calculation) fn analyze(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
    ) -> Self {
        let mut engine = Self::parsed(workbook, options);
        engine.dependency_limit_exceeded = engine.exceeds_dependency_limit();
        engine
    }

    pub(in crate::calculation) fn evaluate(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
    ) -> Self {
        Self::evaluate_cancellable(workbook, options, &|| false)
            .expect("non-cancellable calculation cannot be cancelled")
    }

    pub(in crate::calculation) fn evaluate_cancellable(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut engine = Self::parsed_cancellable(workbook, options, cancelled)?;
        let (dependencies, dependency_limit_exceeded) =
            engine.dependencies_cancellable(cancelled)?;
        engine.dependency_limit_exceeded = dependency_limit_exceeded;
        if dependency_limit_exceeded {
            engine.dependencies = dependencies;
            return Ok(engine);
        }
        let dependencies = if engine.has_unresolved_dynamic_dependencies() {
            if !engine.evaluate_schedule_subset(&dependencies, None, cancelled) {
                return Err(());
            }
            let (resolved_dependencies, dependency_limit_exceeded) =
                engine.dependencies_cancellable(cancelled)?;
            engine.dependency_limit_exceeded = dependency_limit_exceeded;
            if dependency_limit_exceeded {
                engine.results.clear();
                engine.cycle_cells.clear();
                engine.blocked_cells.clear();
                engine.dependencies = resolved_dependencies;
                return Ok(engine);
            }
            resolved_dependencies
        } else {
            dependencies
        };
        if !engine.evaluate_schedule_subset(&dependencies, None, cancelled) {
            return Err(());
        }
        engine.dependencies = dependencies;
        Ok(engine)
    }

    pub(in crate::calculation) fn compiled(&self) -> CompiledWorkbook {
        CompiledWorkbook {
            asts: self.asts.clone(),
            defined_name_asts: self.defined_name_asts.clone(),
            dependencies: self.dependencies.clone(),
            dependency_rectangles: self.dependency_rectangles(),
            parse_failures: self.parse_failures.clone(),
            name_cycle_cells: self.name_cycle_cells.clone(),
            name_limit_cells: self.name_limit_cells.clone(),
            dependency_limit_exceeded: self.dependency_limit_exceeded,
            cycle_cells: self.cycle_cells.clone(),
            blocked_cells: self.blocked_cells.clone(),
            limits: self.options.limits(),
            incremental_safe: !self.has_unstable_incremental_dependencies(),
        }
    }

    pub(in crate::calculation) fn evaluate_compiled(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        compiled: &CompiledWorkbook,
        previous: Option<&CalculationSnapshot>,
        dirty: Option<&BTreeSet<CalculationCellId>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut engine = Self {
            workbook,
            options,
            asts: compiled.asts.clone(),
            defined_name_asts: compiled.defined_name_asts.clone(),
            dependencies: compiled.dependencies.clone(),
            results: BTreeMap::new(),
            numeric_decimal_traces: BTreeMap::new(),
            retained_results: BTreeMap::new(),
            array_regions: collect_array_regions(workbook),
            column_extents: collect_column_extents(workbook),
            dynamic_spills: BTreeMap::new(),
            parse_failures: compiled.parse_failures.clone(),
            name_cycle_cells: compiled.name_cycle_cells.clone(),
            name_limit_cells: compiled.name_limit_cells.clone(),
            dependency_limit_exceeded: compiled.dependency_limit_exceeded,
            cycle_cells: compiled.cycle_cells.clone(),
            blocked_cells: compiled.blocked_cells.clone(),
            evaluated_cell_count: 0,
        };
        let internal_dirty = dirty.map(|cells| {
            cells
                .iter()
                .filter_map(|cell| public_to_internal(workbook, *cell))
                .collect::<BTreeSet<_>>()
        });
        if let Some(previous) = previous {
            engine.seed_previous_results(previous, internal_dirty.as_ref());
        }
        if compiled.dependency_limit_exceeded {
            return Ok(engine);
        }
        if !engine.evaluate_schedule_subset(
            &compiled.dependencies,
            internal_dirty.as_ref(),
            &cancelled,
        ) {
            return Err(());
        }
        Ok(engine)
    }

    fn evaluate_schedule_subset(
        &mut self,
        dependencies: &DependencyGraph,
        dirty: Option<&BTreeSet<CellId>>,
        cancelled: &impl Fn() -> bool,
    ) -> bool {
        if dirty.is_none() {
            self.results.clear();
            self.numeric_decimal_traces.clear();
            self.retained_results.clear();
            self.dynamic_spills.clear();
            self.array_regions.retain(|region| !region.provisional);
        }
        let scheduled = schedule(dependencies);
        self.cycle_cells = scheduled.cycle_cells;
        self.blocked_cells = scheduled.blocked_cells;
        for cell in scheduled.order {
            if dirty.is_some_and(|cells| !cells.contains(&cell)) {
                continue;
            }
            if cancelled() {
                return false;
            }
            self.evaluated_cell_count += 1;
            self.evaluate_one(cell, cancelled);
        }
        !cancelled()
    }

    fn evaluate_one(&mut self, cell: CellId, cancelled: &impl Fn() -> bool) {
        let expr = self.asts.get(&cell).cloned();
        let budget = EvaluationBudget::default();
        let context = EvalContext::for_cancellable(cell, &budget, cancelled);
        let array_range = self.legacy_array_range(cell);
        if let Some(range) = array_range {
            let result = match expr {
                Some(_) if self.name_limit_cells.contains(&cell) => Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FormulaNestingDepth,
                )),
                Some(_) if self.name_cycle_cells.contains(&cell) => Err(ErrorKind::Unsupported),
                Some(expr) => self.eval_array_with_trace(context, &expr),
                None => Err(ErrorKind::Unsupported),
            };
            self.materialize_legacy_array(cell, range, result);
            return;
        }
        if let Some(declared_range) = self.dynamic_array_range(cell) {
            let result = match expr {
                Some(_) if self.name_limit_cells.contains(&cell) => Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FormulaNestingDepth,
                )),
                Some(_) if self.name_cycle_cells.contains(&cell) => Err(ErrorKind::Unsupported),
                Some(expr) => self.eval_array_with_trace(context, &expr),
                None => Err(ErrorKind::Unsupported),
            };
            self.materialize_dynamic_array(cell, declared_range, result);
            return;
        }
        let value = match expr {
            Some(_) if self.name_limit_cells.contains(&cell) => Value::Error(
                ErrorKind::ResourceLimit(CalculationLimitKind::FormulaNestingDepth),
            ),
            Some(_) if self.name_cycle_cells.contains(&cell) => {
                Value::Error(ErrorKind::Unsupported)
            }
            Some(expr) => {
                let evaluated = self.eval_scalar_with_trace(context, &expr);
                if let (Value::Number(_), Some(trace)) = (&evaluated.value, evaluated.decimal_trace)
                {
                    self.numeric_decimal_traces.insert(cell, trace);
                }
                match evaluated.value {
                    // Excel materializes a blank final formula result as numeric zero while
                    // retaining Blank during expression and function evaluation.
                    Value::Blank => Value::Number(0.0),
                    value => value,
                }
            }
            None => Value::Error(ErrorKind::Unsupported),
        };
        self.results.insert(cell, value);
    }

    pub(in crate::calculation) const fn evaluated_cell_count(&self) -> usize {
        self.evaluated_cell_count
    }

    pub(in crate::calculation) fn retained_result(
        &self,
        cell: CellId,
    ) -> Option<&CalculationCellResult> {
        self.retained_results.get(&cell)
    }

    fn parse_all_cancellable(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ()> {
        for defined_name in self.workbook.defined_names() {
            if cancelled() {
                return Err(());
            }
            self.defined_name_asts.push(
                parse_formula_with_limits(defined_name.formula().as_str(), self.options.limits())
                    .ok(),
            );
        }
        for (sheet_index, sheet) in self.workbook.sheets().iter().enumerate() {
            for cell in sheet.cells() {
                let CellContent::Formula(formula) = cell.content() else {
                    continue;
                };
                if cancelled() {
                    return Err(());
                }
                let id = (
                    sheet_index,
                    cell.address().row().get(),
                    cell.address().column().get(),
                );
                let Some(text) = formula.text() else {
                    continue;
                };
                match parse_formula_with_limits(text.as_str(), self.options.limits()) {
                    Ok(expr) => {
                        self.asts.insert(id, expr);
                    }
                    Err(error) => {
                        self.parse_failures.insert(id, error);
                    }
                }
            }
        }
        Ok(())
    }
}
