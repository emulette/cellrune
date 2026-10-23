use crate::CellRange;

/// A worksheet-level auto-filter that belongs to no Excel table.
///
/// The filtered range, the columns that carry filter settings, and which of them declare a filter
/// condition are modeled. The conditions themselves and the sort state stay in the source
/// worksheet XML, which a document-backed writer preserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetAutoFilter {
    range: CellRange,
    filter_column_ids: Vec<u32>,
    filtered_column_ids: Vec<u32>,
}

impl SheetAutoFilter {
    pub(crate) const fn from_xlsx(
        range: CellRange,
        filter_column_ids: Vec<u32>,
        filtered_column_ids: Vec<u32>,
    ) -> Self {
        Self {
            range,
            filter_column_ids,
            filtered_column_ids,
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

    /// Returns the `filterColumn` identifiers, in declaration order, whose column declares a
    /// filter condition (`filters`, `customFilters`, `top10`, `dynamicFilter`, `colorFilter`, or
    /// `iconFilter`). A filter column without one only configures its button.
    ///
    /// Excel stores the rows a filter hides as hidden rows, so the hidden rows of
    /// [`crate::Sheet::hidden_rows`] inside [`Self::range`] count as filtered when this list is not
    /// empty.
    pub fn filtered_column_ids(&self) -> &[u32] {
        &self.filtered_column_ids
    }

    /// Returns this filter over any `range`: each filter column of a worksheet column the range
    /// still holds keeps its settings, renumbered from the new first column, and the others leave.
    pub(crate) fn respanned(&self, range: CellRange) -> Self {
        let old_first = self.range.start().column().get();
        let new_first = range.start().column().get();
        let new_last = range.end().column().get();
        let kept = |column_ids: &[u32]| {
            column_ids
                .iter()
                .map(|column_id| old_first + column_id)
                .filter(|column| (new_first..=new_last).contains(column))
                .map(|column| column - new_first)
                .collect()
        };
        Self {
            range,
            filter_column_ids: kept(&self.filter_column_ids),
            filtered_column_ids: kept(&self.filtered_column_ids),
        }
    }

    /// Returns this filter over `range`, which keeps its rows and contains its columns, with each
    /// filter column renumbered from the new first column.
    pub(crate) fn widened(&self, range: CellRange) -> Self {
        let shift = self.range.start().column().get() - range.start().column().get();
        let shifted = |column_ids: &[u32]| {
            column_ids
                .iter()
                .map(|column_id| column_id + shift)
                .collect()
        };
        Self {
            range,
            filter_column_ids: shifted(&self.filter_column_ids),
            filtered_column_ids: shifted(&self.filtered_column_ids),
        }
    }
}
