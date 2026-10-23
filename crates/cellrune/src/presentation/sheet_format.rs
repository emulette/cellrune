//! Column widths, row heights, and edited cell formats kept with presentation state.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::format::{validate_column_width, validate_row_height};
use super::{Alignment, Border, CellFormat, DocumentPresentation, Fill, Font};
use crate::{CellAddress, Column, Row, SheetId, ValidationError};

/// One span of columns that share an explicit width, in characters of the default font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidth {
    first: Column,
    last: Column,
    width: f64,
}

// Widths are validated finite, so equality is reflexive.
impl Eq for ColumnWidth {}

impl ColumnWidth {
    /// Returns the first column of the span.
    pub const fn first(&self) -> Column {
        self.first
    }

    /// Returns the last column of the span.
    pub const fn last(&self) -> Column {
        self.last
    }

    /// Returns the width in characters.
    pub const fn width(&self) -> f64 {
        self.width
    }
}

/// The layout and edited formats of one sheet.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct SheetFormats {
    /// Sorted, non-overlapping spans.
    pub(crate) column_widths: Vec<ColumnWidth>,
    pub(crate) row_heights: BTreeMap<Row, f64>,
    /// Formats set by draft edits, which replace the source style of these cells.
    pub(crate) cell_formats: BTreeMap<CellAddress, CellFormat>,
}

// Widths and heights are validated finite, so equality is reflexive.
impl Eq for SheetFormats {}

impl SheetFormats {
    fn is_empty(&self) -> bool {
        self.column_widths.is_empty() && self.row_heights.is_empty() && self.cell_formats.is_empty()
    }

    /// Gives `first..=last` the width `width`, or no width, splitting the spans it overlaps.
    fn set_column_width(&mut self, first: Column, last: Column, width: Option<f64>) {
        let mut spans = Vec::with_capacity(self.column_widths.len() + 2);
        for span in &self.column_widths {
            if span.last < first || span.first > last {
                spans.push(*span);
                continue;
            }
            if span.first < first {
                spans.push(ColumnWidth {
                    last: column_before(first),
                    ..*span
                });
            }
            if span.last > last {
                spans.push(ColumnWidth {
                    first: column_after(last),
                    ..*span
                });
            }
        }
        if let Some(width) = width {
            spans.push(ColumnWidth { first, last, width });
        }
        spans.sort_by_key(|span| span.first);
        self.column_widths = spans;
    }
}

fn column_before(column: Column) -> Column {
    Column::new(column.get() - 1).expect("a span starting after column 1 has a previous column")
}

fn column_after(column: Column) -> Column {
    Column::new(column.get() + 1).expect("a span ending before the last column has a next column")
}

/// The format a new workbook's default style displays: Calibri 11.
pub(crate) fn canonical_default_format() -> CellFormat {
    let font = Font::new()
        .with_name("Calibri")
        .and_then(|font| font.with_size(11.0))
        .expect("Calibri 11 is a valid font");
    CellFormat::from_parts(
        font,
        Fill::default(),
        Border::default(),
        Alignment::default(),
    )
}

impl DocumentPresentation {
    /// Returns the sheet's explicit column widths, sorted and non-overlapping.
    pub fn column_widths(&self, sheet_id: SheetId) -> &[ColumnWidth] {
        self.formats
            .get(&sheet_id)
            .map_or(&[], |formats| formats.column_widths.as_slice())
    }

    /// Returns the explicit width of one column, in characters of the default font.
    pub fn column_width(&self, sheet_id: SheetId, column: Column) -> Option<f64> {
        self.column_widths(sheet_id)
            .iter()
            .find(|span| span.first <= column && column <= span.last)
            .map(|span| span.width)
    }

    /// Returns the explicit height of one row, in points.
    pub fn row_height(&self, sheet_id: SheetId, row: Row) -> Option<f64> {
        self.formats
            .get(&sheet_id)
            .and_then(|formats| formats.row_heights.get(&row))
            .copied()
    }

