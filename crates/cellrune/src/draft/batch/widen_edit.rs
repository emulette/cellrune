use super::table_edit::{TableLocations, ranges_overlap, table_header_edit_error, table_location};
use crate::{
    Cell, CellAddress, CellContent, CellRange, CellValue, Column, DefinedName, DefinedNameScope,
    FormulaText, Sheet, Table, TableColumn, TableColumnName, TableId, TableType, ValidationError,
    case_insensitive_eq,
};

/// The hidden sheet-scoped name Excel keeps equal to the sheet-level auto-filter range.
const FILTER_DATABASE_NAME: &str = "_xlnm._FilterDatabase";

/// Widens one table by columns and reports whether its range changed.
///
/// Each new column is named by the text constant in its header cell and has no calculated
/// column formula. Its totals-row cell, when the table has one, must be empty.
pub(super) fn widen_table_columns(
    sheets: &mut [Sheet],
    locations: &TableLocations,
    table_id: TableId,
    range: CellRange,
) -> Result<bool, ValidationError> {
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let sheet = &sheets[sheet_index];
    let table = &sheet.tables()[table_index];
    if range == table.range() {
        return Ok(false);
    }
    validate_widening(table.range(), range)?;
    // New column names come from the one header row, and a query or XML table owns its columns
    // through parts this edit does not rewrite.
    if table.table_type() != TableType::Worksheet
        || table.header_row_count() != 1
        || table.totals_row_count() > 1
    {
        return Err(ValidationError::UnsupportedTableAuthoringMetadata {
            table_id: table_id.get(),
        });
    }
    if let Some(other) = sheet
        .tables()
        .iter()
        .find(|other| other.id() != table_id && ranges_overlap(range, other.range()))
    {
        return Err(ValidationError::OverlappingTables {
            sheet_id: sheet.id().get(),
            first_table_id: table_id.get().min(other.id().get()),
            second_table_id: table_id.get().max(other.id().get()),
        });
    }
    if sheet
        .auto_filter()
        .is_some_and(|filter| ranges_overlap(range, filter.range()))
    {
        return Err(ValidationError::SheetAutoFilterOverlapsTable {
            sheet_id: sheet.id().get(),
            table_id: table_id.get(),
        });
    }
    validate_added_columns(sheet, table.range(), range)?;
    let columns = widened_columns(sheet, table, range)?;
    sheets[sheet_index].tables_mut()[table_index].widen_columns(range, columns);
    Ok(true)
}

/// Widens the sheet-level auto-filter by columns and reports whether its range changed.
///
/// Filter columns keep their settings and are renumbered from the new first column.
pub(super) fn widen_sheet_auto_filter(
    sheet: &mut Sheet,
    range: CellRange,
) -> Result<bool, ValidationError> {
    let filter = sheet
        .auto_filter()
        .ok_or(ValidationError::SheetAutoFilterNotFound {
            sheet_id: sheet.id().get(),
        })?;
    if range == filter.range() {
        return Ok(false);
    }
    validate_widening(filter.range(), range)?;
    if let Some(table) = sheet
        .tables()
        .iter()
        .find(|table| ranges_overlap(range, table.range()))
    {
        return Err(ValidationError::SheetAutoFilterOverlapsTable {
            sheet_id: sheet.id().get(),
            table_id: table.id().get(),
        });
    }
    validate_added_columns(sheet, filter.range(), range)?;
    let widened = filter.widened(range);
    sheet.set_auto_filter(Some(widened));
    Ok(true)
}

/// Points the sheet's hidden `_xlnm._FilterDatabase` name at its auto-filter range and reports
/// whether the name changed.
///
/// Only the area after the sheet prefix the name already uses is replaced. A sheet without the
/// name gains none, and a name whose formula is not a sheet-qualified area is left as it is.
pub(super) fn align_filter_database_name(
    sheet: &Sheet,
    defined_names: &mut [DefinedName],
) -> Result<bool, ValidationError> {
    let Some(filter) = sheet.auto_filter() else {
        return Ok(false);
    };
    let Some(name) = defined_names.iter_mut().find(|name| {
        name.scope() == DefinedNameScope::Sheet(sheet.id())
            && case_insensitive_eq(name.name(), FILTER_DATABASE_NAME)
    }) else {
        return Ok(false);
    };
    let Some((prefix, area)) = name.formula().as_str().rsplit_once('!') else {
        return Ok(false);
    };
    let is_area = area.split(':').count() <= 2
        && area
            .split(':')
            .all(|corner| CellAddress::from_a1(&corner.replace('$', "")).is_ok());
    if !is_area {
        return Ok(false);
    }
    let formula = FormulaText::from_xlsx(format!(
        "{prefix}!{}:{}",
        absolute_a1(filter.range().start()),
        absolute_a1(filter.range().end())
    ))?;
    if name.formula() == &formula {
        return Ok(false);
    }
    *name = DefinedName::new(name.name().to_owned(), name.scope(), formula, name.hidden())?;
    Ok(true)
}

