use super::BatchExecutionError;
use super::staged::sheet_by_id_mut;
use super::table_edit::{
    TableEditState, ranges_overlap, table_header_edit_error, upsert_literal_text,
};
use crate::{
    Cell, CellAddress, CellContent, CellRange, CellValue, Column, DefinedName, Sheet, SheetId,
    Table, TableAutoFilter, TableColumn, TableColumnName, TableId, TableName, TableStyleInfo,
    TableType, ValidationError, case_insensitive_eq,
};

/// The style Excel gives a new table.
const DEFAULT_TABLE_STYLE: &str = "TableStyleMedium2";

/// Creates a table over `range` on `sheet_id` whose first row is its header, as Excel's Format as
/// Table does: each text header cell names its column, and a blank one is named `Column` and its
/// position, which the cell then holds. The table gets the next free ID, a filter over its range,
/// Excel's default style, and `name`, or by default `Table` and the number after the workbook's
/// table count, the next free one when that is taken.
pub(super) fn create_table(
    sheets: &mut [Sheet],
    defined_names: &[DefinedName],
    mut state: TableEditState<'_>,
    sheet_id: SheetId,
    range: CellRange,
    name: Option<&TableName>,
) -> Result<TableId, BatchExecutionError> {
    let table_count = sheets
        .iter()
        .map(|sheet| sheet.tables().len())
        .sum::<usize>();
    // A source table converted to a range keeps its part until the save, so its ID stays taken.
    let next_id = sheets
        .iter()
        .chain(state.source.into_iter().flat_map(|source| source.sheets()))
        .flat_map(Sheet::tables)
        .map(|table| table.id().get())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(ValidationError::UnsupportedTableAuthoringMetadata { table_id: u32::MAX })?;
    let table_id = TableId::new(next_id)?;
    let taken = |candidate: &str| {
        sheets
            .iter()
            .flat_map(Sheet::tables)
            .any(|table| case_insensitive_eq(table.display_name().as_str(), candidate))
            || defined_names
                .iter()
                .any(|defined| case_insensitive_eq(defined.name(), candidate))
    };
    let name = match name {
        Some(name) => {
            name.validate_xlsx()?;
            if taken(name.as_str()) {
                return Err(ValidationError::DuplicateTableDisplayName {
                    name: name.as_str().to_owned(),
                }
                .into());
            }
            name.clone()
        }
        None => {
            let mut number = table_count + 1;
            loop {
                let candidate = format!("Table{number}");
                if !taken(&candidate) {
                    break TableName::new(candidate)?;
                }
                number += 1;
            }
        }
    };

    let sheet = sheet_by_id_mut(sheets, sheet_id)?;
    if range.start().row() == range.end().row() {
        return Err(ValidationError::TableRangeInvalid.into());
    }
    if let Some(other) = sheet
        .tables()
        .iter()
        .find(|other| ranges_overlap(range, other.range()))
    {
        return Err(ValidationError::OverlappingTables {
            sheet_id: sheet_id.get(),
            first_table_id: other.id().get(),
            second_table_id: table_id.get(),
        }
        .into());
    }
    if sheet
        .auto_filter()
        .is_some_and(|filter| ranges_overlap(range, filter.range()))
    {
        return Err(ValidationError::SheetAutoFilterOverlapsTable {
            sheet_id: sheet_id.get(),
            table_id: table_id.get(),
        }
        .into());
    }
    if let Some(blocking) = sheet
        .merged_ranges()
        .iter()
        .chain(sheet.array_formula_ranges().values())
        .find(|blocking| ranges_overlap(range, **blocking))
    {
        return Err(ValidationError::TableRangeOverlap {
            sheet_id: sheet_id.get(),
            row: blocking.start().row().get(),
            column: blocking.start().column().get(),
        }
        .into());
    }

    let mut columns = Vec::new();
    let mut generated = Vec::new();
    for (index, column) in (range.start().column().get()..=range.end().column().get()).enumerate() {
        let header = CellAddress::new(
            range.start().row(),
            Column::new(column).expect("validated table range column"),
        );
        let column_name = match sheet.cell(header).map(Cell::content) {
            Some(CellContent::Literal(CellValue::Text(text))) => {
                TableColumnName::new(text.as_str())?
            }
            None | Some(CellContent::Literal(CellValue::Blank)) => {
                let name = TableColumnName::new(format!("Column{}", index + 1))?;
                generated.push((header, name.as_str().to_owned()));
                name
            }
            Some(_) => return Err(table_header_edit_error(table_id, header, false).into()),
        };
        if columns.iter().any(|existing: &TableColumn| {
            case_insensitive_eq(existing.name(), column_name.as_str())
        }) {
            return Err(ValidationError::DuplicateTableColumnName {
                name: column_name.as_str().to_owned(),
            }
            .into());
        }
        let column_id = u32::try_from(index + 1).map_err(|_| {
            ValidationError::UnsupportedTableAuthoringMetadata { table_id: next_id }
        })?;
        columns.push(TableColumn::new(column_id, column_name.as_str(), None)?);
    }
    for (header, text) in generated {
        upsert_literal_text(sheet, header, &text);
        state.mark_cell(sheet_id, header);
    }
    let table = Table::new(table_id, name.clone(), name, range, 1, 0, columns)?
        .try_with_metadata(
            TableType::Worksheet,
            true,
            Some(TableAutoFilter::from_xlsx(range, true, Vec::new(), None)),
            None,
            Some(TableStyleInfo::new(
                Some(DEFAULT_TABLE_STYLE.to_owned()),
                false,
                false,
                true,
                false,
            )),
            None,
        )
        .map_err(|_| ValidationError::UnsupportedTableAuthoringMetadata { table_id: next_id })?;
    let mut tables = sheet.tables().to_vec();
    tables.push(table);
    sheet.set_tables(tables);
    state.touched_sheets.insert(sheet_id);
    state.changed_table_ids.insert(table_id);
    Ok(table_id)
}

#[cfg(test)]
#[path = "table_create_tests.rs"]
mod tests;
