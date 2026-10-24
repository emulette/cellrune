use std::collections::BTreeMap;

use crate::grid::{GridAxis, GridEdit};
use crate::{CellAddress, Column, EXCEL_MAX_COLUMNS, NumberFormat, Row};

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
    /// The number format of the default style when it is not General, which a cell written
    /// where no other style applies receives. LibreOffice and Numbers declare General as a custom
    /// format of the default style.
    default_number_format: Option<NumberFormat>,
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

    /// Records the number format of the default style, `cellXfs` index 0.
    pub(crate) fn set_default_number_format(&mut self, number_format: &NumberFormat) {
        self.default_number_format = (number_format.id() != 0).then(|| number_format.clone());
    }

    /// Returns the number format a cell written where no other style applies receives.
    pub(crate) fn default_number_format(&self) -> NumberFormat {
        self.default_number_format.clone().unwrap_or_default()
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

    /// Returns every position that carries a cell style of its own.
    pub(crate) fn styled_positions(&self) -> impl Iterator<Item = CellAddress> + '_ {
        self.valued_cells
            .iter()
            .map(|(address, _)| *address)
            .chain(self.blank_cells.keys().copied())
    }

    /// Reports whether every row and column has the same style in both.
    pub(crate) fn same_line_styles(&self, other: &Self) -> bool {
        let columns = |styles: &Self| {
            let mut columns = vec![None; EXCEL_MAX_COLUMNS as usize + 1];
            for (first, last, index) in &styles.columns {
                for column in first.get()..=last.get() {
                    columns[column as usize] = Some(*index);
                }
            }
            columns
        };
        self.rows == other.rows && columns(self) == columns(other)
    }

    /// Returns the style index and number format of every styled position on line `line` of
    /// `axis`, which lines inserted after it inherit. `valued` gives the number format of a
    /// valued cell, whose style alone is recorded here.
    pub(crate) fn line_styles(
        &self,
        axis: GridAxis,
        line: u32,
        valued: impl Fn(CellAddress) -> Option<NumberFormat>,
    ) -> Vec<(CellAddress, usize, NumberFormat)> {
        let on_line = |address: &CellAddress| axis.of(*address) == line;
        let mut styles = self
            .valued_cells
            .iter()
            .filter(|(address, _)| on_line(address))
            .filter_map(|(address, index)| Some((*address, *index as usize, valued(*address)?)))
            .collect::<Vec<_>>();
        styles.extend(
            self.blank_cells
                .iter()
                .filter(|(address, _)| on_line(address))
                .filter_map(|(address, index)| {
                    Some((*address, *index, self.number_formats.get(index)?.clone()))
                }),
        );
        styles
    }

    /// Moves every recorded style through a structural edit. Lines an insertion adds take the
    /// row or column style of the line before them, and `inherited` gives the cell styles they
    /// take, at their new positions.
    pub(crate) fn apply_grid_edit(
        &mut self,
        edit: GridEdit,
        inherited: Vec<(CellAddress, usize, NumberFormat)>,
    ) {
        let mut valued_cells = self
            .valued_cells
            .iter()
            .filter_map(|(address, index)| Some((edit.map_address(*address)?, *index)))
            .collect::<Vec<_>>();
        valued_cells.sort_unstable_by_key(|(address, _)| *address);
        self.valued_cells = valued_cells;
        self.blank_cells = self
            .blank_cells
            .iter()
            .filter_map(|(address, index)| Some((edit.map_address(*address)?, *index)))
            .collect();
        let inherited_line = match edit {
            GridEdit::Insert { at, count, .. } if at > 1 => Some((at - 1, at..at + count)),
            _ => None,
        };
        match edit.line_axis() {
            Some(GridAxis::Rows) => {
                let mut rows = self
                    .rows
                    .iter()
                    .filter_map(|(row, index)| {
                        Some((Row::new(edit.map_line(row.get())?).ok()?, *index))
                    })
                    .collect::<BTreeMap<_, _>>();
                if let Some((before, lines)) = inherited_line.clone()
                    && let Some(index) = Row::new(before).ok().and_then(|row| self.rows.get(&row))
                {
                    for line in lines {
                        if let Ok(row) = Row::new(line) {
                            rows.insert(row, *index);
                        }
                    }
                }
                self.rows = rows;
            }
            Some(GridAxis::Columns) => {
                let mut columns = vec![None; EXCEL_MAX_COLUMNS as usize + 1];
                for (first, last, index) in &self.columns {
                    for column in first.get()..=last.get() {
                        columns[column as usize] = Some(*index);
                    }
                }
                let mut moved = vec![None; EXCEL_MAX_COLUMNS as usize + 1];
                for (column, index) in columns.iter().enumerate().skip(1) {
                    if let Some(index) = index
                        && let Some(target) = edit.map_line(column as u32)
                    {
                        moved[target as usize] = Some(*index);
                    }
                }
                if let Some((before, lines)) = inherited_line.clone() {
                    for line in lines {
                        moved[line as usize] = columns[before as usize];
                    }
                }
                self.columns = spans(&moved);
            }
            None => {}
        }
        for (address, index, number_format) in inherited {
            self.blank_cells.insert(address, index);
            self.insert_number_format(index, &number_format);
        }
    }

    fn insert_number_format(&mut self, index: usize, number_format: &NumberFormat) {
        self.number_formats
            .entry(index)
            .or_insert_with(|| number_format.clone());
    }
}

/// Collapses per-column styles, indexed from column 1, into inclusive spans.
fn spans(columns: &[Option<usize>]) -> Vec<(Column, Column, usize)> {
    let mut spans = Vec::<(Column, Column, usize)>::new();
    for (column, index) in columns.iter().enumerate().skip(1) {
        let (Some(index), Ok(column)) = (index, Column::new(column as u32)) else {
            continue;
        };
        match spans.last_mut() {
            Some((_, last, previous)) if *previous == *index && last.get() + 1 == column.get() => {
                *last = column;
            }
            _ => spans.push((column, column, *index)),
        }
    }
    spans
}
