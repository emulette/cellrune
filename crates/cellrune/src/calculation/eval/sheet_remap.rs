//! A compiled workbook carried across a change of sheet order: a sheet deleted, moved, or added
//! moves the calculation index of every sheet after it, and with it every compiled cell.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{CellId, CompiledWorkbook, DependencyImpactIndex, VolatileFormulas};
use crate::calculation::graph::Schedule;

impl CompiledWorkbook {
    /// The compiled state with every sheet at index `i` moved to `sheets[i]`, its cells dropped
    /// where that is `None`, and without the dependency targets of `recompiled`, formulas the
    /// next recalculation compiles again. Returns `None` when it cannot follow the move: another
    /// formula reads across sheets, whose members a new order changes, or reads a sheet the move
    /// drops, an array formula is on such a sheet, or the state already needs compiling again.
    pub(in crate::calculation) fn with_sheets_moved(
        &self,
        sheets: &[Option<usize>],
        recompiled: &BTreeSet<CellId>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Self>, ()> {
        if !self.incremental_safe() || !self.dependency_limit_cells.is_empty() {
            return Ok(None);
        }
        let sheet = |index: usize| sheets.get(index).copied().flatten();
        let cell = |cell: &CellId| Some((sheet(cell.0)?, cell.1, cell.2));
        let cells =
            |cells: &BTreeSet<CellId>| cells.iter().filter_map(cell).collect::<BTreeSet<_>>();
        let mut dependency_targets = BTreeMap::new();
        for (formula, targets) in self.dependency_targets.iter() {
            if cancelled() {
                return Err(());
            }
            let Some(moved) = cell(formula) else {
                continue;
            };
            if recompiled.contains(formula) {
                continue;
            }
            let mut kept = Vec::with_capacity(targets.len());
            for target in targets {
                match target.with_sheets(&sheet) {
                    Some(target) => kept.push(target),
                    None => return Ok(None),
                }
            }
            dependency_targets.insert(moved, kept);
        }
        let dependencies = self.dependencies.with_sheets(&sheet);
        let mut static_array_regions = Vec::with_capacity(self.static_array_regions.len());
        for region in self.static_array_regions.iter() {
            let Some(anchor) = cell(&region.anchor) else {
                return Ok(None);
            };
            let mut moved = *region;
            moved.anchor = anchor;
            moved.rect.sheet = anchor.0;
            static_array_regions.push(moved);
        }
        let impact_index = DependencyImpactIndex::build(
            &dependency_targets,
            &dependencies,
            &static_array_regions,
            cancelled,
        )?;
        let schedule = Schedule {
            order: self.schedule.order.iter().filter_map(cell).collect(),
            cycle_cells: cells(&self.schedule.cycle_cells),
            blocked_cells: cells(&self.schedule.blocked_cells),
        };
        let topological_rank = schedule
            .order
            .iter()
            .enumerate()
            .map(|(rank, cell)| (*cell, rank))
            .collect();
        let list = |cells: &[CellId]| cells.iter().filter_map(cell).collect::<Vec<_>>();
        Ok(Some(Self {
            asts: Arc::new(
                self.asts
                    .iter()
                    .filter_map(|(formula, parsed)| Some((cell(formula)?, Arc::clone(parsed))))
                    .collect(),
            ),
            defined_name_asts: Arc::clone(&self.defined_name_asts),
            dependency_areas: Arc::new(self.dependency_areas.with_sheets(&sheet)),
            dependencies: Arc::new(dependencies),
            dependency_targets: Arc::new(dependency_targets),
            static_array_regions: Arc::new(static_array_regions),
            table_topologies: self.table_topologies.clone(),
            parse_failures: Arc::new(
                self.parse_failures
                    .iter()
                    .filter_map(|(formula, error)| Some((cell(formula)?, error.clone())))
                    .collect(),
            ),
            name_cycle_cells: Arc::new(cells(&self.name_cycle_cells)),
            name_limit_cells: Arc::new(cells(&self.name_limit_cells)),
            dependency_limit_cells: Arc::clone(&self.dependency_limit_cells),
            cycle_cells: Arc::new(cells(&self.cycle_cells)),
            blocked_cells: Arc::new(cells(&self.blocked_cells)),
            impact_index,
            schedule,
            topological_rank,
            limits: self.limits,
            undeclared_dynamic_arrays: self.undeclared_dynamic_arrays,
            unstable_formulas: Arc::new(cells(&self.unstable_formulas)),
            volatile_formulas: Arc::new(VolatileFormulas {
                today: list(&self.volatile_formulas.today),
                now: list(&self.volatile_formulas.now),
                random: list(&self.volatile_formulas.random),
                number_formats: list(&self.volatile_formulas.number_formats),
            }),
        }))
    }
}
