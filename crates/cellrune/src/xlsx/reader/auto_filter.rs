use std::collections::BTreeSet;

use super::table::parse_reference;
use crate::{CellRange, SheetAutoFilter};

/// Collects the worksheet-level `<autoFilter>` and the `colId` of each of its `<filterColumn>`
/// children.
///
/// A missing or malformed reference, a repeated declaration, or an invalid, out-of-range, or
/// repeated `colId` leaves the filter unmodeled. Its XML is preserved either way, so such a filter
/// keeps its source form and cannot be edited.
#[derive(Debug, Default)]
pub(super) struct SheetAutoFilterCollector {
    seen: bool,
    valid: bool,
    open_depth: Option<u64>,
    range: Option<CellRange>,
    filter_column_ids: Vec<u32>,
    seen_filter_column_ids: BTreeSet<u32>,
}

impl SheetAutoFilterCollector {
    /// Records one `<autoFilter>` worksheet child; `depth` is set when it has children.
    pub(super) fn start(&mut self, reference: Option<&str>, depth: Option<u64>) {
        self.range = reference.and_then(parse_reference);
        self.valid = !self.seen && self.range.is_some();
        self.seen = true;
        self.open_depth = depth;
    }

    /// Returns whether an element at `depth` is a direct child of the open `<autoFilter>`.
    pub(super) fn is_child_depth(&self, depth: u64) -> bool {
        self.open_depth.is_some_and(|parent| depth == parent + 1)
    }

    /// Records the `colId` of one `<filterColumn>` child.
    pub(super) fn filter_column(&mut self, column_id: Option<&str>) {
        let column_id = column_id.and_then(|value| value.parse::<u32>().ok());
        match (column_id, self.range) {
            (Some(column_id), Some(range))
                if column_id < range.width() && self.seen_filter_column_ids.insert(column_id) =>
            {
                self.filter_column_ids.push(column_id);
            }
            _ => self.valid = false,
        }
    }

    /// Closes the open `<autoFilter>` when `depth` is its depth.
    pub(super) fn end(&mut self, depth: u64) {
        if self.open_depth == Some(depth) {
            self.open_depth = None;
        }
    }

    pub(super) fn finish(self) -> Option<SheetAutoFilter> {
        let range = self.range.filter(|_| self.valid)?;
        Some(SheetAutoFilter::from_xlsx(range, self.filter_column_ids))
    }
}
