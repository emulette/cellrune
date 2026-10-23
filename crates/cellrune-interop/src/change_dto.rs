//! Converts core workbook changes into their edit-schema-v2 DTOs, for the inverse an edit
//! receipt carries.

use std::collections::BTreeMap;

use cellrune::{
    CalculationHints, CalculationMode, CellRange, CellValue, DateSystem, DefinedNameScope,
    EditInverse, FormulaText, NumberFormat, NumberFormatKind, SheetId, WorkbookChange,
    WorkbookSnapshot,
};

use crate::convert::visibility_name;
use crate::format_convert::format_edit_dto;
use crate::{
    EditInverseDto, FormatChangeV2Dto, TableChangeV2Dto, UnrestoredContentDto, WorkbookChangeDto,
    WorkbookChangeV2Dto, WritableCellValueDto,
};

/// Converts the inverse of a batch that produced `workbook`. Sheets are named as they are when
/// each change applies, so a rename in the inverse renames the sheet for the changes after it,
/// and a sheet the inverse adds takes the next sheet ID, as applying the inverse to `workbook`
/// gives it. `None` when a change has no DTO spelling.
pub(crate) fn inverse_dto(
    workbook: &WorkbookSnapshot,
    inverse: &EditInverse,
) -> Option<EditInverseDto> {
    let initial = workbook
        .sheets()
        .iter()
        .map(|sheet| (sheet.id(), sheet.name().as_str().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let mut sheets = InverseSheets {
        names: initial.clone(),
        next_id: workbook.highest_sheet_id().map_or(1, |id| id.get() + 1),
    };
    let changes = inverse
        .changes()
        .iter()
        .map(|change| change_dto(change, &mut sheets))
        .collect::<Option<Vec<_>>>()?;
    let unrestored = inverse
        .unrestored()
        .iter()
        .map(|content| UnrestoredContentDto {
            kind: content.kind().as_str().to_owned(),
            sheet: content
                .sheet_id()
                .and_then(|sheet_id| initial.get(&sheet_id).cloned()),
            table_id: content.table_id().map(|table_id| table_id.get()),
        })
        .collect();
    Some(EditInverseDto {
        changes,
        unrestored,
    })
}

/// The sheets an inverse names as each of its changes applies.
struct InverseSheets {
    names: BTreeMap<SheetId, String>,
    /// The ID the next sheet the inverse adds takes.
    next_id: u32,
}

fn change_dto(change: &WorkbookChange, sheets: &mut InverseSheets) -> Option<WorkbookChangeV2Dto> {
    let names = &mut sheets.names;
    let name = |names: &BTreeMap<SheetId, String>, sheet_id: SheetId| names.get(&sheet_id).cloned();
    let v1 = WorkbookChangeV2Dto::V1;
    Some(match change {
        WorkbookChange::SetCellValue {
            sheet_id,
            address,
            value,
        } => v1(WorkbookChangeDto::SetValue {
            sheet: name(names, *sheet_id)?,
            address: address.to_string(),
            value: writable_value(value)?,
        }),
        WorkbookChange::SetCellFormula {
            sheet_id,
            address,
            formula,
        } => v1(WorkbookChangeDto::SetFormula {
            sheet: name(names, *sheet_id)?,
            address: address.to_string(),
            formula: user_formula(formula),
            dynamic_range: None,
            dynamic_array: false,
        }),
        // A dynamic formula is spelled with its spill range, or as dynamic without one.
        WorkbookChange::SetCellDynamicFormula {
            sheet_id,
            address,
            formula,
            range,
        } => v1(WorkbookChangeDto::SetFormula {
            sheet: name(names, *sheet_id)?,
            address: address.to_string(),
            formula: user_formula(formula),
            dynamic_range: range.map(range_text),
            dynamic_array: range.is_none(),
        }),
        WorkbookChange::ClearCell { sheet_id, address } => v1(WorkbookChangeDto::ClearCell {
            sheet: name(names, *sheet_id)?,
            address: address.to_string(),
        }),
        WorkbookChange::SetCellNumberFormat {
            sheet_id,
            address,
            number_format,
        } => {
            let (id, code, format_kind) = number_format_parts(number_format);
            v1(WorkbookChangeDto::SetNumberFormat {
                sheet: name(names, *sheet_id)?,
                address: address.to_string(),
                id,
                code,
                format_kind,
            })
        }
        WorkbookChange::RenameSheet {
            sheet_id,
            name: new,
        } => {
            let sheet = name(names, *sheet_id)?;
            names.insert(*sheet_id, new.as_str().to_owned());
            v1(WorkbookChangeDto::RenameSheet {
                sheet,
                new_name: new.as_str().to_owned(),
            })
        }
        WorkbookChange::DeleteSheet { sheet_id } => {
            let sheet = names.remove(sheet_id)?;
            v1(WorkbookChangeDto::DeleteSheet { sheet })
        }
        WorkbookChange::AddSheet { name: added } => {
            let sheet_id = SheetId::new(sheets.next_id).ok()?;
            sheets.next_id = sheets.next_id.checked_add(1)?;
            names.insert(sheet_id, added.as_str().to_owned());
            v1(WorkbookChangeDto::AddSheet {
                name: added.as_str().to_owned(),
            })
        }
        WorkbookChange::MoveSheet { sheet_id, position } => v1(WorkbookChangeDto::MoveSheet {
            sheet: name(names, *sheet_id)?,
            position: u32::try_from(*position).ok()?,
        }),
        WorkbookChange::SetSheetAutoFilter { sheet_id, range } => {
            v1(WorkbookChangeDto::SetSheetAutoFilter {
                sheet: name(names, *sheet_id)?,
                range: range.map(range_text),
            })
        }
        WorkbookChange::RenameDefinedName {
            scope,
            name: renamed,
            new_name,
        } => v1(WorkbookChangeDto::RenameDefinedName {
            name: renamed.to_string(),
            scope_sheet: scope_sheet(names, *scope)?,
            new_name: new_name.to_string(),
        }),
        WorkbookChange::CreateTable {
            sheet_id,
            range,
            name: table_name,
        } => WorkbookChangeV2Dto::Table(TableChangeV2Dto::CreateTable {
            sheet: name(names, *sheet_id)?,
            range: range_text(*range),
            name: table_name
                .as_ref()
                .map(|table_name| table_name.as_str().to_owned()),
        }),
        WorkbookChange::ConvertTableToRange { table_id } => {
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::ConvertTableToRange {
                table_id: table_id.get(),
            })
        }
        WorkbookChange::SetTableRange { table_id, range } => {
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::SetTableRange {
                table_id: table_id.get(),
                range: range_text(*range),
            })
        }
        WorkbookChange::SetSheetVisibility {
            sheet_id,
            visibility,
        } => v1(WorkbookChangeDto::SetSheetVisibility {
            sheet: name(names, *sheet_id)?,
            visibility: visibility_name(*visibility).to_owned(),
        }),
        WorkbookChange::SetDefinedName { defined_name } => v1(WorkbookChangeDto::SetDefinedName {
            name: defined_name.name().to_owned(),
            scope_sheet: scope_sheet(names, defined_name.scope())?,
            formula: user_formula(defined_name.formula()),
            hidden: defined_name.hidden(),
        }),
        WorkbookChange::RemoveDefinedName {
            scope,
            name: removed,
        } => v1(WorkbookChangeDto::RemoveDefinedName {
            name: removed.to_string(),
            scope_sheet: scope_sheet(names, *scope)?,
        }),
        WorkbookChange::SetDateSystem { date_system } => v1(WorkbookChangeDto::SetDateSystem {
            date_system: match date_system {
                DateSystem::Excel1900 => "excel_1900",
                DateSystem::Excel1904 => "excel_1904",
            }
            .to_owned(),
        }),
        WorkbookChange::SetCalculationHints { calculation_hints } => {
            calculation_hints_dto(*calculation_hints)
        }
        WorkbookChange::InsertRows {
            sheet_id,
            row,
            count,
        } => v1(WorkbookChangeDto::InsertRows {
            sheet: name(names, *sheet_id)?,
            row: row.get(),
            count: *count,
        }),
        WorkbookChange::DeleteRows {
            sheet_id,
            first,
            last,
        } => v1(WorkbookChangeDto::DeleteRows {
            sheet: name(names, *sheet_id)?,
            first: first.get(),
            last: last.get(),
        }),
        WorkbookChange::InsertColumns {
            sheet_id,
            column,
            count,
        } => v1(WorkbookChangeDto::InsertColumns {
            sheet: name(names, *sheet_id)?,
            column: column.get(),
            count: *count,
        }),
        WorkbookChange::DeleteColumns {
            sheet_id,
            first,
            last,
        } => v1(WorkbookChangeDto::DeleteColumns {
            sheet: name(names, *sheet_id)?,
            first: first.get(),
            last: last.get(),
        }),
        WorkbookChange::MoveRows {
            sheet_id,
            first,
            last,
            destination,
        } => v1(WorkbookChangeDto::MoveRows {
            sheet: name(names, *sheet_id)?,
            first: first.get(),
            last: last.get(),
            destination: destination.get(),
        }),
        WorkbookChange::MoveColumns {
            sheet_id,
            first,
            last,
            destination,
        } => v1(WorkbookChangeDto::MoveColumns {
            sheet: name(names, *sheet_id)?,
            first: first.get(),
            last: last.get(),
            destination: destination.get(),
        }),
        WorkbookChange::RenameTable {
            table_id,
            new_display_name,
        } => WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTable {
            table_id: table_id.get(),
            new_display_name: new_display_name.as_str().to_owned(),
        }),
        WorkbookChange::RenameTableColumn {
            table_id,
            column_id,
            new_name,
        } => WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTableColumn {
            table_id: table_id.get(),
            column_id: column_id.get(),
            new_name: new_name.as_str().to_owned(),
        }),
        WorkbookChange::ResizeTableRows {
            table_id,
            first_data_row,
            last_data_row,
        } => WorkbookChangeV2Dto::Table(TableChangeV2Dto::ResizeTableRows {
            table_id: table_id.get(),
            first_data_row: first_data_row.get(),
            last_data_row: last_data_row.get(),
        }),
        WorkbookChange::SetCellFormat {
            sheet_id,
            address,
            edit,
        } => WorkbookChangeV2Dto::Format(format_edit_dto(name(names, *sheet_id)?, *address, edit)),
        WorkbookChange::SetColumnWidth {
            sheet_id,
            first,
            last,
            width,
        } => WorkbookChangeV2Dto::Format(FormatChangeV2Dto::SetColumnWidth {
            sheet: name(names, *sheet_id)?,
            first: first.get(),
            last: last.get(),
            width: *width,
        }),
        WorkbookChange::SetRowHeight {
            sheet_id,
            row,
            height,
        } => WorkbookChangeV2Dto::Format(FormatChangeV2Dto::SetRowHeight {
            sheet: name(names, *sheet_id)?,
            row: row.get(),
            height: *height,
        }),
        _ => return None,
    })
}