fn validate_widening(current: CellRange, range: CellRange) -> Result<(), ValidationError> {
    if range.start().row() != current.start().row()
        || range.end().row() != current.end().row()
        || range.start().column() > current.start().column()
        || range.end().column() < current.end().column()
    {
        return Err(ValidationError::ColumnWideningRangeInvalid);
    }
    Ok(())
}

/// Rejects new columns that overlap a merged range, a legacy array formula, or a declared spill
/// range. Content already inside the current range is the source's and stays accepted.
fn validate_added_columns(
    sheet: &Sheet,
    current: CellRange,
    range: CellRange,
) -> Result<(), ValidationError> {
    let mut added = Vec::with_capacity(2);
    if range.start().column() < current.start().column() {
        let last = Column::new(current.start().column().get() - 1)
            .expect("a column left of the current range exists");
        added.push(CellRange::from_ordered(
            range.start(),
            CellAddress::new(range.end().row(), last),
        ));
    }
    if range.end().column() > current.end().column() {
        let first = Column::new(current.end().column().get() + 1)
            .expect("a column right of the current range exists");
        added.push(CellRange::from_ordered(
            CellAddress::new(range.start().row(), first),
            range.end(),
        ));
    }
    for added in added {
        if let Some(blocking) = sheet
            .merged_ranges()
            .iter()
            .chain(sheet.array_formula_ranges().values())
            .find(|blocking| ranges_overlap(added, **blocking))
        {
            return Err(ValidationError::ColumnWideningOverlap {
                sheet_id: sheet.id().get(),
                row: blocking.start().row().get(),
                column: blocking.start().column().get(),
            });
        }
    }
    Ok(())
}

/// Returns every column of the widened table in column order, allocating new column IDs after
/// the largest existing one.
fn widened_columns(
    sheet: &Sheet,
    table: &Table,
    range: CellRange,
) -> Result<Vec<TableColumn>, ValidationError> {
    let current = table.range();
    let mut names = table
        .columns()
        .iter()
        .map(|column| column.name().to_owned())
        .collect::<Vec<_>>();
    let mut last_id = table
        .columns()
        .iter()
        .map(TableColumn::id)
        .max()
        .unwrap_or(0);
    let mut columns = Vec::with_capacity(names.len());
    for index in range.start().column().get()..=range.end().column().get() {
        let column = Column::new(index).expect("validated table range column");
        if column == current.start().column() {
            columns.extend(table.columns().iter().cloned());
        }
        if column >= current.start().column() && column <= current.end().column() {
            continue;
        }
        let header = CellAddress::new(range.start().row(), column);
        let content = sheet.cell(header).map(Cell::content);
        let name = match content {
            Some(CellContent::Literal(CellValue::Text(text))) => {
                TableColumnName::new(text.as_str())?
            }
            None | Some(CellContent::Literal(CellValue::Blank)) => {
                return Err(ValidationError::TableColumnNameEmpty);
            }
            Some(_) => return Err(table_header_edit_error(table.id(), header, false)),
        };
        if names
            .iter()
            .any(|existing| case_insensitive_eq(existing, name.as_str()))
        {
            return Err(ValidationError::DuplicateTableColumnName {
                name: name.as_str().to_owned(),
            });
        }
        if table.totals_row_count() == 1 {
            let totals = CellAddress::new(range.end().row(), column);
            if !matches!(
                sheet.cell(totals).map(Cell::content),
                None | Some(CellContent::Literal(CellValue::Blank))
            ) {
                return Err(ValidationError::TableTotalsCellOccupied {
                    table_id: table.id().get(),
                    row: totals.row().get(),
                    column: totals.column().get(),
                });
            }
        }
        last_id =
            last_id
                .checked_add(1)
                .ok_or(ValidationError::UnsupportedTableAuthoringMetadata {
                    table_id: table.id().get(),
                })?;
        columns.push(TableColumn::new(last_id, name.as_str(), None)?);
        names.push(name.as_str().to_owned());
    }
    Ok(columns)
}

fn absolute_a1(address: CellAddress) -> String {
    let text = address.to_string();
    let row_start = text
        .find(|character: char| character.is_ascii_digit())
        .expect("A1 text ends with a row number");
    format!("${}${}", &text[..row_start], &text[row_start..])
}

#[cfg(test)]
#[path = "widen_edit_tests.rs"]
mod tests;
