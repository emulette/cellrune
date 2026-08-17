use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    CompiledWorkbook, Engine, EvalContext, EvaluationBudget, collect_column_extents,
    collect_workbook_layout, public_to_internal,
};
use crate::calculation::graph::{DependencyGraph, schedule_cancellable};
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
        let (array_regions, column_extents) = collect_workbook_layout(workbook, cancelled)?;
        let table_topologies = super::dependency::workbook_table_topologies(workbook, cancelled)?;
        let mut engine = Self {
            workbook,
            previous: None,
            dirty: None,
            options,
            table_topologies,
            asts: Arc::new(BTreeMap::new()),
            defined_name_asts: Arc::new(Vec::new()),
            dependencies: Arc::new(BTreeMap::new()),
            results: BTreeMap::new(),
            numeric_decimal_traces: BTreeMap::new(),
            retained_results: BTreeMap::new(),
            array_regions,
            column_extents,
            dynamic_spills: BTreeMap::new(),
            parse_failures: Arc::new(BTreeMap::new()),
            name_cycle_cells: Arc::new(BTreeSet::new()),
            name_limit_cells: Arc::new(BTreeSet::new()),
            dependency_limit_exceeded: false,
            cycle_cells: Arc::new(BTreeSet::new()),
            blocked_cells: Arc::new(BTreeSet::new()),
            evaluated_cells: BTreeSet::new(),
            function_iterations: 0,
            reference_cells: 0,
        };
        engine.parse_all_cancellable(cancelled)?;
        if cancelled() {
            return Err(());
        }
        engine.classify_name_graphs(cancelled)?;
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
            engine.dependencies = Arc::new(dependencies);
            return Ok(engine);
        }
        let dependencies = if engine.has_unresolved_dynamic_dependencies(cancelled)? {
            if !engine.evaluate_schedule_subset(&dependencies, None, cancelled) {
                return Err(());
            }
            let (resolved_dependencies, dependency_limit_exceeded) =
                engine.dependencies_cancellable(cancelled)?;
            engine.dependency_limit_exceeded = dependency_limit_exceeded;
            if dependency_limit_exceeded {
                engine.results.clear();
                Arc::make_mut(&mut engine.cycle_cells).clear();
                Arc::make_mut(&mut engine.blocked_cells).clear();
                engine.dependencies = Arc::new(resolved_dependencies);
                return Ok(engine);
            }
            resolved_dependencies
        } else {
            dependencies
        };
        if !engine.evaluate_schedule_subset(&dependencies, None, cancelled) {
            return Err(());
        }
        engine.dependencies = Arc::new(dependencies);
        Ok(engine)
    }

    pub(in crate::calculation) fn compiled(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<CompiledWorkbook, ()> {
        let dependency_targets = self.dependency_targets_cancellable(cancelled)?;
        let table_topologies = super::dependency::table_topologies(&dependency_targets, cancelled)?;
        let incremental_safe = !self.has_unstable_incremental_dependencies(cancelled)?;
        let schedule = schedule_cancellable(&self.dependencies, cancelled)?;
        let topological_rank = schedule
            .order
            .iter()
            .enumerate()
            .map(|(rank, cell)| (*cell, rank))
            .collect();
        let impact_index = super::DependencyImpactIndex::build(
            &dependency_targets,
            &self.dependencies,
            &self.array_regions,
            cancelled,
        )?;
        Ok(CompiledWorkbook {
            asts: Arc::clone(&self.asts),
            defined_name_asts: Arc::clone(&self.defined_name_asts),
            dependencies: Arc::clone(&self.dependencies),
            static_array_regions: Arc::new(
                self.array_regions
                    .iter()
                    .filter(|region| !region.provisional)
                    .copied()
                    .collect(),
            ),
            table_topologies,
            parse_failures: Arc::clone(&self.parse_failures),
            name_cycle_cells: Arc::clone(&self.name_cycle_cells),
            name_limit_cells: Arc::clone(&self.name_limit_cells),
            dependency_limit_exceeded: self.dependency_limit_exceeded,
            cycle_cells: Arc::clone(&self.cycle_cells),
            blocked_cells: Arc::clone(&self.blocked_cells),
            impact_index,
            schedule,
            topological_rank,
            limits: self.options.limits(),
            incremental_safe,
        })
    }

    pub(in crate::calculation) fn evaluate_compiled(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        compiled: &CompiledWorkbook,
        previous: Option<&'workbook CalculationSnapshot>,
        dirty: Option<&BTreeSet<CalculationCellId>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let array_regions = compiled.static_array_regions.as_ref().clone();
        let column_extents = collect_column_extents(workbook, cancelled)?;
        let table_topologies = super::dependency::workbook_table_topologies(workbook, cancelled)?;
        let mut engine = Self {
            workbook,
            previous,
            dirty: None,
            options,
            table_topologies,
            asts: Arc::clone(&compiled.asts),
            defined_name_asts: Arc::clone(&compiled.defined_name_asts),
            dependencies: Arc::clone(&compiled.dependencies),
            results: BTreeMap::new(),
            numeric_decimal_traces: BTreeMap::new(),
            retained_results: BTreeMap::new(),
            array_regions,
            column_extents,
            dynamic_spills: BTreeMap::new(),
            parse_failures: Arc::clone(&compiled.parse_failures),
            name_cycle_cells: Arc::clone(&compiled.name_cycle_cells),
            name_limit_cells: Arc::clone(&compiled.name_limit_cells),
            dependency_limit_exceeded: compiled.dependency_limit_exceeded,
            cycle_cells: Arc::clone(&compiled.cycle_cells),
            blocked_cells: Arc::clone(&compiled.blocked_cells),
            evaluated_cells: BTreeSet::new(),
            function_iterations: 0,
            reference_cells: 0,
        };
        let internal_dirty = match dirty {
            Some(cells) => {
                let mut internal = BTreeSet::new();
                for cell in cells {
                    if cancelled() {
                        return Err(());
                    }
                    if let Some(cell) = public_to_internal(workbook, *cell) {
                        internal.insert(cell);
                    }
                }
                Some(internal)
            }
            None => None,
        };
        engine.previous = previous;
        engine.dirty = internal_dirty.clone();
        if compiled.dependency_limit_exceeded {
            return Ok(engine);
        }
        if !engine.evaluate_compiled_schedule(compiled, internal_dirty.as_ref(), cancelled) {
            return Err(());
        }
        Ok(engine)
    }

    fn evaluate_compiled_schedule(
        &mut self,
        compiled: &CompiledWorkbook,
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
        let cells = if let Some(dirty) = dirty {
            let mut cells = dirty
                .iter()
                .filter_map(|cell| compiled.topological_rank(*cell).map(|rank| (rank, *cell)))
                .collect::<Vec<_>>();
            cells.sort_unstable();
            cells.into_iter().map(|(_, cell)| cell).collect()
        } else {
            compiled.schedule().order.clone()
        };
        for cell in cells {
            if cancelled() {
                return false;
            }
            self.evaluated_cells.insert(cell);
            if self.evaluate_one(cell, cancelled).is_err() {
                return false;
            }
        }
        !cancelled()
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
        let Ok(scheduled) = schedule_cancellable(dependencies, cancelled) else {
            return false;
        };
        self.cycle_cells = Arc::new(scheduled.cycle_cells);
        self.blocked_cells = Arc::new(scheduled.blocked_cells);
        for cell in scheduled.order {
            if dirty.is_some_and(|cells| !cells.contains(&cell)) {
                continue;
            }
            if cancelled() {
                return false;
            }
            self.evaluated_cells.insert(cell);
            if self.evaluate_one(cell, cancelled).is_err() {
                return false;
            }
        }
        !cancelled()
    }

    fn evaluate_one(&mut self, cell: CellId, cancelled: &impl Fn() -> bool) -> Result<(), ()> {
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
                Some(expr) => self.eval_final_array_with_trace(context, expr.root()),
                None => Err(ErrorKind::Unsupported),
            };
            let outcome = self.materialize_legacy_array(cell, range, result, cancelled);
            self.record_evaluation_work(&budget);
            return outcome;
        }
        if let Some(declared_range) = self.dynamic_array_range(cell) {
            let result = match expr {
                Some(_) if self.name_limit_cells.contains(&cell) => Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FormulaNestingDepth,
                )),
                Some(_) if self.name_cycle_cells.contains(&cell) => Err(ErrorKind::Unsupported),
                Some(expr) => self.eval_final_array_with_trace(context, expr.root()),
                None => Err(ErrorKind::Unsupported),
            };
            let outcome = self.materialize_dynamic_array(cell, declared_range, result, cancelled);
            self.record_evaluation_work(&budget);
            return outcome;
        }
        let value = match expr {
            Some(_) if self.name_limit_cells.contains(&cell) => Value::Error(
                ErrorKind::ResourceLimit(CalculationLimitKind::FormulaNestingDepth),
            ),
            Some(_) if self.name_cycle_cells.contains(&cell) => {
                Value::Error(ErrorKind::Unsupported)
            }
            Some(expr) => {
                let evaluated = self.eval_final_scalar_with_trace(context, expr.root());
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
        self.record_evaluation_work(&budget);
        Ok(())
    }

    fn record_evaluation_work(&mut self, budget: &EvaluationBudget) {
        self.function_iterations = self
            .function_iterations
            .saturating_add(budget.function_iterations());
        self.reference_cells = self
            .reference_cells
            .saturating_add(budget.reference_cells());
    }

    pub(in crate::calculation) fn evaluated_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.evaluated_cells.iter().copied()
    }

    pub(in crate::calculation) const fn function_iterations(&self) -> u64 {
        self.function_iterations
    }

    pub(in crate::calculation) const fn reference_cells(&self) -> u64 {
        self.reference_cells
    }

    pub(in crate::calculation) fn retained_result(
        &self,
        cell: CellId,
    ) -> Option<&CalculationCellResult> {
        self.retained_results.get(&cell).or_else(|| {
            let previous = self.previous?;
            self.previous_materialized(cell)?;
            let public = super::internal_to_public(self.workbook, cell)?;
            previous.cell(public)
        })
    }

    fn parse_all_cancellable(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ()> {
        for defined_name in self.workbook.defined_names() {
            if cancelled() {
                return Err(());
            }
            Arc::make_mut(&mut self.defined_name_asts).push(
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
                        Arc::make_mut(&mut self.asts).insert(id, Arc::new(expr));
                    }
                    Err(error) => {
                        Arc::make_mut(&mut self.parse_failures).insert(id, error);
                    }
                }
            }
        }
        Ok(())
    }
}
