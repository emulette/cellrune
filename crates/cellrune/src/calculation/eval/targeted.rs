use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::reference::cell_at;
use super::{DependencyTarget, Engine, public_to_internal};
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::runtime::CellId;
use crate::calculation::targeted::source_provenance;
use crate::calculation::value::{ErrorKind, Value};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationIssue, CalculationIssueCode,
    CalculationOptions, CancellationToken, CellContent, TargetCalculationError,
    TargetCalculationErrorCode, TargetCalculationLimits, TargetCalculationResult,
    WorkbookFingerprint, WorkbookSnapshot,
};

struct Frame {
    cell: CellId,
    dependencies: Vec<CellId>,
    next: usize,
}

struct Planner {
    formulas: Vec<BTreeSet<(u32, u32)>>,
    loaded: BTreeSet<CellId>,
    names: BTreeSet<usize>,
    done: BTreeSet<CellId>,
    edges: usize,
    attempts: usize,
    limits: TargetCalculationLimits,
}

fn cancelled_error() -> TargetCalculationError {
    TargetCalculationError::new(TargetCalculationErrorCode::Cancelled)
}

impl<'workbook> Engine<'workbook> {
    pub(in crate::calculation) fn calculate_targets(
        workbook: &'workbook WorkbookSnapshot,
        targets: &BTreeSet<CalculationCellId>,
        options: CalculationOptions,
        limits: TargetCalculationLimits,
        cancellation: &CancellationToken,
    ) -> Result<TargetCalculationResult, TargetCalculationError> {
        let cancelled = || cancellation.is_cancelled();
        let mut engine = Self::unparsed_cancellable(workbook, options, &cancelled)
            .map_err(|()| cancelled_error())?;
        engine.target_pending = Some(std::cell::RefCell::new(BTreeSet::new()));
        Arc::make_mut(&mut engine.defined_name_asts)
            .resize_with(workbook.defined_names().len(), || None);
        let mut formulas = Vec::with_capacity(workbook.sheets().len());
        for sheet in workbook.sheets() {
            let mut entries = BTreeSet::new();
            for cell in sheet.cells() {
                if cancelled() {
                    return Err(cancelled_error());
                }
                if matches!(cell.content(), CellContent::Formula(_)) {
                    entries.insert((cell.address().row().get(), cell.address().column().get()));
                }
            }
            formulas.push(entries);
        }
        let mut planner = Planner {
            formulas,
            loaded: BTreeSet::new(),
            names: BTreeSet::new(),
            done: BTreeSet::new(),
            edges: 0,
            attempts: 0,
            limits,
        };
        for pass in 0..2 {
            for target in targets {
                if cancelled() {
                    return Err(cancelled_error());
                }
                if let Some(cell) = public_to_internal(workbook, *target)
                    && let Some(owner) = planner.owner(&engine, cell)
                {
                    planner.evaluate(&mut engine, owner, &cancelled)?;
                }
            }
            if pass == 1 || !engine.array_regions.iter().any(|region| region.provisional) {
                break;
            }
            // Resolve references to newly discovered spill followers on the captured scope,
            // matching the full evaluator's second scheduling pass without parsing unrelated formulas.
            planner.done.clear();
            planner.edges = 0;
            engine.dependencies = Arc::new(BTreeMap::new());
            engine.results.clear();
            engine.numeric_decimal_traces.clear();
            engine.retained_results.clear();
            engine.dynamic_spills.clear();
            engine.cycle_cells = Arc::new(BTreeSet::new());
            engine.blocked_cells = Arc::new(BTreeSet::new());
        }
        // Final SCC classification includes cross-edges into already visited branches.
        let schedule =
            crate::calculation::graph::schedule_cancellable(&engine.dependencies, &cancelled)
                .map_err(|()| cancelled_error())?;
        engine.cycle_cells = Arc::new(schedule.cycle_cells);
        engine.blocked_cells = Arc::new(schedule.blocked_cells);
        let mut cells = BTreeMap::new();
        for target in targets {
            if cancelled() {
                return Err(cancelled_error());
            }
            if let Some(cell) = public_to_internal(workbook, *target) {
                let owner = planner.owner(&engine, cell);
                let result = if let Some(owner) = owner {
                    let owner_result =
                        crate::calculation::pipeline::target_result(workbook, &engine, owner);
                    match owner_result {
                        CalculationCellResult::Unavailable(_) => owner_result,
                        _ if owner == cell => owner_result,
                        _ => CalculationCellResult::Value(
                            crate::calculation::convert::cell_from_value(engine.cell_value(cell)),
                        ),
                    }
                } else {
                    CalculationCellResult::Value(crate::calculation::convert::cell_from_value(
                        engine.cell_value(cell),
                    ))
                };
                cells.insert(*target, result);
            }
        }
        let fingerprint = workbook
            .semantic_fingerprint_cancellable(&cancelled)
            .map_err(|()| cancelled_error())?;
        Ok(TargetCalculationResult {
            cells,
            source_revision: workbook.semantic_revision(),
            source_fingerprint: WorkbookFingerprint::current(fingerprint),
            provenance: source_provenance(workbook),
            options,
            limits,
            evaluated_count: planner.attempts,
            parsed_formula_count: planner.loaded.len(),
            reused_count: 0,
        })
    }
}

