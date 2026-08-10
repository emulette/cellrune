use std::collections::{BTreeMap, BTreeSet};

use super::runtime::CellId;

pub(super) type DependencyGraph = BTreeMap<CellId, Vec<CellId>>;

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

pub(super) fn schedule_cancellable(
    dependencies: &DependencyGraph,
    cancelled: &impl Fn() -> bool,
) -> Result<Schedule, ()> {
    #[cfg(test)]
    super::work_counter::schedule_build();
    let mut dependents: BTreeMap<CellId, Vec<CellId>> = BTreeMap::new();
    let mut indegree = BTreeMap::new();
    for (cell, cell_dependencies) in dependencies {
        if cancelled() {
            return Err(());
        }
        let mut tracked_dependencies = 0_usize;
        for dependency in cell_dependencies {
            if cancelled() {
                return Err(());
            }
            if dependencies.contains_key(dependency) {
                tracked_dependencies += 1;
                dependents.entry(*dependency).or_default().push(*cell);
            }
        }
        indegree.insert(*cell, tracked_dependencies);
    }

    let mut ready = BTreeSet::new();
    for (cell, degree) in &indegree {
        if cancelled() {
            return Err(());
        }
        if *degree == 0 {
            ready.insert(*cell);
        }
    }
    let mut order = Vec::with_capacity(dependencies.len());
    while let Some(cell) = ready.pop_first() {
        if cancelled() {
            return Err(());
        }
        order.push(cell);
        if let Some(children) = dependents.get(&cell) {
            for child in children {
                if cancelled() {
                    return Err(());
                }
                if let Some(degree) = indegree.get_mut(child) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(*child);
                    }
                }
            }
        }
    }

    let mut unresolved = BTreeSet::new();
    for (cell, degree) in &indegree {
        if cancelled() {
            return Err(());
        }
        if *degree > 0 {
            unresolved.insert(*cell);
        }
    }
    let cycle_cells = cycle_members(&unresolved, dependencies, &dependents, cancelled)?;
    let mut blocked_cells = BTreeSet::new();
    for cell in unresolved.difference(&cycle_cells) {
        if cancelled() {
            return Err(());
        }
        blocked_cells.insert(*cell);
    }
    Ok(Schedule {
        order,
        cycle_cells,
        blocked_cells,
    })
}

fn cycle_members(
    unresolved: &BTreeSet<CellId>,
    dependencies: &DependencyGraph,
    dependents: &BTreeMap<CellId, Vec<CellId>>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CellId>, ()> {
    let mut visited = BTreeSet::new();
    let mut finished = Vec::with_capacity(unresolved.len());
    for start in unresolved {
        if cancelled() {
            return Err(());
        }
        if visited.contains(start) {
            continue;
        }
        let mut pending = vec![(*start, false)];
        while let Some((cell, expanded)) = pending.pop() {
            if cancelled() {
                return Err(());
            }
            if expanded {
                finished.push(cell);
                continue;
            }
            if !visited.insert(cell) {
                continue;
            }
            pending.push((cell, true));
            if let Some(next) = dependencies.get(&cell) {
                for item in next.iter().rev() {
                    if cancelled() {
                        return Err(());
                    }
                    if unresolved.contains(item) {
                        pending.push((*item, false));
                    }
                }
            }
        }
    }

    let mut assigned = BTreeSet::new();
    let mut result = BTreeSet::new();
    for start in finished.into_iter().rev() {
        if cancelled() {
            return Err(());
        }
        if !assigned.insert(start) {
            continue;
        }
        let mut component = Vec::new();
        let mut pending = vec![start];
        while let Some(cell) = pending.pop() {
            if cancelled() {
                return Err(());
            }
            component.push(cell);
            if let Some(next) = dependents.get(&cell) {
                for dependent in next {
                    if cancelled() {
                        return Err(());
                    }
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
            for cell in component {
                if cancelled() {
                    return Err(());
                }
                result.insert(cell);
            }
        }
    }
    Ok(result)
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

    #[test]
    fn scheduling_polls_cancellation_while_building_edges() {
        let dependencies = BTreeMap::from([
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
}
