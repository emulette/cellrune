use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use crate::{
    CellAddress, CellRange, EXCEL_MAX_ROWS, Sheet, SheetId, TableColumnId, TableId, ValidationError,
};

use super::case_insensitive_key;
#[cfg(test)]
use super::clone_map_cancellable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableLocation {
    pub(crate) sheet_index: usize,
    pub(crate) table_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableColumnLocation {
    pub(crate) table: TableLocation,
    pub(crate) column_index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct TableRangeIndex<T> {
    nodes: BTreeMap<u32, BTreeMap<u32, (u32, T)>>,
    subtree_columns: BTreeMap<u32, BTreeMap<u32, u32>>,
}

#[derive(Debug, Clone, Copy)]
struct SpatialEntry<T> {
    row_start: u32,
    row_end: u32,
    column_start: u32,
    column_end: u32,
    value: T,
}

impl<T> Default for TableRangeIndex<T> {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::new(),
            subtree_columns: BTreeMap::new(),
        }
    }
}

impl<T: Copy> TableRangeIndex<T> {
    pub(crate) fn insert(&mut self, range: CellRange, value: T) {
        let entry = SpatialEntry {
            row_start: range.start().row().get(),
            row_end: range.end().row().get(),
            column_start: range.start().column().get(),
            column_end: range.end().column().get(),
            value,
        };
        self.insert_at(1, 1, EXCEL_MAX_ROWS, entry);
    }

    fn insert_at(&mut self, node: u32, node_start: u32, node_end: u32, entry: SpatialEntry<T>) {
        insert_interval(
            self.subtree_columns.entry(node).or_default(),
            entry.column_start,
            entry.column_end,
        );
        if entry.row_start <= node_start && node_end <= entry.row_end {
            let replaced = self
                .nodes
                .entry(node)
                .or_default()
                .insert(entry.column_start, (entry.column_end, entry.value));
            debug_assert!(
                replaced.is_none(),
                "overlap validation makes column starts unique within a row segment"
            );
            return;
        }
        let middle = node_start + (node_end - node_start) / 2;
        if entry.row_start <= middle {
            self.insert_at(node * 2, node_start, middle, entry);
        }
        if entry.row_end > middle {
            self.insert_at(node * 2 + 1, middle + 1, node_end, entry);
        }
    }

    fn containing(&self, address: CellAddress) -> Option<T> {
        let row = address.row().get();
        let column = address.column().get();
        let mut node = 1_u32;
        let mut node_start = 1_u32;
        let mut node_end = EXCEL_MAX_ROWS;
        loop {
            if let Some(columns) = self.nodes.get(&node)
                && let Some((_, (column_end, location))) = columns.range(..=column).next_back()
                && *column_end >= column
            {
                return Some(*location);
            }
            if node_start == node_end {
                return None;
            }
            let middle = node_start + (node_end - node_start) / 2;
            if row <= middle {
                node *= 2;
                node_end = middle;
            } else {
                node = node * 2 + 1;
                node_start = middle + 1;
            }
        }
    }

    pub(crate) fn intersects(&self, range: CellRange) -> bool {
        self.intersects_at(1, 1, EXCEL_MAX_ROWS, range)
    }

    fn intersects_at(&self, node: u32, node_start: u32, node_end: u32, range: CellRange) -> bool {
        if range.end().row().get() < node_start || range.start().row().get() > node_end {
            return false;
        }
        let Some(subtree_columns) = self.subtree_columns.get(&node) else {
            return false;
        };
        if !intervals_intersect(
            subtree_columns,
            range.start().column().get(),
            range.end().column().get(),
        ) {
            return false;
        }
        if self.nodes.get(&node).is_some_and(|columns| {
            columns
                .range(..=range.end().column().get())
                .next_back()
                .is_some_and(|(_, (active_end, _))| *active_end >= range.start().column().get())
        }) {
            return true;
        }
        if node_start == node_end {
            return false;
        }
        let middle = node_start + (node_end - node_start) / 2;
        self.intersects_at(node * 2, node_start, middle, range)
            || self.intersects_at(node * 2 + 1, middle + 1, node_end, range)
    }

    #[cfg(test)]
    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut nodes = BTreeMap::new();
        for (node, columns) in &self.nodes {
            if cancelled() {
                return Err(());
            }
            nodes.insert(*node, clone_map_cancellable(columns, cancelled)?);
        }
        let mut subtree_columns = BTreeMap::new();
        for (node, columns) in &self.subtree_columns {
            if cancelled() {
                return Err(());
            }
            subtree_columns.insert(*node, clone_map_cancellable(columns, cancelled)?);
        }
        Ok(Self {
            nodes,
            subtree_columns,
        })
    }
}

fn intervals_intersect(intervals: &BTreeMap<u32, u32>, start: u32, end: u32) -> bool {
    intervals
        .range(..=end)
        .next_back()
        .is_some_and(|(_, interval_end)| *interval_end >= start)
}

