use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet};

use super::runtime::CellId;

/// Index of an area in its [`DependencyGraph`].
pub(super) type AreaId = u32;

/// Something a formula or an area waits for before it is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum DependencyNode {
    /// A formula cell, or the anchor of an array formula whose result covers a read cell.
    Cell(CellId),
    /// A group of cells that the ranges of several formulas share.
    Area(AreaId),
}

/// Formula cells, the nodes each one waits for, and the areas among those nodes.
///
/// An area is complete once every node it holds is complete. A formula that reads a range waits
/// for the areas covering it instead of one edge per formula cell inside it, and overlapping
/// ranges share their areas.
#[derive(Debug, Clone, Default)]
pub(super) struct DependencyGraph {
    formulas: BTreeMap<CellId, Vec<DependencyNode>>,
    areas: Vec<Box<[DependencyNode]>>,
    area_edges: u64,
}

impl DependencyGraph {
    /// Records the nodes `formula` waits for.
    pub(super) fn insert(&mut self, formula: CellId, mut nodes: Vec<DependencyNode>) {
        nodes.sort_unstable();
        nodes.dedup();
        self.formulas.insert(formula, nodes);
    }

    /// Forgets a cell that is no longer a calculated formula.
    pub(super) fn remove(&mut self, formula: CellId) {
        self.formulas.remove(&formula);
    }

    /// Returns every edge: one per node each formula waits for and one per area member.
    pub(super) fn edge_count(&self) -> u64 {
        self.formulas
            .values()
            .map(|nodes| nodes.len() as u64)
            .sum::<u64>()
            + self.area_edges
    }

    /// Records a formula that waits only for other cells.
    pub(super) fn insert_cells(&mut self, formula: CellId, cells: Vec<CellId>) {
        self.insert(
            formula,
            cells.into_iter().map(DependencyNode::Cell).collect(),
        );
    }

    /// Adds an area holding `members` and returns its node.
    pub(super) fn add_area(&mut self, members: Vec<DependencyNode>) -> DependencyNode {
        let id = AreaId::try_from(self.areas.len()).expect("area count fits the dependency graph");
        self.area_edges += members.len() as u64;
        self.areas.push(members.into_boxed_slice());
        DependencyNode::Area(id)
    }

    /// Returns the number of members held by every area so far.
    pub(super) const fn area_edges(&self) -> u64 {
        self.area_edges
    }

