//! Bringing an impact index up to date with edited formulas instead of building it again.

use std::collections::{BTreeMap, BTreeSet};

use super::dependency::DependencyTarget;
use super::{
    AreaBvhKind, AreaBvhNode, AreaSpatialIndex, DependencyImpactIndex, IndexedAreaDependency,
};
use crate::calculation::graph::{AreaId, DependencyGraph, DependencyNode};
use crate::calculation::runtime::CellId;
#[cfg(test)]
use crate::calculation::runtime::Rect;

/// What changed between the targets and graph an impact index was built from and the ones it is
/// brought up to date with.
pub(super) struct ImpactIndexChanges<'a> {
    pub(super) old_targets: &'a BTreeMap<CellId, Vec<DependencyTarget>>,
    pub(super) new_targets: &'a BTreeMap<CellId, Vec<DependencyTarget>>,
    pub(super) old_graph: &'a DependencyGraph,
    pub(super) new_graph: &'a DependencyGraph,
    /// The formulas whose targets may differ, including those that stopped being formulas.
    pub(super) retargeted: &'a BTreeSet<CellId>,
    /// The formulas whose graph nodes may differ, including those that stopped being formulas.
    pub(super) regrouped: &'a BTreeSet<CellId>,
    /// The id each area has after unreferenced areas were dropped, by its id before, `None` for a
    /// dropped area. Ids from the old graph's area count up are areas added since.
    pub(super) renumbered: &'a [Option<AreaId>],
}

impl DependencyImpactIndex {
    /// Returns this index brought up to date with `changes`: the same formulas for every cell
    /// and the same dependents for every node as [`Self::build`] over the new targets and graph.
    /// The array regions it was built with must not have changed, and no formula in
    /// `retargeted` may anchor one, since its rectangle would be dropped with the formula's.
    pub(super) fn updated(
        &self,
        changes: &ImpactIndexChanges<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let mut index = self.clone();
        index.update_targets(changes, cancelled)?;
        index.update_graph(changes, cancelled)?;
        Ok(index)
    }

    fn update_targets(
        &mut self,
        changes: &ImpactIndexChanges<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let mut area_sheets = BTreeSet::new();
        let mut impact_area_sheets = BTreeSet::new();
        for formula in changes.retargeted {
            if cancelled() {
                return Err(());
            }
            let old = changes
                .old_targets
                .get(formula)
                .map_or(&[][..], Vec::as_slice);
            let new = changes
                .new_targets
                .get(formula)
                .map_or(&[][..], Vec::as_slice);
            if old == new {
                continue;
            }
            for target in old {
                match target {
                    DependencyTarget::Cell(cell)
                    | DependencyTarget::SpillAnchor(cell)
                    | DependencyTarget::FormulaContent(cell) => {
                        remove_sorted(&mut self.exact, *cell, formula);
                    }
                    DependencyTarget::Area(span) => {
                        area_sheets.extend(span.rects().map(|rect| rect.sheet));
                    }
                    DependencyTarget::ImpactArea(span) => {
                        area_sheets.extend(span.rects().map(|rect| rect.sheet));
                        impact_area_sheets.extend(span.rects().map(|rect| rect.sheet));
                    }
                    DependencyTarget::TableIdentity(_) => {}
                }
            }
            for target in new {
                match target {
                    DependencyTarget::Cell(cell)
                    | DependencyTarget::SpillAnchor(cell)
                    | DependencyTarget::FormulaContent(cell) => {
                        insert_sorted(self.exact.entry(*cell).or_default(), *formula);
                    }
                    DependencyTarget::Area(span) => {
                        area_sheets.extend(span.rects().map(|rect| rect.sheet));
                    }
                    DependencyTarget::ImpactArea(span) => {
                        area_sheets.extend(span.rects().map(|rect| rect.sheet));
                        impact_area_sheets.extend(span.rects().map(|rect| rect.sheet));
                    }
                    DependencyTarget::TableIdentity(_) => {}
                }
            }
        }
        rebuild_sheets(
            &mut self.areas_by_sheet,
            &area_sheets,
            changes,
            true,
            cancelled,
        )?;
        rebuild_sheets(
            &mut self.impact_areas_by_sheet,
            &impact_area_sheets,
            changes,
            false,
            cancelled,
        )
    }

