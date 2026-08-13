use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::ast::Expr;
use super::convert::value_from_cell;
use super::decimal::DecimalTrace;
use super::graph::{DependencyGraph, Schedule};
use super::limits::CalculationLimitKind;
use super::parser::ParseError;
use super::performance_counters::{WorkCounter, work_counter_add};
use super::runtime::{CellId, Rect};
use super::scope::{ScopeEntry, ScopeValue, scope_value};
use super::syntax::ParsedFormula;
use super::value::{ErrorKind, Value};
use super::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CalculationSnapshot,
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

pub(super) use dependency::DependencyTarget;
use dependency::{TableTopologyRevision, table_dependency_by_id_cancellable};
use materialization::ArrayRegion;
use reference::{ColumnExtents, cell_at};

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

fn collect_column_extents(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<ColumnExtents>, ()> {
    let mut extents = Vec::with_capacity(workbook.sheets().len());
    for sheet in workbook.sheets() {
        if cancelled() {
            return Err(());
        }
        let mut sheet_extents = ColumnExtents::default();
        for (column, row) in sheet.column_max_rows() {
            if cancelled() {
                return Err(());
            }
            sheet_extents.record(*column, *row);
        }
        extents.push(sheet_extents);
    }
    Ok(extents)
}

#[derive(Debug, Clone)]
pub(super) struct CompiledWorkbook {
    asts: Arc<BTreeMap<CellId, Arc<ParsedFormula>>>,
    defined_name_asts: Arc<Vec<Option<ParsedFormula>>>,
    dependencies: Arc<DependencyGraph>,
    static_array_regions: Arc<Vec<ArrayRegion>>,
    table_topologies: BTreeMap<crate::TableId, TableTopologyRevision>,
    parse_failures: Arc<BTreeMap<CellId, ParseError>>,
    name_cycle_cells: Arc<BTreeSet<CellId>>,
    name_limit_cells: Arc<BTreeSet<CellId>>,
    dependency_limit_exceeded: bool,
    cycle_cells: Arc<BTreeSet<CellId>>,
    blocked_cells: Arc<BTreeSet<CellId>>,
    impact_index: DependencyImpactIndex,
    schedule: Schedule,
    topological_rank: BTreeMap<CellId, usize>,
    limits: CalculationLimits,
    incremental_safe: bool,
}

impl CompiledWorkbook {
    pub(super) fn table_topology_matches(
        &self,
        workbook: &WorkbookSnapshot,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        for (table_id, topology) in &self.table_topologies {
            if cancelled() {
                return Err(());
            }
            if !table_dependency_by_id_cancellable(workbook, *table_id, cancelled)?
                .is_some_and(|current| current.topology() == *topology)
            {
                return Ok(false);
            }
        }
        Ok(true)
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

    pub(super) fn formula_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.asts.keys().chain(self.parse_failures.keys()).copied()
    }

    pub(super) fn direct_affected_formulas(
        &self,
        changed: CellId,
        charge: &mut impl FnMut() -> Result<(), ()>,
    ) -> Result<BTreeSet<CellId>, ()> {
        self.impact_index.formulas_for_cell(changed, charge)
    }

    pub(super) fn dependents(&self, cell: CellId) -> &[CellId] {
        self.impact_index
            .reverse_dependents
            .get(&cell)
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    pub(super) fn topological_rank(&self, cell: CellId) -> Option<usize> {
        self.topological_rank.get(&cell).copied()
    }
}

#[derive(Debug, Clone)]
struct IndexedAreaDependency {
    rect: Rect,
    formula: CellId,
}

/// Branching factor of the retained-reference packed BVH.
const AREA_BRANCH_FACTOR: usize = 16;

/// Maximum number of unique rectangles retained in a single leaf. Every unique
/// rectangle is stored in exactly one leaf, so the index retains exactly A
/// payload references for A deduplicated rectangles.
const AREA_LEAF_CAPACITY: usize = 16;

/// Axis-aligned minimum bounding rectangle of a BVH subtree. A `whole_rows`
/// rectangle is fully encoded by its `row_start`/`row_end` bounds (the flag is
/// defined as `row_start == 1 && row_end == EXCEL_MAX_ROWS`), so no special
/// column handling is required for point containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AreaMbr {
    row_min: u32,
    row_max: u32,
    col_min: u32,
    col_max: u32,
}

impl AreaMbr {
    fn from_rect(rect: &Rect) -> Self {
        Self {
            row_min: rect.row_start,
            row_max: rect.row_end,
            col_min: rect.col_start,
            col_max: rect.col_end,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            row_min: self.row_min.min(other.row_min),
            row_max: self.row_max.max(other.row_max),
            col_min: self.col_min.min(other.col_min),
            col_max: self.col_max.max(other.col_max),
        }
    }

    fn contains(self, row: u32, column: u32) -> bool {
        self.row_min <= row
            && row <= self.row_max
            && self.col_min <= column
            && column <= self.col_max
    }
}

#[derive(Debug, Clone)]
enum AreaBvhKind {
    Internal { children: Vec<AreaBvhNode> },
    Leaf { areas: Vec<IndexedAreaDependency> },
}

#[derive(Debug, Clone)]
struct AreaBvhNode {
    mbr: AreaMbr,
    kind: AreaBvhKind,
}

/// Point-containment for the retained-reference area index. Matches the
/// original `AreaSpatialIndex::formulas_for_cell` semantics exactly: a rectangle
/// matches iff `col_start <= column <= col_end` and `row_start <= row <=
/// row_end`. `whole_rows` (`row_start == 1 && row_end == EXCEL_MAX_ROWS`) is
/// already reflected in the row bounds and never widens the column range.
fn area_rect_contains(rect: &Rect, row: u32, column: u32) -> bool {
    rect.col_start <= column
        && column <= rect.col_end
        && rect.row_start <= row
        && row <= rect.row_end
}

fn area_mbr_of(areas: &[IndexedAreaDependency]) -> AreaMbr {
    let mut areas = areas.iter();
    let first = areas.next().expect("area slice is non-empty");
    let mut mbr = AreaMbr::from_rect(&first.rect);
    for area in areas {
        mbr = mbr.union(AreaMbr::from_rect(&area.rect));
    }
    mbr
}

fn canonical_area_key(area: &IndexedAreaDependency) -> [u8; 41] {
    let mut key = [0_u8; 41];
    key[0..8].copy_from_slice(&(area.rect.sheet as u64).to_be_bytes());
    key[8..12].copy_from_slice(&area.rect.row_start.to_be_bytes());
    key[12..16].copy_from_slice(&area.rect.row_end.to_be_bytes());
    key[16..20].copy_from_slice(&area.rect.col_start.to_be_bytes());
    key[20..24].copy_from_slice(&area.rect.col_end.to_be_bytes());
    key[24] = u8::from(area.rect.whole_rows);
    key[25..33].copy_from_slice(&(area.formula.0 as u64).to_be_bytes());
    key[33..37].copy_from_slice(&area.formula.1.to_be_bytes());
    key[37..41].copy_from_slice(&area.formula.2.to_be_bytes());
    key
}

fn spatial_area_key(area: &IndexedAreaDependency) -> u64 {
    let row = (u64::from(area.rect.row_start) + u64::from(area.rect.row_end)) / 2;
    let column = (u64::from(area.rect.col_start) + u64::from(area.rect.col_end)) / 2;
    interleave_u32(row as u32, column as u32)
}

fn interleave_u32(left: u32, right: u32) -> u64 {
    fn spread(value: u32) -> u64 {
        let mut value = u64::from(value);
        value = (value | value << 16) & 0x0000_FFFF_0000_FFFF;
        value = (value | value << 8) & 0x00FF_00FF_00FF_00FF;
        value = (value | value << 4) & 0x0F0F_0F0F_0F0F_0F0F;
        value = (value | value << 2) & 0x3333_3333_3333_3333;
        value = (value | value << 1) & 0x5555_5555_5555_5555;
        value
    }
    (spread(left) << 1) | spread(right)
}

fn radix_sort_by_bytes<const N: usize>(
    values: &mut Vec<IndexedAreaDependency>,
    key: impl Fn(&IndexedAreaDependency) -> [u8; N],
    cancelled: &impl Fn() -> bool,
) -> Result<(), ()> {
    for byte_index in (0..N).rev() {
        if cancelled() {
            return Err(());
        }
        let mut buckets = std::array::from_fn::<Vec<IndexedAreaDependency>, 256, _>(|_| Vec::new());
        for (index, value) in values.drain(..).enumerate() {
            if index % 256 == 0 && cancelled() {
                return Err(());
            }
            buckets[usize::from(key(&value)[byte_index])].push(value);
        }
        let mut appended = 0_usize;
        for bucket in buckets {
            for value in bucket {
                if appended.is_multiple_of(256) && cancelled() {
                    return Err(());
                }
                values.push(value);
                appended += 1;
            }
        }
    }
    Ok(())
}

fn radix_sort_areas(
    areas: &mut Vec<IndexedAreaDependency>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), ()> {
    radix_sort_by_bytes(areas, canonical_area_key, cancelled)?;
    let mut deduplicated = Vec::with_capacity(areas.len());
    for (index, area) in areas.drain(..).enumerate() {
        if index % 256 == 0 && cancelled() {
            return Err(());
        }
        let duplicate = deduplicated
            .last()
            .is_some_and(|previous: &IndexedAreaDependency| {
                previous.rect == area.rect && previous.formula == area.formula
            });
        if !duplicate {
            deduplicated.push(area);
        }
    }
    *areas = deduplicated;
    radix_sort_by_bytes(
        areas,
        |area| spatial_area_key(area).to_be_bytes(),
        cancelled,
    )
}

#[derive(Debug, Clone, Default)]
struct AreaSpatialIndex {
    root: Option<AreaBvhNode>,
    #[cfg_attr(not(test), allow(dead_code))]
    height: usize,
}

impl AreaSpatialIndex {
    /// Builds the BVH for one sheet's deduplicated rectangle list. The
    /// `AreaSourceRectangles` / `AreaPayloadRefsRetained` / `AreaNodesRetained`
    /// counters are `store`d per build (one build per sheet), so after a
    /// single-sheet build they equal that sheet's counts.
    fn build(
        mut areas: Vec<IndexedAreaDependency>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        radix_sort_areas(&mut areas, cancelled)?;
        let source_rectangles = areas.len();
        let mut node_count = 0_usize;
        let mut payload_count = 0_usize;
        let mut level = Vec::with_capacity(areas.len().div_ceil(AREA_LEAF_CAPACITY));
        let mut area_iter = areas.into_iter();
        loop {
            if cancelled() {
                return Err(());
            }
            let leaf_areas = area_iter
                .by_ref()
                .take(AREA_LEAF_CAPACITY)
                .collect::<Vec<_>>();
            if leaf_areas.is_empty() {
                break;
            }
            node_count += 1;
            payload_count += leaf_areas.len();
            work_counter_add(WorkCounter::AreaBuildPayloadVisits, leaf_areas.len() as u64);
            level.push(AreaBvhNode {
                mbr: area_mbr_of(&leaf_areas),
                kind: AreaBvhKind::Leaf { areas: leaf_areas },
            });
        }
        let mut height = usize::from(!level.is_empty());
        while level.len() > 1 {
            let mut parents = Vec::with_capacity(level.len().div_ceil(AREA_BRANCH_FACTOR));
            let mut children = level.into_iter();
            loop {
                if cancelled() {
                    return Err(());
                }
                let child_group = children
                    .by_ref()
                    .take(AREA_BRANCH_FACTOR)
                    .collect::<Vec<_>>();
                if child_group.is_empty() {
                    break;
                }
                let mut mbr = child_group[0].mbr;
                for child in &child_group[1..] {
                    mbr = mbr.union(child.mbr);
                }
                node_count += 1;
                parents.push(AreaBvhNode {
                    mbr,
                    kind: AreaBvhKind::Internal {
                        children: child_group,
                    },
                });
            }
            level = parents;
            height += 1;
        }
        let root = level.pop();
        work_counter_add(WorkCounter::AreaSourceRectangles, source_rectangles as u64);
        work_counter_add(WorkCounter::AreaPayloadRefsRetained, payload_count as u64);
        work_counter_add(WorkCounter::AreaNodesRetained, node_count as u64);
        Ok(Self { root, height })
    }

    fn formulas_for_cell(
        &self,
        row: u32,
        column: u32,
        output: &mut BTreeSet<CellId>,
        charge: &mut impl FnMut() -> Result<(), ()>,
    ) -> Result<(), ()> {
        if let Some(root) = &self.root {
            root.formulas_for_cell(row, column, output, charge)?;
        }
        Ok(())
    }

    #[cfg(test)]
    const fn height(&self) -> usize {
        self.height
    }
}

impl AreaBvhNode {
    fn formulas_for_cell(
        &self,
        row: u32,
        column: u32,
        output: &mut BTreeSet<CellId>,
        charge: &mut impl FnMut() -> Result<(), ()>,
    ) -> Result<(), ()> {
        charge()?;
        if !self.mbr.contains(row, column) {
            return Ok(());
        }
        work_counter_add(WorkCounter::AreaQueryNodesVisited, 1);
        match &self.kind {
            AreaBvhKind::Leaf { areas } => {
                for area in areas {
                    charge()?;
                    work_counter_add(WorkCounter::AreaQueryCandidatesExamined, 1);
                    if area_rect_contains(&area.rect, row, column) {
                        // Counted before the caller's final sort+dedup: one
                        // increment per matching rectangle (formula emission).
                        work_counter_add(WorkCounter::AreaQueryMatchesEmitted, 1);
                        #[cfg(test)]
                        super::work_counter::area_dependency_visit();
                        output.insert(area.formula);
                    }
                }
            }
            AreaBvhKind::Internal { children } => {
                for child in children {
                    child.formulas_for_cell(row, column, output, charge)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct DependencyImpactIndex {
    exact: BTreeMap<CellId, Vec<CellId>>,
    areas_by_sheet: BTreeMap<usize, AreaSpatialIndex>,
    reverse_dependents: BTreeMap<CellId, Vec<CellId>>,
}

impl DependencyImpactIndex {
    fn build(
        targets: &BTreeMap<CellId, Vec<DependencyTarget>>,
        dependencies: &DependencyGraph,
        array_regions: &[ArrayRegion],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut index = Self::default();
        let mut area_lists = BTreeMap::<usize, Vec<IndexedAreaDependency>>::new();
        for (formula, formula_targets) in targets {
            if cancelled() {
                return Err(());
            }
            for target in formula_targets {
                #[cfg(test)]
                super::work_counter::dependency_target_scan();
                if cancelled() {
                    return Err(());
                }
                match target {
                    DependencyTarget::Cell(cell)
                    | DependencyTarget::SpillAnchor(cell)
                    | DependencyTarget::FormulaContent(cell) => {
                        index.exact.entry(*cell).or_default().push(*formula);
                    }
                    DependencyTarget::Area(span) => {
                        for rect in span.rects() {
                            area_lists
                                .entry(rect.sheet)
                                .or_default()
                                .push(IndexedAreaDependency {
                                    rect,
                                    formula: *formula,
                                });
                        }
                    }
                    DependencyTarget::TableIdentity(_) => {}
                }
            }
        }
        for region in array_regions {
            if cancelled() {
                return Err(());
            }
            area_lists
                .entry(region.rect.sheet)
                .or_default()
                .push(IndexedAreaDependency {
                    rect: region.rect,
                    formula: region.anchor,
                });
        }
        for (formula, formula_dependencies) in dependencies {
            if cancelled() {
                return Err(());
            }
            for dependency in formula_dependencies {
                index
                    .reverse_dependents
                    .entry(*dependency)
                    .or_default()
                    .push(*formula);
            }
        }
        for formulas in index.exact.values_mut() {
            dedup_sorted_cancellable(formulas, cancelled)?;
        }
        for (sheet, areas) in area_lists {
            index
                .areas_by_sheet
                .insert(sheet, AreaSpatialIndex::build(areas, cancelled)?);
        }
        for formulas in index.reverse_dependents.values_mut() {
            dedup_sorted_cancellable(formulas, cancelled)?;
        }
        Ok(index)
    }

    fn formulas_for_cell(
        &self,
        cell: CellId,
        charge: &mut impl FnMut() -> Result<(), ()>,
    ) -> Result<BTreeSet<CellId>, ()> {
        let mut formulas = BTreeSet::new();
        if let Some(exact) = self.exact.get(&cell) {
            for formula in exact {
                charge()?;
                formulas.insert(*formula);
            }
        }
        if let Some(areas) = self.areas_by_sheet.get(&cell.0) {
            areas.formulas_for_cell(cell.1, cell.2, &mut formulas, charge)?;
        }
        Ok(formulas)
    }
}

fn dedup_sorted_cancellable(
    values: &mut Vec<CellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), ()> {
    let mut deduplicated = Vec::with_capacity(values.len());
    for (index, value) in values.drain(..).enumerate() {
        if index % 256 == 0 && cancelled() {
            return Err(());
        }
        if deduplicated.last() != Some(&value) {
            deduplicated.push(value);
        }
    }
    *values = deduplicated;
    Ok(())
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

    pub(super) const fn with_cell(self, cell: CellId) -> Self {
        Self { cell, ..self }
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
    Previous(&'engine CalculationCellResult),
    Literal(&'engine CellValue),
    Blank,
    Error(ErrorKind),
}

#[derive(Debug)]
pub struct Engine<'workbook> {
    workbook: &'workbook WorkbookSnapshot,
    previous: Option<&'workbook CalculationSnapshot>,
    dirty: Option<BTreeSet<CellId>>,
    options: CalculationOptions,
    table_topologies: BTreeMap<crate::TableId, TableTopologyRevision>,
    asts: Arc<BTreeMap<CellId, Arc<ParsedFormula>>>,
    defined_name_asts: Arc<Vec<Option<ParsedFormula>>>,
    dependencies: Arc<DependencyGraph>,
    results: BTreeMap<CellId, Value>,
    numeric_decimal_traces: BTreeMap<CellId, DecimalTrace>,
    retained_results: BTreeMap<CellId, CalculationCellResult>,
    array_regions: Vec<ArrayRegion>,
    column_extents: Vec<ColumnExtents>,
    dynamic_spills: BTreeMap<CellId, Rect>,
    parse_failures: Arc<BTreeMap<CellId, ParseError>>,
    name_cycle_cells: Arc<BTreeSet<CellId>>,
    name_limit_cells: Arc<BTreeSet<CellId>>,
    dependency_limit_exceeded: bool,
    pub(super) cycle_cells: Arc<BTreeSet<CellId>>,
    pub(super) blocked_cells: Arc<BTreeSet<CellId>>,
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
        if let Some(materialized) = self.previous_materialized(cell) {
            return ValueSource::Previous(materialized.result());
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
            ValueSource::Previous(result) => value_from_calculation_result(result),
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
            ValueSource::Previous(_) => self.previous_numeric_decimal_trace(cell),
            ValueSource::Literal(CellValue::Number(number)) => {
                DecimalTrace::from_number(number.get())
            }
            ValueSource::Literal(_) | ValueSource::Blank | ValueSource::Error(_) => None,
        }
    }

    pub(super) fn calculated_decimal_trace(&self, cell: CellId) -> Option<DecimalTrace> {
        self.numeric_decimal_traces
            .get(&cell)
            .copied()
            .or_else(|| self.previous_numeric_decimal_trace(cell))
    }

    fn previous_materialized(&self, cell: CellId) -> Option<&crate::MaterializedCalculationCell> {
        let previous = self.previous?;
        let public = internal_to_public(self.workbook, cell)?;
        let materialized = previous.materialized_cell(public)?;
        let owner = match materialized.origin() {
            crate::MaterializedResultOrigin::DirectFormula => cell,
            crate::MaterializedResultOrigin::LegacyArray { anchor, .. }
            | crate::MaterializedResultOrigin::DynamicSpill { anchor, .. } => {
                public_to_internal(self.workbook, anchor)?
            }
        };
        if self
            .dirty
            .as_ref()
            .is_some_and(|dirty| dirty.contains(&owner))
        {
            None
        } else {
            Some(materialized)
        }
    }

    fn previous_numeric_decimal_trace(&self, cell: CellId) -> Option<DecimalTrace> {
        let previous = self.previous?;
        let public = internal_to_public(self.workbook, cell)?;
        self.previous_materialized(cell)?;
        previous.numeric_decimal_trace(public)
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
                        self.cell_value(*dependency),
                        Value::Error(kind) if kind.is_engine_issue()
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
        self.asts.get(&cell).map(|parsed| parsed.root())
    }

    pub(super) fn cell_has_formula(&self, cell: CellId) -> bool {
        self.workbook
            .sheets()
            .get(cell.0)
            .and_then(|sheet| cell_at(sheet, cell.1, cell.2))
            .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)))
    }

    pub(super) fn cell_formula_text(&self, cell: CellId) -> Option<&str> {
        let CellContent::Formula(formula) = self
            .workbook
            .sheets()
            .get(cell.0)
            .and_then(|sheet| cell_at(sheet, cell.1, cell.2))?
            .content()
        else {
            return None;
        };
        formula.text().map(crate::FormulaText::as_str)
    }

    pub(super) fn workbook_sheet_count(&self) -> usize {
        self.workbook.sheets().len()
    }

    pub(super) fn workbook_sheet_index(&self, name: &str) -> Option<usize> {
        self.workbook.sheet_index_by_name(name)
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

pub(super) fn internal_to_public(
    workbook: &WorkbookSnapshot,
    cell: CellId,
) -> Option<CalculationCellId> {
    let sheet = workbook.sheets().get(cell.0)?;
    let address = crate::CellAddress::from_indices(cell.1, cell.2).ok()?;
    Some(CalculationCellId::new(sheet.id(), address))
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

#[cfg(test)]
mod area_bvh_tests {
    use super::super::runtime::{CellId, Rect};
    use super::{
        AREA_BRANCH_FACTOR, AREA_LEAF_CAPACITY, AreaSpatialIndex, IndexedAreaDependency,
        area_rect_contains,
    };
    use crate::testing::{
        WorkCounter, lock_work_counters, reset_work_counters, snapshot_work_counters,
    };
    use std::collections::BTreeSet;

    const QUERY_SHEET: usize = 0;

    fn rect(row_start: u32, col_start: u32, row_end: u32, col_end: u32, whole_rows: bool) -> Rect {
        Rect {
            sheet: QUERY_SHEET,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows,
        }
    }

    fn dependency(rect: Rect, formula: CellId) -> IndexedAreaDependency {
        IndexedAreaDependency { rect, formula }
    }

    fn formula(id: usize) -> CellId {
        (QUERY_SHEET, id as u32, 1)
    }

    fn brute_force(areas: &[IndexedAreaDependency], row: u32, column: u32) -> Vec<CellId> {
        let mut matches: Vec<CellId> = areas
            .iter()
            .filter(|area| area_rect_contains(&area.rect, row, column))
            .map(|area| area.formula)
            .collect();
        matches.sort_unstable();
        matches.dedup();
        matches
    }

    fn raw_match_count(areas: &[IndexedAreaDependency], row: u32, column: u32) -> u64 {
        areas
            .iter()
            .filter(|area| area_rect_contains(&area.rect, row, column))
            .count() as u64
    }

    fn build_index(areas: Vec<IndexedAreaDependency>) -> AreaSpatialIndex {
        reset_work_counters();
        AreaSpatialIndex::build(areas, &|| false).expect("area index builds")
    }

    fn assert_exact_and_bounded(
        name: &str,
        areas: &[IndexedAreaDependency],
        query_row: u32,
        query_column: u32,
    ) {
        let _guard = lock_work_counters();
        let index = build_index(areas.to_vec());

        let mut got = BTreeSet::new();
        index
            .formulas_for_cell(query_row, query_column, &mut got, &mut || Ok(()))
            .expect("query completes");
        let got = got.into_iter().collect::<Vec<_>>();
        let expected = brute_force(areas, query_row, query_column);
        assert_eq!(got, expected, "{name}: exactness mismatch");

        let build_snapshot = snapshot_work_counters();
        let a = areas.len() as u64;
        assert_eq!(
            build_snapshot.get(WorkCounter::AreaSourceRectangles),
            a,
            "{name}: source rectangle count"
        );
        assert_eq!(
            build_snapshot.get(WorkCounter::AreaPayloadRefsRetained),
            a,
            "{name}: payload references retained"
        );
        assert_eq!(
            build_snapshot.get(WorkCounter::AreaBuildPayloadVisits),
            a,
            "{name}: build payload visits"
        );
        assert!(
            build_snapshot.get(WorkCounter::AreaNodesRetained) <= 4 * a + 1,
            "{name}: nodes retained exceeded 4A+1"
        );

        reset_work_counters();
        let mut ignored = BTreeSet::new();
        index
            .formulas_for_cell(query_row, query_column, &mut ignored, &mut || Ok(()))
            .expect("bounded query completes");
        let query_snapshot = snapshot_work_counters();

        let matches = raw_match_count(areas, query_row, query_column);
        let leaf_capacity = AREA_LEAF_CAPACITY as u64;
        let branch_factor = AREA_BRANCH_FACTOR as u64;
        let height = index.height() as u64;
        let candidates = query_snapshot.get(WorkCounter::AreaQueryCandidatesExamined);
        let nodes = query_snapshot.get(WorkCounter::AreaQueryNodesVisited);
        assert_eq!(
            query_snapshot.get(WorkCounter::AreaQueryMatchesEmitted),
            matches,
            "{name}: matches emitted before dedup"
        );
        assert!(
            candidates <= matches + 2 * leaf_capacity * (height + 1),
            "{name}: candidates {candidates} > {matches} + 2*{leaf_capacity}*({height}+1)"
        );
        assert!(
            nodes <= 2 * branch_factor * (height + 1) + matches.div_ceil(leaf_capacity),
            "{name}: nodes {nodes} exceeded 2*{branch_factor}*({height}+1) + ceil({matches}/{leaf_capacity})"
        );
    }

    #[test]
    fn o2_exactness_and_bounds_same_rows_disjoint_columns() {
        let count = 4096;
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let column = i as u32 + 1;
            areas.push(dependency(rect(1, column, 100, column, false), formula(i)));
        }
        assert_exact_and_bounded("same-rows-disjoint-columns", &areas, 50, 2048);
    }

    #[test]
    fn o2_exactness_and_bounds_same_columns_disjoint_rows() {
        let count = 4096;
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let row = i as u32 + 1;
            areas.push(dependency(rect(row, 1, row, 100, false), formula(i)));
        }
        assert_exact_and_bounded("same-columns-disjoint-rows", &areas, 2048, 50);
    }

    #[test]
    fn o2_exactness_and_bounds_all_contain_query_point_nested() {
        let count = 4096;
        let outer = 5000;
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let start = i as u32 + 1;
            areas.push(dependency(
                rect(start, start, outer, outer, false),
                formula(i),
            ));
        }
        assert_exact_and_bounded("all-contain-query-point-nested", &areas, outer, outer);
    }

    #[test]
    fn o2_exactness_and_bounds_full_width_rows_disjoint() {
        let count = 4096;
        let max_column = crate::EXCEL_MAX_COLUMNS;
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let row = i as u32 + 1;
            areas.push(dependency(rect(row, 1, row, max_column, false), formula(i)));
        }
        assert_exact_and_bounded("full-width-row-disjoint", &areas, 2048, 5000);
    }

    fn xorshift64(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    #[test]
    fn o2_exactness_and_bounds_random_sparse_seed_1515() {
        let count = 10_000;
        let mut state = 1515_u64;
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let row_start = (xorshift64(&mut state) % 1_000_000) as u32 + 1;
            let row_height = (xorshift64(&mut state) % 8) as u32 + 1;
            let col_start = (xorshift64(&mut state) % 16_000) as u32 + 1;
            let col_width = (xorshift64(&mut state) % 8) as u32 + 1;
            areas.push(dependency(
                rect(
                    row_start,
                    col_start,
                    row_start + row_height - 1,
                    col_start + col_width - 1,
                    false,
                ),
                formula(i),
            ));
        }
        let query_row = areas[0].rect.row_start;
        let query_column = areas[0].rect.col_start;
        assert_exact_and_bounded("random-sparse-1515", &areas, query_row, query_column);
    }

    #[test]
    fn o2_whole_rows_rect_matches_all_rows_within_its_declared_columns() {
        let _guard = lock_work_counters();
        let areas = vec![
            dependency(rect(1, 2, crate::EXCEL_MAX_ROWS, 2, true), formula(0)),
            dependency(rect(10, 5, 20, 7, false), formula(1)),
        ];
        let index = build_index(areas.clone());

        let mut got = BTreeSet::new();
        index
            .formulas_for_cell(500_000, 2, &mut got, &mut || Ok(()))
            .expect("whole-row query completes");
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec![formula(0)]);

        let mut got = BTreeSet::new();
        index
            .formulas_for_cell(500_000, 3, &mut got, &mut || Ok(()))
            .expect("disjoint whole-row query completes");
        assert!(got.is_empty());
    }

    #[test]
    fn o2_duplicate_rectangles_are_deduplicated_before_retention() {
        let _guard = lock_work_counters();
        let first = dependency(rect(1, 1, 10, 10, false), formula(0));
        let second = dependency(rect(20, 20, 30, 30, false), formula(1));
        let index = build_index(vec![
            first.clone(),
            second.clone(),
            first.clone(),
            second,
            first,
        ]);
        let snapshot = snapshot_work_counters();
        assert_eq!(snapshot.get(WorkCounter::AreaSourceRectangles), 2);
        assert_eq!(snapshot.get(WorkCounter::AreaPayloadRefsRetained), 2);
        assert_eq!(snapshot.get(WorkCounter::AreaBuildPayloadVisits), 2);

        let mut formulas = BTreeSet::new();
        index
            .formulas_for_cell(5, 5, &mut formulas, &mut || Ok(()))
            .expect("deduplicated query completes");
        assert_eq!(formulas.into_iter().collect::<Vec<_>>(), vec![formula(0)]);
    }

    #[test]
    fn o2_mid_sort_cancellation_publishes_no_partial_index_counters() {
        use std::cell::Cell;

        let _guard = lock_work_counters();
        let areas = (0..1_024)
            .map(|index| {
                dependency(
                    rect(1, index + 1, 10, index + 1, false),
                    formula(index as usize),
                )
            })
            .collect();
        let polls = Cell::new(0_u32);
        reset_work_counters();
        let result = AreaSpatialIndex::build(areas, &|| {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 4
        });
        assert!(result.is_err());
        assert_eq!(
            polls.get(),
            4,
            "cancellation occurred after 256 drain items"
        );
        let snapshot = snapshot_work_counters();
        assert_eq!(snapshot.get(WorkCounter::AreaSourceRectangles), 0);
        assert_eq!(snapshot.get(WorkCounter::AreaPayloadRefsRetained), 0);
        assert_eq!(snapshot.get(WorkCounter::AreaNodesRetained), 0);
        assert_eq!(snapshot.get(WorkCounter::AreaBuildPayloadVisits), 0);
    }
}