fn writable_value(value: &CellValue) -> Option<WritableCellValueDto> {
    Some(match value {
        CellValue::Blank => WritableCellValueDto::Blank,
        CellValue::Number(number) => WritableCellValueDto::Number {
            value: number.get(),
        },
        CellValue::Text(text) => WritableCellValueDto::Text {
            value: text.clone(),
        },
        CellValue::Logical(value) => WritableCellValueDto::Logical { value: *value },
        CellValue::Error(error) => WritableCellValueDto::Error {
            value: error.as_str().to_owned(),
        },
        _ => return None,
    })
}

fn user_formula(formula: &FormulaText) -> String {
    format!("={}", formula.as_str())
}

fn range_text(range: CellRange) -> String {
    crate::convert::range_text(range.start(), range.end())
}

fn number_format_parts(number_format: &NumberFormat) -> (u32, Option<String>, String) {
    let kind = match number_format.kind() {
        NumberFormatKind::General => "general",
        NumberFormatKind::Number => "number",
        NumberFormatKind::Date => "date",
        NumberFormatKind::Time => "time",
        NumberFormatKind::DateTime => "date_time",
        NumberFormatKind::Duration => "duration",
    };
    let code = (number_format.id() >= 164)
        .then(|| number_format.code().map(str::to_owned))
        .flatten();
    (number_format.id(), code, kind.to_owned())
}

fn scope_sheet(
    names: &BTreeMap<SheetId, String>,
    scope: DefinedNameScope,
) -> Option<Option<String>> {
    match scope {
        DefinedNameScope::Workbook => Some(None),
        DefinedNameScope::Sheet(sheet_id) => names.get(&sheet_id).cloned().map(Some),
    }
}

fn calculation_hints_dto(hints: CalculationHints) -> WorkbookChangeV2Dto {
    WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetCalculationHints {
        mode: hints.mode().map(|mode| {
            match mode {
                CalculationMode::Automatic => "automatic",
                CalculationMode::AutomaticExceptDataTables => "automatic_except_data_tables",
                CalculationMode::Manual => "manual",
            }
            .to_owned()
        }),
        calculation_id: hints.calculation_id(),
        full_calculation_on_load: hints.full_calculation_on_load(),
        force_full_calculation: hints.force_full_calculation(),
        iterative_calculation: hints.iterative_calculation(),
    })
}