    fn update_graph(
        &mut self,
        changes: &ImpactIndexChanges<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let old_area_count = changes.old_graph.area_count();
        let renumber = |area: AreaId| changes.renumbered[area as usize];
        let dropped_old_area =
            (0..old_area_count).any(|area| renumber(area as AreaId) != Some(area as AreaId));

        // Withdraw what the regrouped formulas and the dropped areas waited for, under old ids.
        for formula in changes.regrouped {
            if cancelled() {
                return Err(());
            }
            for node in changes.old_graph.dependencies(*formula).unwrap_or_default() {
                match node {
                    DependencyNode::Cell(cell) => {
                        remove_sorted(
                            &mut self.reverse_dependents,
                            *cell,
                            &DependencyNode::Cell(*formula),
                        );
                    }
                    DependencyNode::Area(area) => {
                        let dependents = &mut self.area_dependents[*area as usize];
                        if let Ok(position) =
                            dependents.binary_search(&DependencyNode::Cell(*formula))
                        {
                            dependents.remove(position);
                        }
                    }
                }
            }
        }
        let mut area_dependents = Vec::with_capacity(changes.new_graph.area_count());
        area_dependents.resize_with(changes.new_graph.area_count(), Vec::new);
        for (area, dependents) in std::mem::take(&mut self.area_dependents)
            .into_iter()
            .enumerate()
        {
            if cancelled() {
                return Err(());
            }
            let Some(kept) = renumber(area as AreaId) else {
                for node in changes.old_graph.area(area as AreaId) {
                    if let DependencyNode::Cell(cell) = node {
                        remove_sorted(
                            &mut self.reverse_dependents,
                            *cell,
                            &DependencyNode::Area(area as AreaId),
                        );
                    }
                }
                continue;
            };
            area_dependents[kept as usize] = if dropped_old_area {
                renumbered_nodes(dependents, &renumber)
            } else {
                dependents
            };
        }
        self.area_dependents = area_dependents;
        if dropped_old_area {
            for dependents in self.reverse_dependents.values_mut() {
                if cancelled() {
                    return Err(());
                }
                if dependents
                    .iter()
                    .any(|node| matches!(node, DependencyNode::Area(_)))
                {
                    *dependents = renumbered_nodes(std::mem::take(dependents), &renumber);
                }
            }
        }

        // Add what the regrouped formulas and the added areas wait for, under new ids.
        let mut waiting = Vec::new();
        for formula in changes.regrouped {
            if let Some(nodes) = changes.new_graph.dependencies(*formula) {
                waiting.push((DependencyNode::Cell(*formula), nodes));
            }
        }
        for added in old_area_count..changes.renumbered.len() {
            if let Some(area) = renumber(added as AreaId) {
                waiting.push((DependencyNode::Area(area), changes.new_graph.area(area)));
            }
        }
        for (dependent, nodes) in waiting {
            if cancelled() {
                return Err(());
            }
            for node in nodes {
                match node {
                    DependencyNode::Cell(cell) => {
                        insert_sorted(self.reverse_dependents.entry(*cell).or_default(), dependent);
                    }
                    DependencyNode::Area(area) => {
                        insert_sorted(&mut self.area_dependents[*area as usize], dependent);
                    }
                }
            }
        }
        Ok(())
    }
}

impl AreaSpatialIndex {
    /// Returns every rectangle the index holds.
    fn entries(&self) -> Vec<IndexedAreaDependency> {
        let mut entries = Vec::new();
        let mut pending = self.root.iter().collect::<Vec<&AreaBvhNode>>();
        while let Some(node) = pending.pop() {
            match &node.kind {
                AreaBvhKind::Internal { children } => pending.extend(children),
                AreaBvhKind::Leaf { areas } => entries.extend(areas.iter().cloned()),
            }
        }
        entries
    }
}

