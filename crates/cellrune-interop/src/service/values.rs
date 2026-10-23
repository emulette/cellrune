//! Whole-rectangle value reads and writes.

use cellrune::{CalculationCellId, CellAddress, CellContent, CellRange, SavedResult, Sheet};

use super::WorkbookSession;
use crate::convert::{result_to_dto, value_to_dto};
use crate::error::DETAIL_RANGE_WRITE_NOT_RECTANGULAR;
use crate::{
    CalculationResultDto, CellValueDto, EditBatchDto, EditReceiptDto, INTEROP_SCHEMA_VERSION,
    InteropError, RangeValueDto, RangeValuesDto, RangeValuesRequestDto, RangeWriteRequestDto,
    UnavailableValueDto, WorkbookChangeDto,
};

/// Hard maximum number of cells one rectangle value read returns: one full worksheet column.
pub const MAX_RANGE_VALUE_CELLS: u64 = 1_048_576;

impl WorkbookSession {
    /// Reads every value of an inclusive rectangle as rows.
    ///
    /// A cell holds its current calculated result when the session has calculated the current
    /// revision, including dynamic spill followers, and otherwise its literal value or a formula's
    /// saved value, blank when none was saved.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet or invalid range, or
    /// `interop.page.limit_exceeded` for a rectangle of more than [`MAX_RANGE_VALUE_CELLS`] cells.
    pub fn read_values(
        &self,
        request: &RangeValuesRequestDto,
    ) -> Result<RangeValuesDto, InteropError> {
        let workbook = self.engine.workbook();
        let sheet = workbook
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let start = CellAddress::from_a1(&request.start)?;
        let end = CellAddress::from_a1(&request.end)?;
        let range = CellRange::new(start, end)?;
        if u64::from(range.height()) * u64::from(range.width()) > MAX_RANGE_VALUE_CELLS {
            return Err(InteropError::page_limit());
        }
        let calculation = self.current_calculation();
        let mut rows = Vec::with_capacity(range.height() as usize);
        for row in range.start().row().get()..=range.end().row().get() {
            let mut values = Vec::with_capacity(range.width() as usize);
            for column in range.start().column().get()..=range.end().column().get() {
                let address = CellAddress::from_indices(row, column)?;
                let calculated = calculation.and_then(|calculation| {
                    calculation
                        .materialized_cell(CalculationCellId::new(sheet.id(), address))
                        .map(|cell| result_to_dto(cell.result()))
                });
                values.push(match calculated {
                    Some(CalculationResultDto::Value { value }) => RangeValueDto::Value(value),
                    Some(CalculationResultDto::Unavailable {
                        code,
                        message,
                        detail,
                        limit,
                    }) => RangeValueDto::Unavailable(UnavailableValueDto::Unavailable {
                        code,
                        message,
                        detail,
                        limit,
                    }),
                    None => RangeValueDto::Value(source_value(sheet, address)),
                });
            }
            rows.push(values);
        }
        Ok(RangeValuesDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: sheet.name().as_str().to_owned(),
            start: start.to_string(),
            end: end.to_string(),
            rows,
        })
    }

    /// Writes a rectangle of values from its top-left cell as one atomic edit batch at the current
    /// revision; a blank value clears its cell.
    ///
    /// # Errors
    ///
    /// Returns `interop.change.invalid` when the rows are empty or of different lengths, or the
    /// same revision, input, resource, and validation errors as [`Self::apply_changes`]. A failure
    /// leaves the workbook unchanged.
    pub fn write_values(
        &mut self,
        request: RangeWriteRequestDto,
    ) -> Result<EditReceiptDto, InteropError> {
        let start = CellAddress::from_a1(&request.start)?;
        let width = request.rows.first().map_or(0, Vec::len);
        if width == 0 || request.rows.iter().any(|row| row.len() != width) {
            return Err(InteropError::invalid_change(
                DETAIL_RANGE_WRITE_NOT_RECTANGULAR.to_owned(),
            ));
        }
        let mut changes = Vec::with_capacity(request.rows.len() * width);
        for (row_offset, row) in request.rows.into_iter().enumerate() {
            for (column_offset, value) in row.into_iter().enumerate() {
                let address = CellAddress::from_indices(
                    offset_index(start.row().get(), row_offset),
                    offset_index(start.column().get(), column_offset),
                )?;
                changes.push(WorkbookChangeDto::SetValue {
                    sheet: request.sheet.clone(),
                    address: address.to_string(),
                    value,
                });
            }
        }
        let revision = self.engine.workbook().semantic_revision();
        self.apply_changes(revision, EditBatchDto { changes })
    }
}

/// Adds a zero-based offset to a one-based index, saturating so the address check rejects it.
fn offset_index(start: u32, offset: usize) -> u32 {
    u32::try_from(offset)
        .ok()
        .and_then(|offset| start.checked_add(offset))
        .unwrap_or(u32::MAX)
}

/// Returns a cell's literal value or a formula's saved value, blank when none was saved.
fn source_value(sheet: &Sheet, address: CellAddress) -> CellValueDto {
    match sheet.cell(address).map(|cell| cell.content()) {
        Some(CellContent::Literal(value)) => value_to_dto(value),
        Some(CellContent::Formula(formula)) => match formula.saved_result() {
            SavedResult::Present(value) => value_to_dto(value),
            _ => CellValueDto::Blank,
        },
        None => CellValueDto::Blank,
    }
}
