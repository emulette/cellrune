use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::dependency::CollectedDependencies;
use super::dependency_areas::AreaNodes;

/// Passes a full calculation makes at most when references known only from values form chains.
const MAX_DYNAMIC_REFERENCE_PASSES: u32 = 8;
use super::materialization::ArrayOwners;
use super::{
    CompiledWorkbook, Engine, EvalContext, EvaluationBudget, VolatileFormulas,
    collect_column_extents, collect_workbook_layout, public_to_internal,
};
use crate::calculation::graph::{DependencyGraph, Schedule, schedule_cancellable};
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::pipeline::formula_volatility;
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

    pub(super) fn parsed_cancellable(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut engine = Self::unparsed_cancellable(workbook, options, cancelled)?;
        engine.parse_all_cancellable(cancelled)?;
        engine.classify_name_graphs(cancelled)?;
        Ok(engine)
    }

    pub(super) fn unparsed_cancellable(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let (array_regions, column_extents) = collect_workbook_layout(workbook, cancelled)?;
        let table_topologies = super::dependency::workbook_table_topologies(workbook, cancelled)?;
        let engine = Self {
            workbook,
            previous: None,
            dirty: None,
            options,
            table_topologies,
            asts: Arc::new(BTreeMap::new()),
            defined_name_asts: Arc::new(Vec::new()),
            dependencies: Arc::new(DependencyGraph::default()),
            dependency_areas: Arc::new(AreaNodes::default()),
            results: std::collections::HashMap::new(),
            retained_results: BTreeMap::new(),
            static_array_owners: Arc::new(ArrayOwners::new(&array_regions)),
            array_regions,
            column_extents,
            dynamic_spills: BTreeMap::new(),
            parse_failures: Arc::new(BTreeMap::new()),
            name_cycle_cells: Arc::new(BTreeSet::new()),
            name_limit_cells: Arc::new(BTreeSet::new()),
            dependency_limit_cells: Arc::new(BTreeSet::new()),
            cycle_cells: Arc::new(BTreeSet::new()),
            blocked_cells: Arc::new(BTreeSet::new()),
            evaluated_cells: BTreeSet::new(),
            function_iterations: 0,
            reference_cells: 0,
            target_pending: None,
            lookup_indexes: std::cell::RefCell::default(),
            regex_cache: std::cell::RefCell::default(),
            formula_columns: std::cell::OnceCell::new(),
            read_formula_before_calculation: std::cell::Cell::new(false),
            graph_follows_values: true,
            unavailable_previous_reads: std::cell::RefCell::default(),
        };
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
        engine.dependency_limit_cells = Arc::new(engine.dependency_limit_cells_for_analysis());
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
        let mut collected = engine.dependencies_cancellable(cancelled)?;
        engine.dependency_limit_cells = Arc::new(collected.limited);
        if engine.has_unresolved_dynamic_dependencies(cancelled)? {
            // References known only from values: each pass orders formulas by the graph the
            // previous pass's values resolve, until a pass reads every formula after calculating
            // it.
            let undeclared_dynamic_arrays = engine.has_undeclared_dynamic_arrays(cancelled)?;
            // A pass resolves again only the formulas whose targets can follow values, unless the
            // array regions the other formulas' nodes were built with changed or the last
            // collection limited formulas.
            let value_dependent = engine.value_dependent_formulas(cancelled)?;
            let mut collected_regions = engine.array_regions.clone();
            let mut passes = 0;
            // The formulas the last pass read before calculating them, when the next pass may
            // evaluate only those and the formulas waiting for them.
            let mut stale_pass = None;
            loop {
                let scheduled = schedule_cancellable(&collected.graph, cancelled)?;
                let dirty = match &stale_pass {
                    Some(stale) if !undeclared_dynamic_arrays => {
                        engine.pass_dirty(&collected.graph, &scheduled, stale, cancelled)?
                    }
                    Some(_) | None => None,
                };
                let stale = engine.evaluate_pass(scheduled, dirty.as_ref(), cancelled)?;
                passes += 1;
                let ordered = stale.is_empty();
                if passes > 1 && (ordered || passes == MAX_DYNAMIC_REFERENCE_PASSES) {
                    // The graph is the one collected before this pass.
                    engine.graph_follows_values = false;
                    break;
                }
                let next = if engine.dependency_limit_cells.is_empty()
                    && engine.array_regions == collected_regions
                    && engine.recollect_formulas(
                        &mut collected.graph,
                        &mut collected.areas,
                        &value_dependent,
                        cancelled,
                    )? {
                    CollectedDependencies {
                        graph: std::mem::take(&mut collected.graph),
                        areas: std::mem::take(&mut collected.areas),
                        limited: BTreeSet::new(),
                    }
                } else {
                    engine.dependencies_cancellable(cancelled)?
                };
                collected_regions.clone_from(&engine.array_regions);
                // A first pass that read every formula in order already holds the results the
                // resolved graph gives, unless that graph limits or cycles other formulas, or an
                // undeclared spill, which each pass discovers again, could collide differently.
                let settled = passes == 1
                    && ordered
                    && !undeclared_dynamic_arrays
                    && next.limited == *engine.dependency_limit_cells
                    && {
                        let schedule = schedule_cancellable(&next.graph, cancelled)?;
                        schedule.cycle_cells == *engine.cycle_cells
                            && schedule.blocked_cells == *engine.blocked_cells
                    };
                // Formulas the collection limits differently read differently, so every formula
                // calculates again.
                let limits_changed = next.limited != *engine.dependency_limit_cells;
                engine.dependency_limit_cells = Arc::new(next.limited);
                collected.graph = next.graph;
                collected.areas = next.areas;
                if settled {
                    break;
                }
                stale_pass = (!limits_changed).then_some(stale);
            }
        } else {
            let scheduled = schedule_cancellable(&collected.graph, cancelled)?;
            engine.evaluate_pass(scheduled, None, cancelled)?;
        }
        engine.dependencies = Arc::new(collected.graph);
        engine.dependency_areas = Arc::new(collected.areas);
        Ok(engine)
    }

    pub(in crate::calculation) fn compiled(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<CompiledWorkbook, ()> {
        let dependency_targets = self.dependency_targets_cancellable(cancelled)?;
        let table_topologies = super::dependency::table_topologies(&dependency_targets, cancelled)?;
        let undeclared_dynamic_arrays = self.has_undeclared_dynamic_arrays(cancelled)?;
        let mut unstable_formulas = BTreeSet::new();
        for cell in self.asts.keys() {
            if self.formula_is_incrementally_unstable(*cell, cancelled)? {
                unstable_formulas.insert(*cell);
            }
        }
        let volatile_formulas = self.volatile_formulas(cancelled)?;
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
            dependency_areas: Arc::clone(&self.dependency_areas),
            dependency_targets: Arc::new(dependency_targets),
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
            dependency_limit_cells: Arc::clone(&self.dependency_limit_cells),
            cycle_cells: Arc::clone(&self.cycle_cells),
            blocked_cells: Arc::clone(&self.blocked_cells),
            impact_index,
            schedule,
            topological_rank,
            limits: self.options.limits(),
            undeclared_dynamic_arrays,
            unstable_formulas: Arc::new(unstable_formulas),
            graph_follows_values: self.graph_follows_values,
            volatile_formulas: Arc::new(volatile_formulas),
        })
    }

    /// Classifies every parsed formula by the volatile calculation inputs it reads.
    fn volatile_formulas(&self, cancelled: &impl Fn() -> bool) -> Result<VolatileFormulas, ()> {
        let mut formulas = VolatileFormulas::default();
        for (cell, parsed) in self.asts.iter() {
            if cancelled() {
                return Err(());
            }
            formulas.record(*cell, formula_volatility(self, cell.0, parsed.root()));
        }
        Ok(formulas)
    }

    /// Evaluates the formulas of `compiled`, or only those in `dirty`, in its order, and returns
    /// whether `compiled` no longer holds for the values: a formula read another before it was
    /// calculated, or a formula whose references depend on values resolves them to other targets
    /// than `compiled` was built with, so that its order, cycles, blocked and limited formulas,
    /// and the issues readers inherit may differ from a full calculation.
    pub(in crate::calculation) fn evaluate_compiled(
        workbook: &'workbook WorkbookSnapshot,
        options: CalculationOptions,
        compiled: &CompiledWorkbook,
        previous: Option<&'workbook CalculationSnapshot>,
        dirty: Option<&BTreeSet<CalculationCellId>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(Self, bool), ()> {
        let mut engine =
            Self::from_compiled(workbook, options, compiled, previous, dirty, cancelled)?;
        loop {
            let dirty = engine.dirty.clone();
            if engine.evaluate_compiled_schedule(compiled, dirty.as_ref(), cancelled)? {
                return Ok((engine, true));
            }
            // A pass that read the kept result of a formula with a capability issue evaluates
            // that formula too, so that its readers read the value a full calculation gives them.
            let Some(mut dirty) = dirty else {
                return Ok((engine, false));
            };
            let reread = engine.capability_issue_reads(&dirty, cancelled)?;
            if reread.is_empty() {
                return Ok((engine, false));
            }
            dirty.extend(reread);
            engine = Self::from_compiled(workbook, options, compiled, previous, None, cancelled)?;
            engine.dirty = Some(dirty);
        }
    }

    /// An engine over `workbook` sharing `compiled`'s formulas and dependency state, which reads
    /// the results of `previous` for formula cells outside `dirty`.
    pub(super) fn from_compiled(
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
            dependency_areas: Arc::clone(&compiled.dependency_areas),
            results: std::collections::HashMap::new(),
            retained_results: BTreeMap::new(),
            static_array_owners: Arc::new(ArrayOwners::new(&array_regions)),
            array_regions,
            column_extents,
            dynamic_spills: BTreeMap::new(),
            parse_failures: Arc::clone(&compiled.parse_failures),
            name_cycle_cells: Arc::clone(&compiled.name_cycle_cells),
            name_limit_cells: Arc::clone(&compiled.name_limit_cells),
            dependency_limit_cells: Arc::clone(&compiled.dependency_limit_cells),
            cycle_cells: Arc::clone(&compiled.cycle_cells),
            blocked_cells: Arc::clone(&compiled.blocked_cells),
            evaluated_cells: BTreeSet::new(),
            function_iterations: 0,
            reference_cells: 0,
            target_pending: None,
            lookup_indexes: std::cell::RefCell::default(),
            regex_cache: std::cell::RefCell::default(),
            formula_columns: std::cell::OnceCell::new(),
            read_formula_before_calculation: std::cell::Cell::new(false),
            graph_follows_values: true,
            unavailable_previous_reads: std::cell::RefCell::default(),
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
        engine.dirty = internal_dirty;
        Ok(engine)
    }

    /// Whether `cell`, a formula of `compiled` whose references depend on values, resolves them
    /// with the values of this pass to other targets than `compiled` holds for it.
    fn targets_moved(
        &self,
        compiled: &CompiledWorkbook,
        cell: CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        Ok(self.formula_dependency_targets(cell, cancelled)?.as_slice()
            != compiled.dependency_targets(cell).unwrap_or_default())
    }

    /// Evaluates the formulas of `compiled`, or only those in `dirty`, in its order, and returns
    /// whether `compiled` no longer holds for the values, stopping at the first formula that shows
    /// it: one that read another before it was calculated, or one whose references depend on
    /// values and moved. The values a formula's references depend on precede it in the order, so
    /// its targets are settled once it is evaluated.
    fn evaluate_compiled_schedule(
        &mut self,
        compiled: &CompiledWorkbook,
        dirty: Option<&BTreeSet<CellId>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        self.lookup_indexes.get_mut().clear();
        if dirty.is_none() {
            self.results.clear();
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
        let value_dependent = compiled.value_dependent_formulas();
        for cell in cells {
            if cancelled() {
                return Err(());
            }
            self.evaluated_cells.insert(cell);
            self.read_formula_before_calculation.set(false);
            self.evaluate_one(cell, cancelled)?;
            if self.read_formula_before_calculation.get()
                || (value_dependent.contains(&cell)
                    && self.targets_moved(compiled, cell, cancelled)?)
            {
                return Ok(true);
            }
        }
        if cancelled() {
            return Err(());
        }
        Ok(false)
    }

    /// Evaluates the formulas of `scheduled` in its order, or only those in `dirty`, whose
    /// earlier results it clears first so that reading one before it calculates is seen, and
    /// returns the formulas that read a formula before it was calculated.
    fn evaluate_pass(
        &mut self,
        scheduled: Schedule,
        dirty: Option<&BTreeSet<CellId>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BTreeSet<CellId>, ()> {
        self.lookup_indexes.get_mut().clear();
        match dirty {
            None => {
                self.results.clear();
                self.retained_results.clear();
                self.dynamic_spills.clear();
                self.array_regions.retain(|region| !region.provisional);
            }
            Some(dirty) => {
                for cell in dirty {
                    self.results.remove(cell);
                    self.retained_results.remove(cell);
                }
            }
        }
        self.cycle_cells = Arc::new(scheduled.cycle_cells);
        self.blocked_cells = Arc::new(scheduled.blocked_cells);
        let mut stale = BTreeSet::new();
        for cell in scheduled.order {
            if dirty.is_some_and(|cells| !cells.contains(&cell)) {
                continue;
            }
            if cancelled() {
                return Err(());
            }
            self.evaluated_cells.insert(cell);
            self.read_formula_before_calculation.set(false);
            self.evaluate_one(cell, cancelled)?;
            if self.read_formula_before_calculation.get() {
                stale.insert(cell);
            }
        }
        if cancelled() {
            return Err(());
        }
        Ok(stale)
    }

    pub(super) fn evaluate_one(
        &mut self,
        cell: CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let expr = self.asts.get(&cell).cloned();
        let budget = EvaluationBudget::default();
        let context = EvalContext::for_cancellable(cell, &budget, cancelled);
        let array_range = self.legacy_array_range(cell);
        if let Some(range) = array_range {
            let result = match expr {
                Some(_) if self.name_limit_cells.contains(&cell) => Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FormulaNestingDepth,
                )),
                Some(_) if self.dependency_limit_cells.contains(&cell) => Err(
                    ErrorKind::ResourceLimit(CalculationLimitKind::DependencyEdges),
                ),
                Some(_) if self.name_cycle_cells.contains(&cell) => Err(ErrorKind::Unsupported),
                Some(expr) => self.eval_final_array(
                    context.with_argument_owner(expr.argument_owner()),
                    expr.root(),
                ),
                None => Err(ErrorKind::Unsupported),
            };
            if self.target_dependencies_pending() {
                self.record_evaluation_work(&budget);
                return Ok(());
            }
            let outcome = self.materialize_legacy_array(cell, range, result, cancelled);
            self.record_evaluation_work(&budget);
            return outcome;
        }
        if let Some(declared_range) = self.dynamic_array_range(cell) {
            let result = match expr {
                Some(_) if self.name_limit_cells.contains(&cell) => Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FormulaNestingDepth,
                )),
                Some(_) if self.dependency_limit_cells.contains(&cell) => Err(
                    ErrorKind::ResourceLimit(CalculationLimitKind::DependencyEdges),
                ),
                Some(_) if self.name_cycle_cells.contains(&cell) => Err(ErrorKind::Unsupported),
                Some(expr) => self.eval_final_array(
                    context.with_argument_owner(expr.argument_owner()),
                    expr.root(),
                ),
                None => Err(ErrorKind::Unsupported),
            };
            if self.target_dependencies_pending() {
                self.record_evaluation_work(&budget);
                return Ok(());
            }
            let outcome = self.materialize_dynamic_array(cell, declared_range, result, cancelled);
            self.record_evaluation_work(&budget);
            return outcome;
        }
        let value = match expr {
            Some(_) if self.name_limit_cells.contains(&cell) => Value::Error(
                ErrorKind::ResourceLimit(CalculationLimitKind::FormulaNestingDepth),
            ),
            Some(_) if self.dependency_limit_cells.contains(&cell) => Value::Error(
                ErrorKind::ResourceLimit(CalculationLimitKind::DependencyEdges),
            ),
            Some(_) if self.name_cycle_cells.contains(&cell) => {
                Value::Error(ErrorKind::Unsupported)
            }
            Some(expr) => {
                let evaluated = self.eval_final_scalar(
                    context.with_argument_owner(expr.argument_owner()),
                    expr.root(),
                );
                if self.target_dependencies_pending() {
                    self.record_evaluation_work(&budget);
                    return Ok(());
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
        self.store_result(cell, value);
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
