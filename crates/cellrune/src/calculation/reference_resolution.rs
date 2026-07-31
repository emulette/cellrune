use super::ast::{RefBody, Reference, StructuredColumns, StructuredItem, StructuredReference};
use super::limits::CalculationLimitKind;
use super::runtime::{CellId, Rect, RectSpan, ReferenceValue, SheetSpan};
use super::value::ErrorKind;
use super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
use crate::WorkbookSnapshot;

pub(in crate::calculation) fn resolve_reference_span(
    workbook: &WorkbookSnapshot,
    current_sheet: usize,
    reference: &Reference,
) -> Result<RectSpan, ErrorKind> {
    let (start_sheet, end_sheet) = match &reference.sheet {
        Some(prefix) if prefix.external_workbook_detail().is_some() => {
            return Err(ErrorKind::Unsupported);
        }
        Some(prefix) => {
            let start = workbook
                .sheet_index_by_name(&prefix.name)
                .ok_or(ErrorKind::Ref)?;
            let end = match &prefix.end_name {
                Some(name) => workbook.sheet_index_by_name(name).ok_or(ErrorKind::Ref)?,
                None => start,
            };
            (start, end)
        }
        None => (current_sheet, current_sheet),
    };
    let (row_start, col_start, row_end, col_end, whole_rows) = reference_bounds(&reference.body);
    let sheets = reference.sheet.as_ref().map_or_else(
        || SheetSpan::single(start_sheet),
        |prefix| {
            if prefix.end_name.is_some() {
                SheetSpan::new(start_sheet, end_sheet)
            } else {
                SheetSpan::single(start_sheet)
            }
        },
    );
    Ok(RectSpan::new(
        sheets,
        Rect {
            sheet: start_sheet,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows,
        },
    ))
}

pub(in crate::calculation) fn structured_table_coordinates(
    workbook: &WorkbookSnapshot,
    context: CellId,
    reference: &StructuredReference,
) -> Result<(usize, usize), ErrorKind> {
    let address =
        crate::CellAddress::from_indices(context.1, context.2).map_err(|_| ErrorKind::Ref)?;
    let location = match &reference.table {
        Some(name) => workbook.table_location(name).ok_or(ErrorKind::Name)?,
        None => {
            let sheet = workbook.sheets().get(context.0).ok_or(ErrorKind::Ref)?;
            workbook
                .containing_table_location(sheet.id(), address)
                .ok_or(ErrorKind::Value)?
        }
    };
    Ok((location.sheet_index, location.table_index))
}

pub(in crate::calculation) fn validate_explicit_structured_reference_target(
    workbook: &WorkbookSnapshot,
    reference: &StructuredReference,
) -> Result<(), ErrorKind> {
    let table_name = reference.table.as_deref().ok_or(ErrorKind::Value)?;
    let location = workbook.table_location(table_name).ok_or(ErrorKind::Name)?;
    let table = &workbook.sheets()[location.sheet_index].tables()[location.table_index];
    let validate_column = |name: &str| {
        workbook
            .table_column_location(table.id(), name)
            .map(|_| ())
            .ok_or(ErrorKind::Ref)
    };
    match &reference.columns {
        None => {}
        Some(StructuredColumns::Single(name)) => validate_column(name)?,
        Some(StructuredColumns::Range { start, end }) => {
            validate_column(start)?;
            validate_column(end)?;
        }
    }
    if reference.items.contains(&StructuredItem::Headers) && table.header_row_count() == 0 {
        return Err(ErrorKind::Ref);
    }
    Ok(())
}

pub(in crate::calculation) fn resolve_structured_reference(
    workbook: &WorkbookSnapshot,
    context: CellId,
    reference: &StructuredReference,
) -> Result<ReferenceValue, ErrorKind> {
    let (sheet_index, table_index) = structured_table_coordinates(workbook, context, reference)?;
    let table = &workbook.sheets()[sheet_index].tables()[table_index];
    let range = table.range();
    let table_row_start = range.start().row().get();
    let table_row_end = range.end().row().get();
    let table_col_start = range.start().column().get();
    let table_col_end = range.end().column().get();
    let header_end = table_row_start
        .checked_add(table.header_row_count())
        .and_then(|row| row.checked_sub(1))
        .ok_or(ErrorKind::Ref)?;
    let data_row_start = table_row_start
        .checked_add(table.header_row_count())
        .ok_or(ErrorKind::Ref)?;
    let data_row_end = table_row_end
        .checked_sub(table.totals_row_count())
        .ok_or(ErrorKind::Ref)?;
    let has_headers = table.header_row_count() > 0;
    let has_totals = table.totals_row_count() > 0 && table.totals_row_shown();
    let column_index = |name: &str| {
        workbook
            .table_column_location(table.id(), name)
            .map(|column| column.column_index)
            .ok_or(ErrorKind::Ref)
    };
    let (col_start, col_end) = match &reference.columns {
        None => (table_col_start, table_col_end),
        Some(StructuredColumns::Single(name)) => {
            let index = column_index(name)?;
            let column = table_col_start
                .checked_add(u32::try_from(index).map_err(|_| ErrorKind::Ref)?)
                .ok_or(ErrorKind::Ref)?;
            (column, column)
        }
        Some(StructuredColumns::Range { start, end }) => {
            let start = column_index(start)?;
            let end = column_index(end)?;
            let start = table_col_start
                .checked_add(u32::try_from(start).map_err(|_| ErrorKind::Ref)?)
                .ok_or(ErrorKind::Ref)?;
            let end = table_col_start
                .checked_add(u32::try_from(end).map_err(|_| ErrorKind::Ref)?)
                .ok_or(ErrorKind::Ref)?;
            (start.min(end), start.max(end))
        }
    };

    let default_item = if reference.table.is_some() {
        StructuredItem::Data
    } else {
        StructuredItem::ThisRow
    };
    let mut row_start = None::<u32>;
    let mut row_end = None::<u32>;
    let items = if reference.items.is_empty() {
        std::slice::from_ref(&default_item)
    } else {
        reference.items.as_slice()
    };
    for item in items {
        let band = match item {
            StructuredItem::All => Some((table_row_start, table_row_end)),
            StructuredItem::Headers if !has_headers => return Err(ErrorKind::Ref),
            StructuredItem::Headers => Some((table_row_start, header_end)),
            StructuredItem::Data if data_row_start > data_row_end => None,
            StructuredItem::Data => Some((data_row_start, data_row_end)),
            StructuredItem::Totals if !has_totals => None,
            StructuredItem::Totals => {
                Some((table_row_end - table.totals_row_count() + 1, table_row_end))
            }
            StructuredItem::ThisRow
                if context.0 != sheet_index
                    || context.1 < data_row_start
                    || context.1 > data_row_end =>
            {
                return Err(ErrorKind::Value);
            }
            StructuredItem::ThisRow => Some((context.1, context.1)),
        };
        if let Some((start, end)) = band {
            row_start = Some(row_start.map_or(start, |current| current.min(start)));
            row_end = Some(row_end.map_or(end, |current| current.max(end)));
        }
    }
    let (Some(row_start), Some(row_end)) = (row_start, row_end) else {
        return Ok(ReferenceValue::Empty);
    };
    Ok(ReferenceValue::from_rect(Rect {
        sheet: sheet_index,
        row_start,
        col_start,
        row_end,
        col_end,
        whole_rows: false,
    }))
}

