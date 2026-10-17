//! Formula cells of each sheet in column-major order, so a range dependency reads only the formulas
//! in its columns instead of every formula in its rows.

use super::super::runtime::Rect;
use super::Engine;
use crate::WorkbookSnapshot;

/// The formula cells of one sheet as ascending `(column, row)` pairs.
#[derive(Debug)]
pub(super) struct FormulaColumns {
    cells: Vec<(u32, u32)>,
}

impl FormulaColumns {
    fn for_workbook(workbook: &WorkbookSnapshot) -> Vec<Self> {
        workbook
            .sheets()
            .iter()
            .map(|sheet| {
                Self::from_cells(
                    sheet
                        .formula_addresses()
                        .iter()
                        .map(|address| (address.column().get(), address.row().get())),
                )
            })
            .collect()
    }

    fn from_cells(cells: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let mut cells = cells.into_iter().collect::<Vec<_>>();
        cells.sort_unstable();
        Self { cells }
    }

    fn within(&self, rect: &Rect) -> FormulasWithin<'_> {
        let mut within = FormulasWithin {
            cells: &self.cells,
            position: 0,
            row_start: rect.row_start,
            row_end: rect.row_end,
            col_end: rect.col_end,
        };
        within.seek(rect.col_start);
        within
    }
}

/// The formula cells inside one rectangle, as `(row, column)` pairs in column-major order.
#[derive(Debug)]
pub(super) struct FormulasWithin<'cells> {
    cells: &'cells [(u32, u32)],
    position: usize,
    row_start: u32,
    row_end: u32,
    col_end: u32,
}

impl FormulasWithin<'_> {
    /// Moves to the first cell at or after the first row of the rectangle in `column`.
    fn seek(&mut self, column: u32) {
        let target = (column, self.row_start);
        self.position += self.cells[self.position..].partition_point(|cell| *cell < target);
    }
}

impl Iterator for FormulasWithin<'_> {
    type Item = (u32, u32);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(column, row) = self.cells.get(self.position)?;
            if column > self.col_end {
                return None;
            }
            if row < self.row_start {
                self.seek(column);
            } else if row > self.row_end {
                self.seek(column + 1);
            } else {
                self.position += 1;
                return Some((row, column));
            }
        }
    }
}

impl Engine<'_> {
    /// Returns the formula cells inside `rect`, indexing the workbook's formulas by column on the
    /// first call.
    pub(super) fn formula_cells_within(&self, rect: &Rect) -> FormulasWithin<'_> {
        self.formula_columns
            .get_or_init(|| FormulaColumns::for_workbook(self.workbook))[rect.sheet]
            .within(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::FormulaColumns;
    use crate::calculation::runtime::Rect;

    fn rect(row_start: u32, col_start: u32, row_end: u32, col_end: u32) -> Rect {
        Rect {
            sheet: 0,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows: false,
        }
    }

    #[test]
    fn cells_within_a_rectangle_match_a_filter_of_every_formula() {
        let cells = (1..=40)
            .flat_map(|row| [(1, row), (3, row), (4, row * 3), (9, row * 7)])
            .filter(|(column, row)| (row + column) % 5 != 0)
            .collect::<Vec<_>>();
        let columns = FormulaColumns::from_cells(cells.iter().copied());
        for rect in [
            rect(1, 1, 40, 1),
            rect(5, 2, 12, 4),
            rect(1, 1, 1_048_576, 16_384),
            rect(10, 2, 30, 2),
            rect(41, 1, 50, 3),
            rect(100, 4, 120, 9),
            rect(7, 9, 7, 9),
        ] {
            let mut expected = cells
                .iter()
                .filter(|(column, row)| {
                    (rect.row_start..=rect.row_end).contains(row)
                        && (rect.col_start..=rect.col_end).contains(column)
                })
                .map(|(column, row)| (*row, *column))
                .collect::<Vec<_>>();
            expected.sort_unstable_by_key(|(row, column)| (*column, *row));
            assert_eq!(
                columns.within(&rect).collect::<Vec<_>>(),
                expected,
                "{rect:?}"
            );
        }
    }
}
