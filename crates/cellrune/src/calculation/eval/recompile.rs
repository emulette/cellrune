//! Bringing a compiled workbook up to date with edited formula cells without compiling every
//! formula again.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::reference::cell_at;
use super::{CompiledWorkbook, DependencyImpactIndex, Engine, public_to_internal};
use crate::calculation::graph::schedule_cancellable;
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::pipeline::formula_volatility;
use crate::calculation::{CalculationCellId, CalculationOptions, CalculationSnapshot};
use crate::{CellContent, FormulaMetadata, MaterializedResultOrigin, WorkbookSnapshot};

/// A compiled workbook brought up to date with formula edits, and how many formulas that parsed.
pub(in crate::calculation) struct RecompiledWorkbook {
    pub(in crate::calculation) compiled: CompiledWorkbook,
    pub(in crate::calculation) parsed_formula_count: usize,
}

impl CompiledWorkbook {
    /// Returns this state updated for `workbook`, which differs from the workbook `previous` was
    /// calculated from only in the formulas of the `edited` cells, or `None` when the workbook
    /// must be compiled in full.
    ///
    /// Only the edited formulas are parsed, and only they and the formulas whose ranges cover a
    /// cell that became or stopped being a formula collect their dependencies again. A full
    /// compilation is needed when this state left formulas out at the dependency limit or cannot
    /// follow edits, when an edited cell is or was part of an array formula, and when the updated
    /// graph could reach `max_dependency_edges`: below it, no full compilation could leave a
    /// formula out either.
    pub(in crate::calculation) fn with_formula_edits(
        &self,
        workbook: &WorkbookSnapshot,
        options: CalculationOptions,
        previous: &CalculationSnapshot,
        edited: &BTreeSet<CalculationCellId>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<RecompiledWorkbook>, ()> {
        if !self.dependency_limit_cells.is_empty()
            || !self.follows_edits()
            || options.limits() != self.limits
        {
            return Ok(None);
        }
        let mut cells = BTreeSet::new();
        let mut appeared_or_removed = BTreeSet::new();
        for public in edited {
            if cancelled() {
                return Err(());
            }
            let Some(cell) = public_to_internal(workbook, *public) else {
                return Ok(None);
            };
            let formula =
                cell_at(&workbook.sheets()[cell.0], cell.1, cell.2).and_then(|source| match source
                    .content()
                {
                    CellContent::Formula(formula) => Some(formula),
                    CellContent::Literal(_) => None,
                });
            let in_array = self.static_array_regions.iter().any(|region| {
                region.rect.sheet == cell.0
                    && (region.rect.row_start..=region.rect.row_end).contains(&cell.1)
                    && (region.rect.col_start..=region.rect.col_end).contains(&cell.2)
            });
            if in_array
                || formula.is_some_and(|formula| {
                    !matches!(
                        formula.metadata(),
                        FormulaMetadata::Normal | FormulaMetadata::Shared { .. }
                    )
                })
                || previous
                    .materialized_cell(*public)
                    .is_some_and(|materialized| {
                        materialized.origin() != MaterializedResultOrigin::DirectFormula
                    })
            {
                return Ok(None);
            }
            if previous.cell(*public).is_some() != formula.is_some() {
                appeared_or_removed.insert(cell);
            }
            cells.insert(cell);
        }

        let mut engine = Engine::from_compiled(
            workbook,
            options,
            self,
            Some(previous),
            Some(edited),
            cancelled,
        )?;
        let mut parsed_formula_count = 0;
        for cell in &cells {
            if cancelled() {
                return Err(());
            }
            Arc::make_mut(&mut engine.asts).remove(cell);
            Arc::make_mut(&mut engine.parse_failures).remove(cell);
            if let Some(text) = engine.cell_formula_text(*cell) {
                parsed_formula_count += 1;
                match parse_formula_with_limits(text, options.limits()) {
                    Ok(parsed) => {
                        Arc::make_mut(&mut engine.asts).insert(*cell, Arc::new(parsed));
                    }
                    Err(error) => {
                        Arc::make_mut(&mut engine.parse_failures).insert(*cell, error);
                    }
                }
            }
            engine.classify_name_graph(*cell, cancelled)?;
        }

        // The edited formulas collect their dependencies again, and so does every formula whose
        // references cover a cell that became or stopped being a formula: the areas and cells
        // it waits for changed with that cell.
        let mut collected = cells
            .iter()
            .copied()
            .filter(|cell| engine.asts.contains_key(cell))
            .collect::<BTreeSet<_>>();
        let mut charge = || if cancelled() { Err(()) } else { Ok(()) };
        let mut areas = (*self.dependency_areas).clone();
        for cell in &appeared_or_removed {
            for formula in self.impact_index.formulas_for_cell(*cell, &mut charge)? {
                if engine.asts.contains_key(&formula) {
                    collected.insert(formula);
                }
            }
            areas.forget(*cell);
        }
        let mut targets = (*self.dependency_targets).clone();
        let mut graph = (*self.dependencies).clone();
        for cell in &cells {
            if !engine.asts.contains_key(cell) {
                targets.remove(cell);
                graph.remove(*cell);
            }
        }
        for cell in &collected {
            if cancelled() {
                return Err(());
            }
            if cells.contains(cell) {
                targets.insert(*cell, engine.formula_dependency_targets(*cell, cancelled)?);
            }
            let nodes = if engine.name_cycle_cells.contains(cell)
                || engine.name_limit_cells.contains(cell)
            {
                Vec::new()
            } else {
                let formula_targets = targets.get(cell).map_or(&[][..], Vec::as_slice);
                engine.dependency_nodes(formula_targets, &mut graph, &mut areas, cancelled)?
            };
            graph.insert(*cell, nodes);
        }
        // The areas of replaced ranges, and of blocks forgotten around a cell that appeared or
        // disappeared, would otherwise stay in the graph, so every later recompilation and
        // schedule would grow with every area ever built and the edge count with them.
        let renumbered = graph.retain_referenced_areas();
        areas.renumber(&renumbered);
        if graph.edge_count() > options.limits().max_dependency_edges() {
            return Ok(None);
        }

        let mut unstable_formulas = (*self.unstable_formulas).clone();
        for cell in &cells {
            unstable_formulas.remove(cell);
        }
        for cell in &collected {
            unstable_formulas.remove(cell);
            if engine.formula_is_incrementally_unstable(*cell, cancelled)? {
                unstable_formulas.insert(*cell);
            }
        }
        let mut volatile_formulas = (*self.volatile_formulas).clone();
        volatile_formulas.forget(&cells);
        for cell in &cells {
            if let Some(parsed) = engine.asts.get(cell) {
                volatile_formulas.record(*cell, formula_volatility(&engine, cell.0, parsed.root()));
            }
        }
        let table_topologies = super::dependency::table_topologies(&targets, cancelled)?;
        let schedule = schedule_cancellable(&graph, cancelled)?;
        let topological_rank = schedule
            .order
            .iter()
            .enumerate()
            .map(|(rank, cell)| (*cell, rank))
            .collect();
        let impact_index =
            DependencyImpactIndex::build(&targets, &graph, &self.static_array_regions, cancelled)?;
        Ok(Some(RecompiledWorkbook {
            compiled: Self {
                asts: Arc::clone(&engine.asts),
                defined_name_asts: Arc::clone(&self.defined_name_asts),
                dependencies: Arc::new(graph),
                dependency_areas: Arc::new(areas),
                dependency_targets: Arc::new(targets),
                static_array_regions: Arc::clone(&self.static_array_regions),
                table_topologies,
                parse_failures: Arc::clone(&engine.parse_failures),
                name_cycle_cells: Arc::clone(&engine.name_cycle_cells),
                name_limit_cells: Arc::clone(&engine.name_limit_cells),
                dependency_limit_cells: Arc::clone(&self.dependency_limit_cells),
                cycle_cells: Arc::new(schedule.cycle_cells.clone()),
                blocked_cells: Arc::new(schedule.blocked_cells.clone()),
                impact_index,
                schedule,
                topological_rank,
                limits: self.limits,
                undeclared_dynamic_arrays: self.undeclared_dynamic_arrays,
                unstable_formulas: Arc::new(unstable_formulas),
                graph_follows_values: self.graph_follows_values,
                volatile_formulas: Arc::new(volatile_formulas),
            },
            parsed_formula_count,
        }))
    }
}
