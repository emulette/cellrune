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
