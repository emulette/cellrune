//! Structured references to a table rewritten as the sheet-qualified absolute references to the
//! cells they name, as Excel 16.114 saves them when it converts the table to a range:
//! `Things[val]` becomes `'S2'!$D$2:$D$4`, `Things[[#Totals],[val]]` becomes `'S2'!$D$5`, and a
//! this-row reference such as `[@val]` keeps its row relative, `'S2'!$D2`.

use super::ast::{StructuredColumns, StructuredItem, StructuredReference};
use crate::address::column_label;
use crate::{CellRange, case_insensitive_eq};

/// The geometry of the table being converted, with its sheet as formulas will name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableArea<'a> {
    pub(crate) display_name: &'a str,
    pub(crate) sheet_name: &'a str,
    pub(crate) range: CellRange,
    pub(crate) header_rows: u32,
    /// Totals rows a reference can reach: none while the totals row is hidden.
    pub(crate) totals_rows: u32,
    pub(crate) columns: &'a [&'a str],
}

/// The text a structured reference to a deleted or unreachable part of the table becomes.
const REFERENCE_ERROR: &str = "#REF!";

impl TableArea<'_> {
    /// Returns the A1 text that replaces `reference`, which names this table. `this_row` is the
    /// row of the formula when it is on the table's sheet, which a this-row reference keeps as a
    /// relative row; elsewhere a this-row reference becomes `#REF!`, as does a missing column or
    /// an empty part such as the totals of a table without one.
    pub(crate) fn a1_text(&self, reference: &StructuredReference, this_row: Option<u32>) -> String {
        self.area(reference, this_row)
            .unwrap_or_else(|| REFERENCE_ERROR.to_owned())
    }

    fn area(&self, reference: &StructuredReference, this_row: Option<u32>) -> Option<String> {
        let first_column = self.range.start().column().get();
        let column = |name: &str| {
            self.columns
                .iter()
                .position(|column| case_insensitive_eq(column, name))
                .and_then(|index| u32::try_from(index).ok())
                .map(|index| first_column + index)
        };
        let (left, right) = match &reference.columns {
            None => (first_column, self.range.end().column().get()),
            Some(StructuredColumns::Single(name)) => {
                let column = column(name)?;
                (column, column)
            }
            Some(StructuredColumns::Range { start, end }) => {
                let (start, end) = (column(start)?, column(end)?);
                (start.min(end), start.max(end))
            }
        };
        let top = self.range.start().row().get();
        let bottom = self.range.end().row().get();
        let data_top = top + self.header_rows;
        let data_bottom = bottom.checked_sub(self.totals_rows)?;
        let default_item = if reference.table.is_some() {
            StructuredItem::Data
        } else {
            StructuredItem::ThisRow
        };
        let items = if reference.items.is_empty() {
            std::slice::from_ref(&default_item)
        } else {
            reference.items.as_slice()
        };
        if items.contains(&StructuredItem::ThisRow) {
            let row = this_row?;
            return Some(self.text(left, right, None, Some(row)));
        }
        let mut rows = None::<(u32, u32)>;
        for item in items {
            let band = match item {
                StructuredItem::All => Some((top, bottom)),
                StructuredItem::Headers if self.header_rows == 0 => return None,
                StructuredItem::Headers => Some((top, data_top - 1)),
                StructuredItem::Data if data_top > data_bottom => None,
                StructuredItem::Data => Some((data_top, data_bottom)),
                StructuredItem::Totals if self.totals_rows == 0 => None,
                StructuredItem::Totals => Some((data_bottom + 1, bottom)),
                StructuredItem::ThisRow => None,
            };
            if let Some((start, end)) = band {
                rows = Some(rows.map_or((start, end), |(first, last)| {
                    (first.min(start), last.max(end))
                }));
            }
        }
        let (first_row, last_row) = rows?;
        Some(self.text(left, right, Some((first_row, last_row)), None))
    }

    fn text(&self, left: u32, right: u32, rows: Option<(u32, u32)>, row: Option<u32>) -> String {
        let cell = |column: u32, row: String| format!("${}{row}", column_label(column));
        let (start, end) = match (rows, row) {
            (Some((first, last)), _) => (
                cell(left, format!("${first}")),
                cell(right, format!("${last}")),
            ),
            (None, Some(row)) => (cell(left, row.to_string()), cell(right, row.to_string())),
            (None, None) => unreachable!("a converted area has rows"),
        };
        let sheet = self.sheet_name.replace('\'', "''");
        if start == end {
            format!("'{sheet}'!{start}")
        } else {
            format!("'{sheet}'!{start}:{end}")
        }
    }
}