fn insert_interval(intervals: &mut BTreeMap<u32, u32>, mut start: u32, mut end: u32) {
    if let Some((previous_start, previous_end)) = intervals
        .range(..=start)
        .next_back()
        .map(|(interval_start, interval_end)| (*interval_start, *interval_end))
        && previous_end.saturating_add(1) >= start
    {
        start = previous_start;
        end = end.max(previous_end);
        intervals.remove(&previous_start);
    }
    loop {
        let Some((next_start, next_end)) = intervals
            .range(start..)
            .next()
            .map(|(interval_start, interval_end)| (*interval_start, *interval_end))
        else {
            break;
        };
        if next_start > end.saturating_add(1) {
            break;
        }
        end = end.max(next_end);
        intervals.remove(&next_start);
    }
    intervals.insert(start, end);
}

#[derive(Debug, Clone, Default)]
pub(super) struct TableIndex {
    display_names: BTreeMap<Box<str>, TableLocation>,
    table_ids: BTreeMap<TableId, TableLocation>,
    column_ids: BTreeMap<(TableId, TableColumnId), TableColumnLocation>,
    column_names: BTreeMap<TableId, BTreeMap<Box<str>, usize>>,
    containing_tables: BTreeMap<SheetId, TableRangeIndex<TableLocation>>,
}

pub(super) enum TableIndexBuildError {
    Validation(ValidationError),
    Cancelled,
}

impl From<ValidationError> for TableIndexBuildError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl TableIndex {
    pub(super) fn new_cancellable(
        sheets: &[Sheet],
        defined_name_keys: &BTreeSet<Box<str>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, TableIndexBuildError> {
        let mut index = Self::default();
        for (sheet_index, sheet) in sheets.iter().enumerate() {
            if cancelled() {
                return Err(TableIndexBuildError::Cancelled);
            }
            let mut programmatic_names = BTreeSet::<Box<str>>::new();
            let mut locations = Vec::with_capacity(sheet.tables().len());
            for (table_index, table) in sheet.tables().iter().enumerate() {
                if cancelled() {
                    return Err(TableIndexBuildError::Cancelled);
                }
                let location = TableLocation {
                    sheet_index,
                    table_index,
                };
                if index.table_ids.insert(table.id(), location).is_some() {
                    return Err(ValidationError::DuplicateTableId {
                        id: table.id().get(),
                    }
                    .into());
                }
                let display_key = Box::<str>::from(table.display_name().lookup_key());
                if defined_name_keys.contains(display_key.as_ref()) {
                    return Err(ValidationError::TableDisplayNameConflictsWithDefinedName {
                        name: table.display_name().as_str().to_owned(),
                    }
                    .into());
                }
                if index.display_names.insert(display_key, location).is_some() {
                    return Err(ValidationError::DuplicateTableDisplayName {
                        name: table.display_name().as_str().to_owned(),
                    }
                    .into());
                }
                let programmatic_key = Box::<str>::from(table.name().lookup_key());
                if !programmatic_names.insert(programmatic_key) {
                    return Err(ValidationError::DuplicateTableProgrammaticName {
                        name: table.name().as_str().to_owned(),
                    }
                    .into());
                }
                let mut column_names = BTreeMap::new();
                for (column_index, column) in table.columns().iter().enumerate() {
                    if cancelled() {
                        return Err(TableIndexBuildError::Cancelled);
                    }
                    let column_location = TableColumnLocation {
                        table: location,
                        column_index,
                    };
                    index
                        .column_ids
                        .insert((table.id(), column.column_id()), column_location);
                    if column_names
                        .insert(Box::from(case_insensitive_key(column.name())), column_index)
                        .is_some()
                    {
                        return Err(ValidationError::DuplicateTableColumnName {
                            name: column.name().to_owned(),
                        }
                        .into());
                    }
                }
                index.column_names.insert(table.id(), column_names);
                locations.push(location);
            }
            let locations = validate_non_overlapping(sheets, sheet_index, locations, cancelled)?;
            let mut spatial = TableRangeIndex::default();
            for location in locations {
                if cancelled() {
                    return Err(TableIndexBuildError::Cancelled);
                }
                let range = sheets[location.sheet_index].tables()[location.table_index].range();
                spatial.insert(range, location);
            }
            index.containing_tables.insert(sheet.id(), spatial);
        }
        Ok(index)
    }

    #[cfg(test)]
    pub(super) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut column_names = BTreeMap::new();
        for (table_id, names) in &self.column_names {
            if cancelled() {
                return Err(());
            }
            column_names.insert(*table_id, clone_map_cancellable(names, cancelled)?);
        }
        let mut containing_tables = BTreeMap::new();
        for (sheet_id, spatial) in &self.containing_tables {
            if cancelled() {
                return Err(());
            }
            containing_tables.insert(*sheet_id, spatial.clone_cancellable(cancelled)?);
        }
        Ok(Self {
            display_names: clone_map_cancellable(&self.display_names, cancelled)?,
            table_ids: clone_map_cancellable(&self.table_ids, cancelled)?,
            column_ids: clone_map_cancellable(&self.column_ids, cancelled)?,
            column_names,
            containing_tables,
        })
    }

