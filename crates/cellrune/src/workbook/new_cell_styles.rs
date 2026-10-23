use std::collections::BTreeMap;

use crate::{CellAddress, Column, NumberFormat, Row};

/// The source styles that a cell written at an empty position receives.
///
/// A spreadsheet app formats a value entered at an empty position with the style the position
/// already carries: a blank cell's own style, else its row's style when the row declares
/// `customFormat`, else its column's style. Each style is kept as its source `cellXfs` index with
/// the number format that index resolves to.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewCellStyles {
    /// The nonzero style of each valued source cell in row-major order, which the format read
    /// model resolves. A sorted vector keeps each in 12 bytes; a map took several times that for
    /// every styled cell of a document.
    valued_cells: Vec<(CellAddress, u32)>,
    /// Whether a valued cell arrived out of row-major order, so `valued_cells` needs sorting.
    valued_cells_unsorted: bool,
    blank_cells: BTreeMap<CellAddress, usize>,
    rows: BTreeMap<Row, usize>,
    columns: Vec<(Column, Column, usize)>,
    number_formats: BTreeMap<usize, NumberFormat>,
}

impl NewCellStyles {
    /// Records the style of a source cell element that holds no value.
    pub(crate) fn insert_blank_cell(
        &mut self,
        address: CellAddress,
        index: usize,
        number_format: &NumberFormat,
    ) {
        self.blank_cells.insert(address, index);
        self.insert_number_format(index, number_format);
    }

    /// Records the nonzero style of a source cell that holds a value. Cells usually arrive in
    /// row-major order; [`Self::finish_reading`] sorts any that do not.
    pub(crate) fn insert_valued_cell(&mut self, address: CellAddress, index: usize) {
        let index = u32::try_from(index)
            .expect("a styles part within the read limits holds fewer than 2^32 cell formats");
        if self
            .valued_cells
            .last()
            .is_some_and(|(last, _)| *last >= address)
        {
            self.valued_cells_unsorted = true;
        }
        self.valued_cells.push((address, index));
    }

    /// Sorts the valued cell styles a read recorded out of order and releases spare capacity.
    pub(crate) fn finish_reading(&mut self) {
        if std::mem::take(&mut self.valued_cells_unsorted) {
            self.valued_cells
                .sort_unstable_by_key(|(address, _)| *address);
        }
        self.valued_cells.shrink_to_fit();
    }

    /// Returns the style index of a valued source cell: its recorded style, else the default.
    pub(crate) fn valued_cell_style(&self, address: CellAddress) -> usize {
        debug_assert!(!self.valued_cells_unsorted);
        self.valued_cells
            .binary_search_by_key(&address, |(cell, _)| *cell)
            .map_or(0, |position| self.valued_cells[position].1 as usize)
    }

    /// Records the style of a row that declares `customFormat`.
    pub(crate) fn insert_row(&mut self, row: Row, index: usize, number_format: &NumberFormat) {
        self.rows.insert(row, index);
        self.insert_number_format(index, number_format);
    }

    /// Records the style of an inclusive column span; the later span wins where spans overlap.
    pub(crate) fn push_columns(
        &mut self,
        first: Column,
        last: Column,
        index: usize,
        number_format: &NumberFormat,
    ) {
        self.columns.push((first, last, index));
        self.insert_number_format(index, number_format);
    }

    /// Returns the source style index and number format that a cell written at the empty
    /// `address` receives, or `None` when the default style applies.
    pub(crate) fn style(&self, address: CellAddress) -> Option<(usize, &NumberFormat)> {
        let index = self
            .blank_cells
            .get(&address)
            .or_else(|| self.rows.get(&address.row()))
            .or_else(|| {
                self.columns
                    .iter()
                    .rev()
                    .find(|(first, last, _)| (*first..=*last).contains(&address.column()))
                    .map(|(_, _, index)| index)
            })
            .copied()?;
        self.number_formats
            .get(&index)
            .map(|number_format| (index, number_format))
    }

    fn insert_number_format(&mut self, index: usize, number_format: &NumberFormat) {
        self.number_formats
            .entry(index)
            .or_insert_with(|| number_format.clone());
    }
}
