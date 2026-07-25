use std::collections::{BTreeMap, BTreeSet};

use super::runtime::CellId;

pub(super) type DependencyGraph = BTreeMap<CellId, Vec<CellId>>;

pub(super) struct Schedule {
    pub(super) order: Vec<CellId>,
    pub(super) cycle_cells: BTreeSet<CellId>,
    pub(super) blocked_cells: BTreeSet<CellId>,
}

pub(super) fn schedule(dependencies: &DependencyGraph) -> Schedule {
    let mut dependents: BTreeMap<CellId, Vec<CellId>> = BTreeMap::new();
    let mut indegree = BTreeMap::new();
    for (cell, cell_dependencies) in dependencies {
        let tracked_dependencies = cell_dependencies
            .iter()
            .filter(|dependency| dependencies.contains_key(dependency))
            .count();
        indegree.insert(*cell, tracked_dependencies);
        for dependency in cell_dependencies {
            if dependencies.contains_key(dependency) {
                dependents.entry(*dependency).or_default().push(*cell);
            }
        }
    }

    let mut ready: BTreeSet<CellId> = indegree
        .iter()
        .filter_map(|(cell, degree)| (*degree == 0).then_some(*cell))
        .collect();
    let mut order = Vec::with_capacity(dependencies.len());
    while let Some(cell) = ready.pop_first() {
        order.push(cell);
        if let Some(children) = dependents.get(&cell) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(*child);
                    }
                }
            }
        }
    }

    let unresolved: BTreeSet<CellId> = indegree
        .iter()
        .filter_map(|(cell, degree)| (*degree > 0).then_some(*cell))
        .collect();
    let cycle_cells = cycle_members(&unresolved, dependencies, &dependents);
    let blocked_cells = unresolved.difference(&cycle_cells).copied().collect();
    Schedule {
        order,
        cycle_cells,
        blocked_cells,
    }
}

fn cycle_members(
    unresolved: &BTreeSet<CellId>,
    dependencies: &DependencyGraph,
    dependents: &BTreeMap<CellId, Vec<CellId>>,
) -> BTreeSet<CellId> {
    let mut visited = BTreeSet::new();
    let mut finished = Vec::with_capacity(unresolved.len());
    for start in unresolved {
        if visited.contains(start) {
            continue;
        }
        let mut pending = vec![(*start, false)];
        while let Some((cell, expanded)) = pending.pop() {
            if expanded {
                finished.push(cell);
                continue;
            }
            if !visited.insert(cell) {
                continue;
            }
            pending.push((cell, true));
            if let Some(next) = dependencies.get(&cell) {
                pending.extend(
                    next.iter()
                        .rev()
                        .filter(|item| unresolved.contains(item))
                        .map(|item| (*item, false)),
                );
            }
        }
    }

    let mut assigned = BTreeSet::new();
    let mut result = BTreeSet::new();
    for start in finished.into_iter().rev() {
        if !assigned.insert(start) {
            continue;
        }
        let mut component = Vec::new();
        let mut pending = vec![start];
        while let Some(cell) = pending.pop() {
            component.push(cell);
            if let Some(next) = dependents.get(&cell) {
                for dependent in next {
                    if unresolved.contains(dependent) && assigned.insert(*dependent) {
                        pending.push(*dependent);
                    }
                }
            }
        }
        let is_cycle = component.len() > 1
            || dependencies
                .get(&start)
                .is_some_and(|items| items.contains(&start));
        if is_cycle {
            result.extend(component);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_cycle_members_from_downstream_cells() {
        let a = (0, 1, 1);
        let b = (0, 1, 2);
        let downstream = (0, 1, 3);
        let independent = (0, 1, 4);
        let dependencies = BTreeMap::from([
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
        let mut dependencies = BTreeMap::new();
        for column in 1..=cell_count {
            let current = (0, 1, column);
            let next = (0, 1, if column == cell_count { 1 } else { column + 1 });
            dependencies.insert(current, vec![next]);
        }

        let result = schedule(&dependencies);

        assert_eq!(result.cycle_cells.len(), cell_count as usize);
        assert!(result.blocked_cells.is_empty());
        assert!(result.order.is_empty());
    }
}
