//! Range pages in their optional forms: without empty cells, values only, compact, or within a
//! byte limit.

use cellrune::{CellAddress, CellRange};

use super::WorkbookSession;
use super::query::normalized_limit;
use super::values::range_value;
use crate::convert::cell_dto;
use crate::{
    CellDto, CellValueDto, INTEROP_SCHEMA_VERSION, InteropError, RangeCellValueDto, RangePageDto,
    RangeReadOptionsDto, RangeRequestDto, RangeValueDto,
};

/// One position of the page window with what each form needs of it.
struct Position {
    address: CellAddress,
    empty: bool,
    cell: Option<CellDto>,
    value: Option<RangeValueDto>,
}

impl WorkbookSession {
    /// Reads a bounded row-major page in the form `options` selects. Default options return
    /// exactly what [`Self::read_range`] returns.
    ///
    /// Offset and limit count positions of the range whatever the form, so an omitted empty cell
    /// still advances `next_offset`. With `max_bytes`, the page is the longest prefix of its window
    /// whose serialized JSON fits.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Self::read_range`], or `interop.page.byte_limit_exceeded` when the
    /// byte limit leaves no room for the page's first position.
    pub fn read_range_with_options(
        &self,
        request: &RangeRequestDto,
        options: &RangeReadOptionsDto,
    ) -> Result<RangePageDto, InteropError> {
        if *options == RangeReadOptionsDto::default() {
            return self.read_range(request);
        }
        let workbook = self.engine.workbook();
        let sheet = workbook
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
        let window = (total_cells - request.offset).min(u64::from(limit));
        let calculation = self.current_calculation();
        let values = options.values_only || options.compact;
        let mut positions = Vec::with_capacity(window as usize);
        for index in request.offset..request.offset + window {
            let address = CellAddress::from_indices(
                start.row().get() + (index / width) as u32,
                start.column().get() + (index % width) as u32,
            )?;
            let value = range_value(sheet, calculation, address);
            let empty = value.is_none();
            positions.push(Position {
                address,
                empty,
                cell: (!(values || (options.omit_empty && empty)))
                    .then(|| cell_dto(workbook, calculation, sheet.id(), address)),
                value: values.then(|| value.unwrap_or(RangeValueDto::Value(CellValueDto::Blank))),
            });
        }
        let page = |count: usize| {
            let consumed = request.offset + count as u64;
            let window = &positions[..count];
            let mut page = RangePageDto {
                schema_version: INTEROP_SCHEMA_VERSION,
                sheet: sheet.name().as_str().to_owned(),
                start: start.to_string(),
                end: end.to_string(),
                total_cells,
                offset: request.offset,
                next_offset: (consumed < total_cells).then_some(consumed),
                cells: Vec::new(),
                values: None,
                rows: None,
            };
            if options.compact {
                page.rows = Some(compact_rows(window, options.omit_empty));
            } else if options.values_only {
                page.values = Some(
                    window
                        .iter()
                        .filter(|position| !(options.omit_empty && position.empty))
                        .filter_map(|position| {
                            position.value.clone().map(|value| RangeCellValueDto {
                                address: position.address.to_string(),
                                value,
                            })
                        })
                        .collect(),
                );
            } else {
                page.cells = window
                    .iter()
                    .filter_map(|position| position.cell.clone())
                    .collect();
            }
            page
        };
        let Some(max_bytes) = options.max_bytes else {
            return Ok(page(positions.len()));
        };
        let fits = |count: usize| -> Result<bool, InteropError> {
            let bytes =
                serde_json::to_vec(&page(count)).map_err(|_| InteropError::serialization())?;
            Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= max_bytes)
        };
        if fits(positions.len())? {
            return Ok(page(positions.len()));
        }
        // Short of the whole window `next_offset` is always a number, so the size grows with the
        // prefix and the longest fitting prefix can be found by bisection.
        if !fits(0)? {
            return Err(InteropError::page_byte_limit());
        }
        let (mut low, mut high) = (0, positions.len() - 1);
        while low < high {
            let middle = (low + high).div_ceil(2);
            if fits(middle)? {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        if low == 0 {
            return Err(InteropError::page_byte_limit());
        }
        Ok(page(low))
    }
}

/// Groups positions by worksheet row; with `omit_empty`, each row drops its trailing empty cells.
fn compact_rows(positions: &[Position], omit_empty: bool) -> Vec<Vec<RangeValueDto>> {
    let mut rows: Vec<(u32, Vec<&Position>)> = Vec::new();
    for position in positions {
        let row = position.address.row().get();
        match rows.last_mut() {
            Some((current, members)) if *current == row => members.push(position),
            _ => rows.push((row, vec![position])),
        }
    }
    rows.into_iter()
        .map(|(_, mut members)| {
            if omit_empty {
                while members.last().is_some_and(|position| position.empty) {
                    members.pop();
                }
            }
            members
                .into_iter()
                .filter_map(|position| position.value.clone())
                .collect()
        })
        .collect()
}