/// Builds again the spatial index of each of `sheets`, keeping the rectangles of formulas whose
/// targets did not change and taking the new targets' rectangles of the rest: their `Area`
/// targets as well when `with_areas`, and their `ImpactArea` targets always.
fn rebuild_sheets(
    indexes: &mut BTreeMap<usize, AreaSpatialIndex>,
    sheets: &BTreeSet<usize>,
    changes: &ImpactIndexChanges<'_>,
    with_areas: bool,
    cancelled: &impl Fn() -> bool,
) -> Result<(), ()> {
    for sheet in sheets {
        let mut areas = indexes
            .get(sheet)
            .map(AreaSpatialIndex::entries)
            .unwrap_or_default();
        areas.retain(|area| !changes.retargeted.contains(&area.formula));
        for formula in changes.retargeted {
            for target in changes
                .new_targets
                .get(formula)
                .map_or(&[][..], Vec::as_slice)
            {
                let span = match target {
                    DependencyTarget::Area(span) if with_areas => span,
                    DependencyTarget::ImpactArea(span) => span,
                    _ => continue,
                };
                areas.extend(
                    span.rects()
                        .filter(|rect| rect.sheet == *sheet)
                        .map(|rect| IndexedAreaDependency {
                            rect,
                            formula: *formula,
                        }),
                );
            }
        }
        if areas.is_empty() {
            indexes.remove(sheet);
        } else {
            indexes.insert(*sheet, AreaSpatialIndex::build(areas, cancelled)?);
        }
    }
    Ok(())
}

/// Removes `value` from the sorted list of `key`, and the list once it is empty.
fn remove_sorted<T: Ord>(lists: &mut BTreeMap<CellId, Vec<T>>, key: CellId, value: &T) {
    if let Some(list) = lists.get_mut(&key) {
        if let Ok(position) = list.binary_search(value) {
            list.remove(position);
        }
        if list.is_empty() {
            lists.remove(&key);
        }
    }
}

/// Inserts `value` into a sorted list without duplicating it.
fn insert_sorted<T: Ord>(list: &mut Vec<T>, value: T) {
    if let Err(position) = list.binary_search(&value) {
        list.insert(position, value);
    }
}

/// Maps the areas among `nodes` to their new ids and drops the dropped ones, keeping the order.
fn renumbered_nodes(
    nodes: Vec<DependencyNode>,
    renumber: &impl Fn(AreaId) -> Option<AreaId>,
) -> Vec<DependencyNode> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            DependencyNode::Cell(_) => Some(node),
            DependencyNode::Area(area) => renumber(area).map(DependencyNode::Area),
        })
        .collect()
}

#[cfg(test)]
impl super::CompiledWorkbook {
    /// Whether the impact index holds what building it again from the targets and graph would.
    pub(in crate::calculation) fn impact_index_matches_build(&self) -> bool {
        let built = DependencyImpactIndex::build(
            &self.dependency_targets,
            &self.dependencies,
            &self.static_array_regions,
            &|| false,
        )
        .expect("an uncancelled build completes");
        normalized(&self.impact_index) == normalized(&built)
    }
}

/// Rectangles and their formulas by sheet.
#[cfg(test)]
type SheetAreas = BTreeMap<usize, Vec<(Rect, CellId)>>;

/// The contents of an index in a canonical order, independent of how its trees were shaped.
#[cfg(test)]
type NormalizedIndex = (
    BTreeMap<CellId, Vec<CellId>>,
    SheetAreas,
    SheetAreas,
    BTreeMap<CellId, Vec<DependencyNode>>,
    Vec<Vec<DependencyNode>>,
);

#[cfg(test)]
fn normalized(index: &DependencyImpactIndex) -> NormalizedIndex {
    let areas = |indexes: &BTreeMap<usize, AreaSpatialIndex>| {
        indexes
            .iter()
            .map(|(sheet, index)| {
                let mut entries = index
                    .entries()
                    .into_iter()
                    .map(|area| (area.rect, area.formula))
                    .collect::<Vec<_>>();
                entries.sort_by_key(|(rect, formula)| {
                    (
                        *formula,
                        rect.row_start,
                        rect.col_start,
                        rect.row_end,
                        rect.col_end,
                        rect.whole_rows,
                    )
                });
                (*sheet, entries)
            })
            .collect()
    };
    let area_dependents = index
        .area_dependents
        .iter()
        .map(|dependents| {
            let mut dependents = dependents.clone();
            dependents.sort_unstable();
            dependents.dedup();
            dependents
        })
        .collect();
    (
        index.exact.clone(),
        areas(&index.areas_by_sheet),
        areas(&index.impact_areas_by_sheet),
        index.reverse_dependents.clone(),
        area_dependents,
    )
}
