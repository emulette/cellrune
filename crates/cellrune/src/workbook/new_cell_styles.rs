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
    /// The nonzero style of each valued source cell, which the format read model resolves.
    valued_cells: BTreeMap<CellAddress, usize>,
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

    /// Records the nonzero style of a source cell that holds a value.
    pub(crate) fn insert_valued_cell(&mut self, address: CellAddress, index: usize) {
        self.valued_cells.insert(address, index);
    }

    /// Returns the style index of a valued source cell: its recorded style, else the default.
    pub(crate) fn valued_cell_style(&self, address: CellAddress) -> usize {
        self.valued_cells.get(&address).copied().unwrap_or(0)
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

    /// Returns the style index and number format of every styled position on line `line` of
    /// `axis`, which lines inserted after it inherit. `valued` gives the number format of a
    /// valued cell, whose style alone is recorded here.
    pub(crate) fn line_styles(
        &self,
        axis: GridAxis,
        line: u32,
        valued: impl Fn(CellAddress) -> Option<NumberFormat>,
    ) -> Vec<(CellAddress, usize, NumberFormat)> {
        let on_line = |address: &&CellAddress| axis.of(**address) == line;
        let mut styles = self
            .valued_cells
            .iter()
            .filter(|(address, _)| on_line(address))
            .filter_map(|(address, index)| Some((*address, *index, valued(*address)?)))
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
        let move_cells = |cells: &BTreeMap<CellAddress, usize>| {
            cells
                .iter()
                .filter_map(|(address, index)| Some((edit.map_address(*address)?, *index)))
                .collect::<BTreeMap<_, _>>()
        };
        self.valued_cells = move_cells(&self.valued_cells);
        self.blank_cells = move_cells(&self.blank_cells);
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