    /// Drops the areas no formula waits for, directly or through other areas, and renumbers the
    /// rest in their order. Returns the new id of each old area, `None` for a dropped one.
    pub(super) fn retain_referenced_areas(&mut self) -> Vec<Option<AreaId>> {
        let area_of = |node: &DependencyNode| match node {
            DependencyNode::Area(area) => Some(*area),
            DependencyNode::Cell(_) => None,
        };
        let mut referenced = vec![false; self.areas.len()];
        let mut pending = self
            .formulas
            .values()
            .flatten()
            .filter_map(area_of)
            .collect::<Vec<_>>();
        while let Some(area) = pending.pop() {
            if !std::mem::replace(&mut referenced[area as usize], true) {
                pending.extend(self.areas[area as usize].iter().filter_map(area_of));
            }
        }
        let mut next: AreaId = 0;
        let renumbered = referenced
            .iter()
            .map(|kept| {
                kept.then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect::<Vec<_>>();
        if next as usize == self.areas.len() {
            return renumbered;
        }
        let renumber = |node: &mut DependencyNode| {
            if let DependencyNode::Area(area) = node {
                *area = renumbered[*area as usize].expect("a referenced area's members are kept");
            }
        };
        let areas = std::mem::take(&mut self.areas);
        self.area_edges = 0;
        for (mut members, kept) in areas.into_iter().zip(&referenced) {
            if *kept {
                members.iter_mut().for_each(renumber);
                self.area_edges += members.len() as u64;
                self.areas.push(members);
            }
        }
        for nodes in self.formulas.values_mut() {
            nodes.iter_mut().for_each(renumber);
        }
        renumbered
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.formulas.is_empty()
    }

    /// Returns the nodes `formula` waits for directly.
    pub(super) fn dependencies(&self, formula: CellId) -> Option<&[DependencyNode]> {
        self.formulas.get(&formula).map(Vec::as_slice)
    }

    pub(super) fn formulas(&self) -> impl Iterator<Item = (CellId, &[DependencyNode])> {
        self.formulas
            .iter()
            .map(|(cell, nodes)| (*cell, nodes.as_slice()))
    }

    pub(super) fn areas(&self) -> impl Iterator<Item = (AreaId, &[DependencyNode])> {
        self.areas.iter().enumerate().map(|(id, members)| {
            (
                AreaId::try_from(id).expect("area count fits the dependency graph"),
                &**members,
            )
        })
    }

    pub(super) fn area_count(&self) -> usize {
        self.areas.len()
    }

    /// Returns the members of `area`.
    pub(super) fn area(&self, area: AreaId) -> &[DependencyNode] {
        &self.areas[area as usize]
    }

    /// Counts the distinct cells `nodes` reach through their areas, stopping at `limit`.
    pub(super) fn count_cells(&self, nodes: &[DependencyNode], limit: u64) -> u64 {
        let mut cells = HashSet::new();
        let mut areas = HashSet::new();
        let mut pending = nodes.to_vec();
        while let Some(node) = pending.pop() {
            match node {
                DependencyNode::Cell(cell) => {
                    cells.insert(cell);
                    if cells.len() as u64 >= limit {
                        break;
                    }
                }
                DependencyNode::Area(area) => {
                    if areas.insert(area) {
                        pending.extend_from_slice(self.area(area));
                    }
                }
            }
        }
        cells.len() as u64
    }

    /// Reports whether `found` holds for any cell `formula` waits for, directly or through areas.
    pub(super) fn any_dependency(
        &self,
        formula: CellId,
        mut found: impl FnMut(CellId) -> bool,
    ) -> bool {
        let Some(nodes) = self.formulas.get(&formula) else {
            return false;
        };
        let mut areas = HashSet::new();
        let mut pending = nodes.clone();
        while let Some(node) = pending.pop() {
            match node {
                DependencyNode::Cell(cell) => {
                    if found(cell) {
                        return true;
                    }
                }
                DependencyNode::Area(area) => {
                    if areas.insert(area) {
                        pending.extend_from_slice(self.area(area));
                    }
                }
            }
        }
        false
    }

    /// Returns every cell `formula` waits for, directly or through areas.
    #[cfg(test)]
    pub(super) fn cells(&self, formula: CellId) -> Option<Vec<CellId>> {
        let mut cells = BTreeSet::new();
        self.any_dependency(formula, |cell| {
            cells.insert(cell);
            false
        });
        self.formulas
            .contains_key(&formula)
            .then(|| cells.into_iter().collect())
    }
}

#[derive(Debug, Clone)]
pub(super) struct Schedule {
    pub(super) order: Vec<CellId>,
    pub(super) cycle_cells: BTreeSet<CellId>,
    pub(super) blocked_cells: BTreeSet<CellId>,
}

#[cfg(test)]
pub(super) fn schedule(dependencies: &DependencyGraph) -> Schedule {
    schedule_cancellable(dependencies, &|| false)
        .expect("non-cancellable scheduling cannot be cancelled")
}

/// Edges between the formulas and areas of a graph, numbered densely: formula `i` is the `i`th
/// formula cell in cell order and area `a` follows every formula. Only edges to formulas in the
/// graph and to areas are kept; other cells are never calculated, so nothing waits for them.
struct DenseGraph {
    formulas: Vec<CellId>,
    /// `dependencies[dependency_starts[v]..dependency_starts[v + 1]]` are the nodes `v` waits for.
    dependency_starts: Vec<usize>,
    dependencies: Vec<u32>,
    dependent_starts: Vec<usize>,
    dependents: Vec<u32>,
}

impl DenseGraph {
    fn build(graph: &DependencyGraph, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let formulas = graph.formulas.keys().copied().collect::<Vec<_>>();
        let formula_count = formulas.len();
        let node_count = formula_count + graph.areas.len();
        let index = |node: &DependencyNode| match node {
            DependencyNode::Cell(cell) => formulas
                .binary_search(cell)
                .ok()
                .map(|position| position as u32),
            DependencyNode::Area(area) => Some((formula_count + *area as usize) as u32),
        };
        let mut dependency_starts = Vec::with_capacity(node_count + 1);
        let mut dependencies = Vec::new();
        dependency_starts.push(0);
        for nodes in graph
            .formulas
            .values()
            .map(Vec::as_slice)
            .chain(graph.areas.iter().map(|members| &**members))
        {
            if cancelled() {
                return Err(());
            }
            dependencies.extend(nodes.iter().filter_map(index));
            dependency_starts.push(dependencies.len());
        }
        let mut dependent_starts = vec![0_usize; node_count + 1];
        for dependency in &dependencies {
            dependent_starts[*dependency as usize + 1] += 1;
        }
        for position in 0..node_count {
            dependent_starts[position + 1] += dependent_starts[position];
        }
        let mut next = dependent_starts.clone();
        let mut dependents = vec![0_u32; dependencies.len()];
        for node in 0..node_count {
            if cancelled() {
                return Err(());
            }
            for dependency in &dependencies[dependency_starts[node]..dependency_starts[node + 1]] {
                let slot = &mut next[*dependency as usize];
                dependents[*slot] = node as u32;
                *slot += 1;
            }
        }
        Ok(Self {
            formulas,
            dependency_starts,
            dependencies,
            dependent_starts,
            dependents,
        })
    }

    fn node_count(&self) -> usize {
        self.dependency_starts.len() - 1
    }

    fn is_formula(&self, node: u32) -> bool {
        (node as usize) < self.formulas.len()
    }

    fn dependencies(&self, node: u32) -> &[u32] {
        &self.dependencies
            [self.dependency_starts[node as usize]..self.dependency_starts[node as usize + 1]]
    }

    fn dependents(&self, node: u32) -> &[u32] {
        &self.dependents
            [self.dependent_starts[node as usize]..self.dependent_starts[node as usize + 1]]
    }
}

/// Orders formulas so each follows everything it waits for, choosing the first ready formula in
/// cell order at every step.
///
/// An area completes as soon as its last member does, so a formula becomes ready at the same step
/// as if it waited for every cell of its areas directly.
pub(super) fn schedule_cancellable(
    dependencies: &DependencyGraph,
    cancelled: &impl Fn() -> bool,
) -> Result<Schedule, ()> {
    let graph = DenseGraph::build(dependencies, cancelled)?;
    let mut remaining = (0..graph.node_count() as u32)
        .map(|node| graph.dependencies(node).len())
        .collect::<Vec<_>>();
    let mut ready = BinaryHeap::new();
    let mut completed_areas = Vec::new();
    for node in 0..graph.node_count() as u32 {
        if remaining[node as usize] == 0 {
            if graph.is_formula(node) {
                ready.push(Reverse(node));
            } else {
                completed_areas.push(node);
            }
        }
    }
    let mut order = Vec::with_capacity(graph.formulas.len());
    let complete = |node: u32,
                    remaining: &mut [usize],
                    ready: &mut BinaryHeap<Reverse<u32>>,
                    completed_areas: &mut Vec<u32>| {
        for dependent in graph.dependents(node) {
            let count = &mut remaining[*dependent as usize];
            *count -= 1;
            if *count == 0 {
                if graph.is_formula(*dependent) {
                    ready.push(Reverse(*dependent));
                } else {
                    completed_areas.push(*dependent);
                }
            }
        }
    };
    loop {
        while let Some(area) = completed_areas.pop() {
            if cancelled() {
                return Err(());
            }
            complete(area, &mut remaining, &mut ready, &mut completed_areas);
        }
        let Some(Reverse(formula)) = ready.pop() else {
            break;
        };
        if cancelled() {
            return Err(());
        }
        order.push(graph.formulas[formula as usize]);
        complete(formula, &mut remaining, &mut ready, &mut completed_areas);
    }

    let unresolved = (0..graph.node_count() as u32)
        .filter(|node| remaining[*node as usize] > 0)
        .collect::<Vec<_>>();
    let cycle_nodes = cycle_members(&graph, &unresolved, &remaining, cancelled)?;
    let mut cycle_cells = BTreeSet::new();
    let mut blocked_cells = BTreeSet::new();
    for node in unresolved {
        if !graph.is_formula(node) {
            continue;
        }
        let cell = graph.formulas[node as usize];
        if cycle_nodes.contains(&node) {
            cycle_cells.insert(cell);
        } else {
            blocked_cells.insert(cell);
        }
    }
    Ok(Schedule {
        order,
        cycle_cells,
        blocked_cells,
    })
}

/// Returns the unresolved nodes on a cycle: those in a strongly connected component of more than
/// one node, and a formula that waits for itself.
///
/// No area reaches itself, since each holds only cells and areas within its own range, so a
/// formula whose only cycle runs through areas waits for itself.
fn cycle_members(
    graph: &DenseGraph,
    unresolved: &[u32],
    remaining: &[usize],
    cancelled: &impl Fn() -> bool,
) -> Result<HashSet<u32>, ()> {
    let is_unresolved = |node: u32| remaining[node as usize] > 0;
    let mut visited = vec![false; graph.node_count()];
    let mut finished = Vec::with_capacity(unresolved.len());
    for start in unresolved {
        if cancelled() {
            return Err(());
        }
        if visited[*start as usize] {
            continue;
        }
        let mut pending = vec![(*start, false)];
        while let Some((node, expanded)) = pending.pop() {
            if cancelled() {
                return Err(());
            }
            if expanded {
                finished.push(node);
                continue;
            }
            if std::mem::replace(&mut visited[node as usize], true) {
                continue;
            }
            pending.push((node, true));
            for next in graph.dependencies(node).iter().rev() {
                if is_unresolved(*next) && !visited[*next as usize] {
                    pending.push((*next, false));
                }
            }
        }
    }

    let mut assigned = vec![false; graph.node_count()];
    let mut result = HashSet::new();
    for start in finished.into_iter().rev() {
        if cancelled() {
            return Err(());
        }
        if std::mem::replace(&mut assigned[start as usize], true) {
            continue;
        }
        let mut component = vec![start];
        let mut pending = vec![start];
        while let Some(node) = pending.pop() {
            if cancelled() {
                return Err(());
            }
            for dependent in graph.dependents(node) {
                if is_unresolved(*dependent)
                    && !std::mem::replace(&mut assigned[*dependent as usize], true)
                {
                    component.push(*dependent);
                    pending.push(*dependent);
                }
            }
        }
        let is_cycle = component.len() > 1 || graph.dependencies(start).contains(&start);
        if is_cycle {
            result.extend(component);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells_graph(edges: impl IntoIterator<Item = (CellId, Vec<CellId>)>) -> DependencyGraph {
        let mut graph = DependencyGraph::default();
        for (cell, dependencies) in edges {
            graph.insert_cells(cell, dependencies);
        }
        graph
    }

    #[test]
    fn distinguishes_cycle_members_from_downstream_cells() {
        let a = (0, 1, 1);
        let b = (0, 1, 2);
        let downstream = (0, 1, 3);
        let independent = (0, 1, 4);
        let dependencies = cells_graph([
            (a, vec![b]),
            (b, vec![a]),
            (downstream, vec![a]),
            (independent, Vec::new()),
        ]);

        let result = schedule(&dependencies);

        assert_eq!(result.order, vec![independent]);
        assert_eq!(result.cycle_cells, BTreeSet::from([a, b]));
        assert_eq!(result.blocked_cells, BTreeSet::from([downstream]));
    }

    #[test]
    fn large_cycle_is_classified_without_recursive_graph_walks() {
        let cell_count = 10_000_u32;
        let dependencies = cells_graph((1..=cell_count).map(|column| {
            let next = if column == cell_count { 1 } else { column + 1 };
            ((0, 1, column), vec![(0, 1, next)])
        }));

        let result = schedule(&dependencies);

        assert_eq!(result.cycle_cells.len(), cell_count as usize);
        assert!(result.blocked_cells.is_empty());
        assert!(result.order.is_empty());
    }

    #[test]
    fn scheduling_polls_cancellation_while_building_edges() {
        let dependencies = cells_graph([
            ((0, 1, 1), vec![(0, 1, 2), (0, 1, 3)]),
            ((0, 1, 2), Vec::new()),
            ((0, 1, 3), Vec::new()),
        ]);
        let polls = std::cell::Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 3
        };

        assert!(schedule_cancellable(&dependencies, &cancelled).is_err());
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn areas_order_formulas_as_their_cells_would() {
        // C1 reads an area holding A1 and B2, and A1 waits for B2: through the area, C1 still
        // follows both, and the area alone adds no cell to the order.
        let a1 = (0, 1, 1);
        let b2 = (0, 2, 2);
        let c1 = (0, 1, 3);
        let mut graph = DependencyGraph::default();
        let area = graph.add_area(vec![DependencyNode::Cell(a1), DependencyNode::Cell(b2)]);
        graph.insert(c1, vec![area]);
        graph.insert_cells(a1, vec![b2]);
        graph.insert_cells(b2, Vec::new());

        let result = schedule(&graph);

        assert_eq!(result.order, vec![b2, a1, c1]);
        assert!(result.cycle_cells.is_empty());
        assert!(result.blocked_cells.is_empty());
    }

    #[test]
    fn a_formula_inside_its_own_area_is_a_cycle_and_its_readers_are_blocked() {
        let a1 = (0, 1, 1);
        let a2 = (0, 2, 1);
        let reader = (0, 1, 2);
        let mut graph = DependencyGraph::default();
        let area = graph.add_area(vec![DependencyNode::Cell(a1), DependencyNode::Cell(a2)]);
        graph.insert(a2, vec![area]);
        graph.insert_cells(a1, Vec::new());
        graph.insert(reader, vec![area]);

        let result = schedule(&graph);

        assert_eq!(result.order, vec![a1]);
        assert_eq!(result.cycle_cells, BTreeSet::from([a2]));
        assert_eq!(result.blocked_cells, BTreeSet::from([reader]));
    }
}