impl Planner {
    fn owner(&self, engine: &Engine<'_>, cell: CellId) -> Option<CellId> {
        engine.array_owner(cell).or_else(|| {
            self.formulas
                .get(cell.0)
                .filter(|formulas| formulas.contains(&(cell.1, cell.2)))
                .map(|_| cell)
        })
    }

    fn prepare(
        &mut self,
        engine: &mut Engine<'_>,
        cell: CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Frame, TargetCalculationError> {
        if self.loaded.insert(cell) {
            if self.loaded.len() > self.limits.max_evaluated_cells() {
                return Err(TargetCalculationError::new(
                    TargetCalculationErrorCode::EvaluationLimitExceeded,
                ));
            }
            if let Some(source) = cell_at(&engine.workbook.sheets()[cell.0], cell.1, cell.2)
                && let CellContent::Formula(formula) = source.content()
                && let Some(text) = formula.text()
            {
                match parse_formula_with_limits(text.as_str(), engine.options.limits()) {
                    Ok(parsed) => {
                        Arc::make_mut(&mut engine.asts).insert(cell, Arc::new(parsed));
                    }
                    Err(error) => {
                        Arc::make_mut(&mut engine.parse_failures).insert(cell, error);
                    }
                }
            }
            engine
                .prepare_target_names(cell, &mut self.names, cancelled)
                .map_err(|()| cancelled_error())?;
        }
        let dependencies = self.dependencies(engine, cell, cancelled)?;
        Ok(Frame {
            cell,
            dependencies,
            next: 0,
        })
    }

    fn dependencies(
        &mut self,
        engine: &mut Engine<'_>,
        cell: CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<CellId>, TargetCalculationError> {
        let targets = engine
            .target_dependencies(cell, cancelled)
            .map_err(|()| cancelled_error())?;
        let mut cells = engine
            .dependencies
            .get(&cell)
            .into_iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        for target in targets {
            if cancelled() {
                return Err(cancelled_error());
            }
            match target {
                DependencyTarget::Cell(cell) | DependencyTarget::SpillAnchor(cell) => {
                    if let Some(owner) = self.owner(engine, cell) {
                        cells.insert(owner);
                    }
                }
                DependencyTarget::TableIdentity(_) | DependencyTarget::FormulaContent(_) => {}
                DependencyTarget::Area(span) => {
                    for rect in span.rects() {
                        for &(row, column) in self.formulas[rect.sheet]
                            .range((rect.row_start, 0)..=(rect.row_end, u32::MAX))
                        {
                            if cancelled() {
                                return Err(cancelled_error());
                            }
                            if column >= rect.col_start && column <= rect.col_end {
                                cells.insert(
                                    self.owner(engine, (rect.sheet, row, column))
                                        .unwrap_or((rect.sheet, row, column)),
                                );
                                if cells.len() as u64
                                    > engine.options.limits().max_dependency_edges()
                                {
                                    break;
                                }
                            }
                        }
                        for region in &engine.array_regions {
                            if cancelled() {
                                return Err(cancelled_error());
                            }
                            if rect.sheet == region.rect.sheet
                                && rect.row_start <= region.rect.row_end
                                && rect.row_end >= region.rect.row_start
                                && rect.col_start <= region.rect.col_end
                                && rect.col_end >= region.rect.col_start
                            {
                                cells.insert(region.anchor);
                            }
                        }
                    }
                }
            }
            if cells.len() as u64 > engine.options.limits().max_dependency_edges() {
                break;
            }
        }
        let previous = engine.dependencies.get(&cell).map_or(0, Vec::len);
        let total = self
            .edges
            .saturating_sub(previous)
            .saturating_add(cells.len());
        if total as u64 > engine.options.limits().max_dependency_edges() {
            engine.retained_results.insert(
                cell,
                CalculationCellResult::Unavailable(CalculationIssue::new(
                    CalculationIssueCode::ResourceLimitExceeded,
                    Some(
                        crate::calculation::CalculationLimitKind::DependencyEdges
                            .detail()
                            .to_owned(),
                    ),
                )),
            );
            engine.results.insert(
                cell,
                Value::Error(ErrorKind::ResourceLimit(
                    crate::calculation::CalculationLimitKind::DependencyEdges,
                )),
            );
            cells.clear();
        } else {
            self.edges = total;
        }
        let dependencies = cells.into_iter().collect::<Vec<_>>();
        Arc::make_mut(&mut engine.dependencies).insert(cell, dependencies.clone());
        Ok(dependencies)
    }

    fn evaluate(
        &mut self,
        engine: &mut Engine<'_>,
        root: CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), TargetCalculationError> {
        if self.done.contains(&root) {
            return Ok(());
        }
        let mut stack = vec![self.prepare(engine, root, cancelled)?];
        let mut active = BTreeMap::from([(root, 0_usize)]);
        while let Some(frame) = stack.last_mut() {
            if cancelled() {
                return Err(cancelled_error());
            }
            let cell = frame.cell;
            if let Some(&dependency) = frame.dependencies.get(frame.next) {
                frame.next += 1;
                if let Some(&start) = active.get(&dependency) {
                    for entry in &stack[start..] {
                        Arc::make_mut(&mut engine.cycle_cells).insert(entry.cell);
                    }
                } else if !self.done.contains(&dependency) {
                    let frame = self.prepare(engine, dependency, cancelled)?;
                    active.insert(dependency, stack.len());
                    stack.push(frame);
                }
                continue;
            }
            if !engine.cycle_cells.contains(&cell) && !engine.retained_results.contains_key(&cell) {
                // Dynamic reference selectors may only resolve after their input formulas finish.
                let refreshed = self.dependencies(engine, cell, cancelled)?;
                let known = frame.dependencies.iter().copied().collect::<BTreeSet<_>>();
                let added = refreshed
                    .into_iter()
                    .filter(|dependency| !known.contains(dependency))
                    .collect::<Vec<_>>();
                if !added.is_empty() {
                    frame.dependencies.extend(added);
                    continue;
                }
                if engine.retained_results.contains_key(&cell) {
                    // A refreshed dynamic scope may exhaust the dependency budget.
                } else if frame.dependencies.iter().any(|dependency| {
                    engine.cycle_cells.contains(dependency)
                        || engine.blocked_cells.contains(dependency)
                }) {
                    Arc::make_mut(&mut engine.blocked_cells).insert(cell);
                } else {
                    if self.attempts >= self.limits.max_evaluated_cells() {
                        return Err(TargetCalculationError::new(
                            TargetCalculationErrorCode::EvaluationLimitExceeded,
                        ));
                    }
                    self.attempts += 1;
                    if let Some(pending) = &engine.target_pending {
                        pending.borrow_mut().clear();
                    }
                    engine
                        .evaluate_one(cell, cancelled)
                        .map_err(|()| cancelled_error())?;
                    let pending = engine
                        .target_pending
                        .as_ref()
                        .map(|pending| std::mem::take(&mut *pending.borrow_mut()))
                        .unwrap_or_default();
                    let added = pending
                        .into_iter()
                        .filter(|dependency| !known.contains(dependency))
                        .collect::<Vec<_>>();
                    if !added.is_empty() {
                        self.edges = self.edges.saturating_add(added.len());
                        if self.edges as u64 > engine.options.limits().max_dependency_edges() {
                            engine.retained_results.insert(
                                cell,
                                CalculationCellResult::Unavailable(CalculationIssue::new(
                                    CalculationIssueCode::ResourceLimitExceeded,
                                    Some(
                                        crate::calculation::CalculationLimitKind::DependencyEdges
                                            .detail()
                                            .to_owned(),
                                    ),
                                )),
                            );
                            engine.results.insert(
                                cell,
                                Value::Error(ErrorKind::ResourceLimit(
                                    crate::calculation::CalculationLimitKind::DependencyEdges,
                                )),
                            );
                        } else {
                            frame.dependencies.extend(added);
                            Arc::make_mut(&mut engine.dependencies)
                                .insert(cell, frame.dependencies.clone());
                            continue;
                        }
                    }
                    engine.evaluated_cells.insert(cell);
                }
            }
            self.done.insert(cell);
            active.remove(&cell);
            stack.pop();
        }
        Ok(())
    }
}