    /// Returns the sheet's explicit row heights in row order.
    pub fn row_heights(&self, sheet_id: SheetId) -> impl Iterator<Item = (Row, f64)> + '_ {
        self.formats
            .get(&sheet_id)
            .into_iter()
            .flat_map(|formats| formats.row_heights.iter())
            .map(|(row, height)| (*row, *height))
    }

    /// Returns the format of a `cellXfs` entry, or the default style's format.
    pub(crate) fn style_format(&self, index: usize) -> CellFormat {
        self.style_formats
            .get(index)
            .or_else(|| self.style_formats.first())
            .cloned()
            .unwrap_or_else(canonical_default_format)
    }

    pub(crate) fn edited_cell_format(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<&CellFormat> {
        self.formats
            .get(&sheet_id)
            .and_then(|formats| formats.cell_formats.get(&address))
    }

    pub(crate) fn edited_cell_formats(
        &self,
        sheet_id: SheetId,
    ) -> impl Iterator<Item = (CellAddress, &CellFormat)> {
        self.formats
            .get(&sheet_id)
            .into_iter()
            .flat_map(|formats| formats.cell_formats.iter())
            .map(|(address, format)| (*address, format))
    }

    pub(crate) fn set_style_formats(&mut self, formats: Vec<CellFormat>) {
        self.style_formats = Arc::new(formats);
    }

    pub(crate) fn source_column_width(
        &mut self,
        sheet_id: SheetId,
        first: Column,
        last: Column,
        width: f64,
    ) {
        Arc::make_mut(self.formats.entry(sheet_id).or_default()).set_column_width(
            first,
            last,
            Some(width),
        );
    }

    pub(crate) fn source_row_height(&mut self, sheet_id: SheetId, row: Row, height: f64) {
        Arc::make_mut(self.formats.entry(sheet_id).or_default())
            .row_heights
            .insert(row, height);
    }

    /// Sets the width of `first..=last`, or clears it, and reports whether anything changed.
    pub(crate) fn set_column_width(
        &mut self,
        sheet_id: SheetId,
        first: Column,
        last: Column,
        width: Option<f64>,
    ) -> Result<bool, ValidationError> {
        if first > last {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        let width = width.map(validate_column_width).transpose()?;
        let mut spans = SheetFormats {
            column_widths: self.column_widths(sheet_id).to_vec(),
            ..SheetFormats::default()
        };
        spans.set_column_width(first, last, width);
        if spans.column_widths == self.column_widths(sheet_id) {
            return Ok(false);
        }
        self.advance_revision()?;
        self.edit_sheet_formats(sheet_id, |formats| {
            formats.column_widths = spans.column_widths;
        });
        Ok(true)
    }

    /// Sets the height of `row`, or clears it, and reports whether anything changed.
    pub(crate) fn set_row_height(
        &mut self,
        sheet_id: SheetId,
        row: Row,
        height: Option<f64>,
    ) -> Result<bool, ValidationError> {
        let height = height.map(validate_row_height).transpose()?;
        if self.row_height(sheet_id, row) == height {
            return Ok(false);
        }
        self.advance_revision()?;
        self.edit_sheet_formats(sheet_id, |formats| {
            match height {
                Some(height) => formats.row_heights.insert(row, height),
                None => formats.row_heights.remove(&row),
            };
        });
        Ok(true)
    }

    /// Records the format a cell displays with after an edit, and reports whether it changed.
    pub(crate) fn set_cell_format(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        format: CellFormat,
    ) -> Result<bool, ValidationError> {
        if self.edited_cell_format(sheet_id, address) == Some(&format) {
            return Ok(false);
        }
        self.advance_revision()?;
        self.edit_sheet_formats(sheet_id, |formats| {
            formats.cell_formats.insert(address, format);
        });
        Ok(true)
    }

    fn edit_sheet_formats(&mut self, sheet_id: SheetId, edit: impl FnOnce(&mut SheetFormats)) {
        let formats = Arc::make_mut(self.formats.entry(sheet_id).or_default());
        edit(formats);
        if formats.is_empty() {
            self.formats.remove(&sheet_id);
        }
    }

    /// Returns whether column widths and row heights match `other` on every sheet.
    pub(crate) fn layout_matches(&self, other: &Self) -> bool {
        let sheets = self.formats.keys().chain(other.formats.keys());
        sheets.into_iter().all(|sheet_id| {
            self.column_widths(*sheet_id) == other.column_widths(*sheet_id)
                && self.row_heights(*sheet_id).eq(other.row_heights(*sheet_id))
        })
    }
}
