use crate::CellRange;

/// A worksheet-level auto-filter that belongs to no Excel table.
///
/// Only the filtered range and the columns that carry filter settings are modeled. Filter
/// criteria and sort state stay in the source worksheet XML, which a document-backed writer
/// preserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetAutoFilter {
    range: CellRange,
    filter_column_ids: Vec<u32>,
}

impl SheetAutoFilter {
    pub(crate) const fn from_xlsx(range: CellRange, filter_column_ids: Vec<u32>) -> Self {
        Self {
            range,
            filter_column_ids,
        }
    }

    /// Returns the filtered range, including its header row.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns the zero-based `filterColumn` identifiers in declaration order.
    ///
    /// Each identifier is a column offset from the first column of [`Self::range`].
    pub fn filter_column_ids(&self) -> &[u32] {
        &self.filter_column_ids
    }

    /// Returns this filter over `range`, which keeps its rows and contains its columns, with each
    /// filter column renumbered from the new first column.
    pub(crate) fn widened(&self, range: CellRange) -> Self {
        let shift = self.range.start().column().get() - range.start().column().get();
        Self {
            range,
            filter_column_ids: self
                .filter_column_ids
                .iter()
                .map(|column_id| column_id + shift)
                .collect(),
        }
    }
}
