//! Bounded reads of cell formats and sheet layout.

use cellrune::{CellAddress, CellRange};

use super::WorkbookSession;
use super::query::normalized_limit;
use crate::convert::count_u64;
use crate::format_convert::cell_format_dto;
use crate::{
    CellFormatEntryDto, CellFormatPageDto, ColumnWidthDto, INTEROP_SCHEMA_VERSION, InteropError,
    RangeRequestDto, RowHeightDto,
};

impl WorkbookSession {
    /// Reads a bounded row-major page of the font, fill, border, and alignment each cell of a
    /// range displays with, with the explicit widths of its columns and heights of the page's
    /// rows.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet, invalid range, excessive
    /// page size, or out-of-range offset.
    pub fn read_formats(
        &self,
        request: &RangeRequestDto,
    ) -> Result<CellFormatPageDto, InteropError> {
        let draft = self.engine.draft();
        let sheet = draft
            .workbook()
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let start = CellAddress::from_a1(&request.start)?;
        let end = CellAddress::from_a1(&request.end)?;
        let range = CellRange::new(start, end)?;
        let width = u64::from(range.width());
        let total_cells = u64::from(range.height()) * width;
        if request.offset > total_cells {
            return Err(InteropError::page_offset());
        }
        let limit = normalized_limit(request.limit)?;
        let returned = (total_cells - request.offset).min(u64::from(limit));
        let mut cells = Vec::with_capacity(returned as usize);
        for index in request.offset..request.offset + returned {
            let address = CellAddress::from_indices(
                start.row().get() + (index / width) as u32,
                start.column().get() + (index % width) as u32,
            )?;
            cells.push(CellFormatEntryDto {
                address: address.to_string(),
                format: cell_format_dto(&draft.cell_format(sheet.id(), address)?),
            });
        }
        let presentation = draft.presentation();
        let column_widths = presentation
            .column_widths(sheet.id())
            .iter()
            .filter(|span| span.last() >= start.column() && span.first() <= end.column())
            .map(|span| ColumnWidthDto {
                first: span.first().get().max(start.column().get()),
                last: span.last().get().min(end.column().get()),
                width: span.width(),
            })
            .collect();
        let page_rows = (returned > 0).then(|| {
            let first = start.row().get() + (request.offset / width) as u32;
            let last = start.row().get() + ((request.offset + returned - 1) / width) as u32;
            first..=last
        });
        let row_heights = presentation
            .row_heights(sheet.id())
            .filter(|(row, _)| {
                page_rows
                    .as_ref()
                    .is_some_and(|rows| rows.contains(&row.get()))
            })
            .map(|(row, height)| RowHeightDto {
                row: row.get(),
                height,
            })
            .collect();
        let consumed = request.offset + count_u64(cells.len());
        Ok(CellFormatPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: sheet.name().as_str().to_owned(),
            start: start.to_string(),
            end: end.to_string(),
            total_cells,
            offset: request.offset,
            next_offset: (consumed < total_cells).then_some(consumed),
            cells,
            column_widths,
            row_heights,
        })
    }
}