pub(in crate::calculation) fn union_reference_values(
    left: &ReferenceValue,
    right: &ReferenceValue,
) -> Result<ReferenceValue, ErrorKind> {
    if matches!(left, ReferenceValue::Empty) || matches!(right, ReferenceValue::Empty) {
        return Err(ErrorKind::Ref);
    }
    let mut areas = Vec::with_capacity(left.area_count().saturating_add(right.area_count()));
    areas.extend_from_slice(left.areas());
    areas.extend_from_slice(right.areas());
    Ok(ReferenceValue::from_areas(areas))
}

pub(in crate::calculation) fn intersection_reference_work(
    left: &ReferenceValue,
    right: &ReferenceValue,
) -> Result<u64, ErrorKind> {
    if matches!(left, ReferenceValue::Empty) || matches!(right, ReferenceValue::Empty) {
        return Err(ErrorKind::Ref);
    }
    if left.has_sheet_span() || right.has_sheet_span() {
        return Err(ErrorKind::Value);
    }
    let left_sheet = left
        .areas()
        .first()
        .and_then(|area| area.rects().next())
        .map(|rect| rect.sheet);
    let right_sheet = right
        .areas()
        .first()
        .and_then(|area| area.rects().next())
        .map(|rect| rect.sheet);
    if left_sheet != right_sheet {
        return Err(ErrorKind::Value);
    }
    u64::try_from(left.area_count())
        .ok()
        .and_then(|left| {
            u64::try_from(right.area_count())
                .ok()
                .and_then(|right| left.checked_mul(right))
        })
        .ok_or(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ))
}

pub(in crate::calculation) fn intersect_reference_values(
    left: &ReferenceValue,
    right: &ReferenceValue,
    max_areas: u64,
    mut check_cancelled: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<ReferenceValue, ErrorKind> {
    let mut areas = Vec::new();
    for left in left.areas() {
        for right in right.areas() {
            check_cancelled()?;
            if let Some(area) = left.intersection(right) {
                areas.push(area);
                if u64::try_from(areas.len()).map_or(true, |count| count > max_areas) {
                    return Err(ErrorKind::ResourceLimit(
                        CalculationLimitKind::ReferenceAreas,
                    ));
                }
            }
        }
    }
    if areas.is_empty() {
        Err(ErrorKind::Null)
    } else {
        Ok(ReferenceValue::from_areas(areas))
    }
}

pub(in crate::calculation) fn range_reference_rect(
    start: &ReferenceValue,
    end: &ReferenceValue,
) -> Result<Rect, ErrorKind> {
    let start = start.bounding_rect()?;
    let end = end.bounding_rect()?;
    if start.sheet != end.sheet {
        return Err(ErrorKind::Value);
    }
    let row_start = start.row_start.min(end.row_start);
    let row_end = start.row_end.max(end.row_end);
    Ok(Rect {
        sheet: start.sheet,
        row_start,
        col_start: start.col_start.min(end.col_start),
        row_end,
        col_end: start.col_end.max(end.col_end),
        whole_rows: row_start == 1
            && row_end == EXCEL_MAX_ROWS
            && (start.whole_rows || end.whole_rows),
    })
}

fn reference_bounds(body: &RefBody) -> (u32, u32, u32, u32, bool) {
    match body {
        RefBody::Cell(cell) => (cell.row, cell.column, cell.row, cell.column, false),
        RefBody::Area(start, end) => (
            start.row.min(end.row),
            start.column.min(end.column),
            start.row.max(end.row),
            start.column.max(end.column),
            false,
        ),
        RefBody::Columns(start, end) => (
            1,
            start.column.min(end.column),
            EXCEL_MAX_ROWS,
            start.column.max(end.column),
            true,
        ),
        RefBody::Rows(start, end) => (
            start.row.min(end.row),
            1,
            start.row.max(end.row),
            EXCEL_MAX_COLUMNS,
            false,
        ),
    }
}