    pub(crate) fn by_display_name(&self, name: &str) -> Option<TableLocation> {
        let key = case_insensitive_key(name);
        self.display_names.get(key.as_str()).copied()
    }

    pub(crate) fn by_id(&self, table_id: TableId) -> Option<TableLocation> {
        self.table_ids.get(&table_id).copied()
    }

    pub(crate) fn column_by_id(
        &self,
        table_id: TableId,
        column_id: TableColumnId,
    ) -> Option<TableColumnLocation> {
        self.column_ids.get(&(table_id, column_id)).copied()
    }

    pub(crate) fn column_by_name(
        &self,
        table_id: TableId,
        name: &str,
    ) -> Option<TableColumnLocation> {
        let table = self.by_id(table_id)?;
        let key = case_insensitive_key(name);
        let column_index = *self.column_names.get(&table_id)?.get(key.as_str())?;
        Some(TableColumnLocation {
            table,
            column_index,
        })
    }

    pub(crate) fn containing(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<TableLocation> {
        self.containing_tables.get(&sheet_id)?.containing(address)
    }
}

fn validate_non_overlapping(
    sheets: &[Sheet],
    sheet_index: usize,
    mut locations: Vec<TableLocation>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<TableLocation>, TableIndexBuildError> {
    if cancelled() {
        return Err(TableIndexBuildError::Cancelled);
    }
    locations.sort_unstable_by_key(|location| {
        let table = &sheets[location.sheet_index].tables()[location.table_index];
        (table.range().start(), table.range().end(), table.id())
    });
    let mut active_columns = BTreeMap::<u32, (u32, TableId)>::new();
    let mut active_expiry = BinaryHeap::<Reverse<(u32, u32)>>::new();
    for location in &locations {
        if cancelled() {
            return Err(TableIndexBuildError::Cancelled);
        }
        let table = &sheets[location.sheet_index].tables()[location.table_index];
        let range = table.range();
        let start_row = range.start().row().get();
        while let Some(Reverse((end_row, column_start))) = active_expiry.peek().copied() {
            if end_row >= start_row {
                break;
            }
            active_expiry.pop();
            active_columns.remove(&column_start);
        }
        let column_start = range.start().column().get();
        let column_end = range.end().column().get();
        if let Some((_, (active_end, first_table_id))) =
            active_columns.range(..=column_end).next_back()
            && *active_end >= column_start
        {
            return Err(ValidationError::OverlappingTables {
                sheet_id: sheets[sheet_index].id().get(),
                first_table_id: first_table_id.get(),
                second_table_id: table.id().get(),
            }
            .into());
        }
        active_columns.insert(column_start, (column_end, table.id()));
        active_expiry.push(Reverse((range.end().row().get(), column_start)));
    }
    Ok(locations)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{TableLocation, TableRangeIndex, insert_interval, intervals_intersect};
    use crate::{CellAddress, CellRange, EXCEL_MAX_ROWS};
    use std::collections::BTreeMap;

    fn range(start: &str, end: &str) -> CellRange {
        CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    }

    #[test]
    fn spatial_index_resolves_row_and_column_intervals_without_scanning_tables() {
        let first = TableLocation {
            sheet_index: 0,
            table_index: 0,
        };
        let second = TableLocation {
            sheet_index: 0,
            table_index: 1,
        };
        let mut index = TableRangeIndex::default();
        index.insert(range("A1", &format!("A{EXCEL_MAX_ROWS}")), first);
        index.insert(range("B500", "D700"), second);

        assert_eq!(
            index.containing(CellAddress::from_a1("A1048576").expect("address")),
            Some(first)
        );
        assert_eq!(
            index.containing(CellAddress::from_a1("C600").expect("address")),
            Some(second)
        );
        assert_eq!(
            index.containing(CellAddress::from_a1("E600").expect("address")),
            None
        );
        assert!(index.intersects(range("A1048576", "A1048576")));
        assert!(index.intersects(range("C650", "F800")));
        assert!(!index.intersects(range("E500", "F700")));
    }

    #[test]
    fn spatial_index_clone_polls_cancellation_inside_node_maps() {
        let mut index = TableRangeIndex::default();
        index.insert(
            range("A1", &format!("A{EXCEL_MAX_ROWS}")),
            TableLocation {
                sheet_index: 0,
                table_index: 0,
            },
        );
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        };

        assert!(index.clone_cancellable(&cancelled).is_err());
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn column_summaries_merge_and_prune_disjoint_queries() {
        let mut intervals = BTreeMap::new();
        insert_interval(&mut intervals, 5, 7);
        insert_interval(&mut intervals, 1, 2);
        insert_interval(&mut intervals, 3, 4);
        insert_interval(&mut intervals, 10, 12);

        assert_eq!(intervals, BTreeMap::from([(1, 7), (10, 12)]));
        assert!(intervals_intersect(&intervals, 7, 10));
        assert!(!intervals_intersect(&intervals, 8, 9));
    }
}
