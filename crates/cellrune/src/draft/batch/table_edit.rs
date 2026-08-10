use std::collections::{BTreeMap, BTreeSet};

use super::super::annotated_text_replacement_required;
use super::formula_edit::{
    FormulaEditState, TableFormulaLocations, WorkbookFormulaEdit, WorkbookFormulaRename,
    rewrite_workbook_formulas,
};
use super::staged::mark_upsert;
use super::{BatchExecutionError, TableMaterializationBudget};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, render_unqualified_structured_column,
};
use crate::{
    CalculationCellId, CellAddress, CellContent, CellRange, CellValue, FormulaCell, FormulaDialect,
    FormulaMetadata, FormulaText, NumberFormat, Row, SavedResult, Sheet, SheetId, Table,
    TableColumn, TableColumnId, TableColumnName, TableId, TableName, TotalsRowFunction,
    ValidationError, case_insensitive_eq,
};

pub(super) type TableLocations = BTreeMap<TableId, (usize, usize)>;

pub(super) struct TableEditState<'a> {
    pub(super) mutations: &'a mut super::super::DraftCellMutationStore,
    pub(super) changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) calculation_changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) touched_sheets: &'a mut BTreeSet<SheetId>,
    pub(super) changed_table_ids: &'a mut BTreeSet<TableId>,
    pub(super) presentation: &'a crate::DocumentPresentation,
}

impl TableEditState<'_> {
    fn mark_cell(&mut self, sheet_id: SheetId, address: CellAddress) {
        mark_upsert(self.mutations, sheet_id, address, false);
        let cell = CalculationCellId::new(sheet_id, address);
        self.changed_cells.insert(cell);
        self.calculation_changed_cells.insert(cell);
        self.touched_sheets.insert(sheet_id);
    }
}

pub(super) struct TableFormulaEdit<'a, 'cancel> {
    pub(super) state: TableEditState<'a>,
    pub(super) locations: &'a TableLocations,
    pub(super) formula_locations: &'a TableFormulaLocations,
    pub(super) budget: &'a mut FormulaRewriteBudget<'cancel>,
}

pub(super) struct TableResizeEdit<'a, 'rewrite_cancel, 'materialization_cancel> {
    pub(super) state: TableEditState<'a>,
    pub(super) locations: &'a TableLocations,
    pub(super) rewrite_budget: &'a mut FormulaRewriteBudget<'rewrite_cancel>,
    pub(super) materialization_budget: &'a mut TableMaterializationBudget<'materialization_cancel>,
}

pub(super) fn rename_table(
    sheets: &mut [Sheet],
    defined_names: &mut [crate::DefinedName],
    edit: TableFormulaEdit<'_, '_>,
    table_id: TableId,
    new_name: &TableName,
) -> Result<bool, BatchExecutionError> {
    let TableFormulaEdit {
        state,
        locations,
        formula_locations,
        budget,
    } = edit;
    new_name.validate_xlsx()?;
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let table = &sheets[sheet_index].tables()[table_index];
    if table.name() == new_name && table.display_name() == new_name {
        return Ok(false);
    }
    let old_name = table.display_name().as_str().to_owned();
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state: FormulaEditState {
                mutations: &mut *state.mutations,
                changed_cells: &mut *state.changed_cells,
                calculation_changed_cells: &mut *state.calculation_changed_cells,
                touched_sheets: &mut *state.touched_sheets,
            },
            changed_table_ids: &mut *state.changed_table_ids,
            table_formula_locations: formula_locations,
            budget,
        },
        WorkbookFormulaRename::Table {
            old_name: &old_name,
            new_name: new_name.as_str(),
        },
    )?;
    sheets[sheet_index].tables_mut()[table_index].rename(new_name.clone());
    state.changed_table_ids.insert(table_id);
    Ok(true)
}

pub(super) fn rename_table_column(
    sheets: &mut [Sheet],
    defined_names: &mut [crate::DefinedName],
    edit: TableFormulaEdit<'_, '_>,
    table_id: TableId,
    column_id: TableColumnId,
    new_name: &TableColumnName,
) -> Result<bool, BatchExecutionError> {
    let TableFormulaEdit {
        mut state,
        locations,
        formula_locations,
        budget,
    } = edit;
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let table = &sheets[sheet_index].tables()[table_index];
    let column_index = table
        .columns()
        .iter()
        .position(|column| column.column_id() == column_id)
        .ok_or(ValidationError::UnknownTableColumnId {
            table_id: table_id.get(),
            column_id: column_id.get(),
        })?;
    let old_name = table.columns()[column_index].name().to_owned();
    if old_name == new_name.as_str() {
        return Ok(false);
    }
    if table.header_row_count() > 1 {
        return Err(ValidationError::UnsupportedTableAuthoringMetadata {
            table_id: table_id.get(),
        }
        .into());
    }
    if table.columns().iter().enumerate().any(|(index, column)| {
        index != column_index && case_insensitive_eq(column.name(), new_name.as_str())
    }) {
        return Err(ValidationError::DuplicateTableColumnName {
            name: new_name.as_str().to_owned(),
        }
        .into());
    }
    let table_name = table.display_name().as_str().to_owned();
    let table_range = table.range();
    let header = (table.header_row_count() > 0).then(|| {
        let column = table.range().start().column().get()
            + u32::try_from(column_index).expect("table columns fit Excel bounds");
        CellAddress::new(
            table.range().start().row(),
            crate::Column::new(column).expect("validated table column"),
        )
    });
    if let Some(address) = header {
        validate_rename_header(
            &sheets[sheet_index],
            table_id,
            address,
            &old_name,
            new_name.as_str(),
        )?;
        if state
            .presentation
            .has_cell_annotation(sheets[sheet_index].id(), address)
            && header_needs_write(&sheets[sheet_index], address, new_name.as_str())
        {
            return Err(
                annotated_text_replacement_required(sheets[sheet_index].id(), address).into(),
            );
        }
    }
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state: FormulaEditState {
                mutations: &mut *state.mutations,
                changed_cells: &mut *state.changed_cells,
                calculation_changed_cells: &mut *state.calculation_changed_cells,
                touched_sheets: &mut *state.touched_sheets,
            },
            changed_table_ids: &mut *state.changed_table_ids,
            table_formula_locations: formula_locations,
            budget,
        },
        WorkbookFormulaRename::TableColumn {
            table_id,
            target_sheet_index: sheet_index,
            target_range: table_range,
            table_name: &table_name,
            old_name: &old_name,
            new_name: new_name.as_str(),
        },
    )?;
    if !sheets[sheet_index].tables_mut()[table_index].rename_column(column_id, new_name) {
        return Err(ValidationError::UnknownTableColumnId {
            table_id: table_id.get(),
            column_id: column_id.get(),
        }
        .into());
    }
    if let Some(address) = header
        && header_needs_write(&sheets[sheet_index], address, new_name.as_str())
    {
        let sheet_id = sheets[sheet_index].id();
        upsert_literal_text(&mut sheets[sheet_index], address, new_name.as_str());
        state.mark_cell(sheet_id, address);
    }
    state.changed_table_ids.insert(table_id);
    Ok(true)
}

pub(super) fn resize_table_rows(
    sheets: &mut [Sheet],
    edit: TableResizeEdit<'_, '_, '_>,
    table_id: TableId,
    first_data_row: Row,
    last_data_row: Row,
) -> Result<bool, BatchExecutionError> {
    let TableResizeEdit {
        mut state,
        locations,
        rewrite_budget,
        materialization_budget,
    } = edit;
    rewrite_budget.check_cancelled()?;
    if first_data_row > last_data_row {
        return Err(ValidationError::TableDataRowsReversed {
            first_data_row: first_data_row.get(),
            last_data_row: last_data_row.get(),
        }
        .into());
    }
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let old_table = sheets[sheet_index].tables()[table_index].clone();
    if old_table.data_range().is_some_and(|old_data_range| {
        old_data_range.start().row() == first_data_row
            && old_data_range.end().row() == last_data_row
    }) {
        return Ok(false);
    }
    if old_table.header_row_count() > 1 || old_table.totals_row_count() > 1 {
        return Err(ValidationError::UnsupportedTableAuthoringMetadata {
            table_id: table_id.get(),
        }
        .into());
    }
    if first_data_row.get() <= old_table.header_row_count() {
        return Err(ValidationError::TableResizeHeaderUnderflow {
            table_id: table_id.get(),
            first_data_row: first_data_row.get(),
            header_row_count: old_table.header_row_count(),
        }
        .into());
    }
    let mut resized = old_table.clone();
    resized
        .resize_data_rows(first_data_row, last_data_row)
        .map_err(|()| ValidationError::UnsupportedTableAuthoringMetadata {
            table_id: table_id.get(),
        })?;
    for other in sheets[sheet_index].tables() {
        if other.id() != table_id && ranges_overlap(resized.range(), other.range()) {
            return Err(ValidationError::OverlappingTables {
                sheet_id: sheets[sheet_index].id().get(),
                first_table_id: table_id.get().min(other.id().get()),
                second_table_id: table_id.get().max(other.id().get()),
            }
            .into());
        }
    }
    visit_resize_materialization_targets(
        &old_table,
        &resized,
        materialization_budget,
        true,
        |address, content| {
            validate_materialization_target(&sheets[sheet_index], table_id, address, &content)?;
            Ok(())
        },
    )?;
    let sheet_id = sheets[sheet_index].id();
    visit_resize_materialization_targets(
        &old_table,
        &resized,
        materialization_budget,
        false,
        |address, content| {
            if content_is_semantically_equal(
                sheets[sheet_index].cell(address).map(|cell| cell.content()),
                &content,
            ) {
                return Ok(());
            }
            let number_format = sheets[sheet_index]
                .cell(address)
                .map_or_else(NumberFormat::default, |cell| cell.number_format().clone());
            sheets[sheet_index].upsert_cell_deferred(address, content, number_format);
            state.mark_cell(sheet_id, address);
            Ok(())
        },
    )?;
    sheets[sheet_index].tables_mut()[table_index] = resized;
    state.changed_table_ids.insert(table_id);
    Ok(true)
}

pub(super) fn table_locations(sheets: &[Sheet]) -> Result<TableLocations, ValidationError> {
    let mut locations = BTreeMap::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        for (table_index, table) in sheet.tables().iter().enumerate() {
            if locations
                .insert(table.id(), (sheet_index, table_index))
                .is_some()
            {
                return Err(ValidationError::DuplicateTableId {
                    id: table.id().get(),
                });
            }
        }
    }
    Ok(locations)
}

fn table_location(
    locations: &TableLocations,
    table_id: TableId,
) -> Result<(usize, usize), ValidationError> {
    locations
        .get(&table_id)
        .copied()
        .ok_or(ValidationError::UnknownTableId {
            value: table_id.get(),
        })
}

fn validate_rename_header(
    sheet: &Sheet,
    table_id: TableId,
    address: CellAddress,
    old_name: &str,
    new_name: &str,
) -> Result<(), ValidationError> {
    let acceptable = match sheet.cell(address).map(|cell| cell.content()) {
        None | Some(CellContent::Literal(CellValue::Blank)) => true,
        Some(CellContent::Literal(CellValue::Text(value))) => {
            value == old_name || value == new_name
        }
        Some(CellContent::Literal(_)) | Some(CellContent::Formula(_)) => false,
    };
    if acceptable {
        Ok(())
    } else {
        Err(collision(sheet, table_id, address))
    }
}

fn upsert_literal_text(sheet: &mut Sheet, address: CellAddress, value: &str) {
    let number_format = sheet
        .cell(address)
        .map_or_else(NumberFormat::default, |cell| cell.number_format().clone());
    sheet.upsert_cell_deferred(
        address,
        CellContent::Literal(CellValue::Text(value.to_owned())),
        number_format,
    );
}

fn header_needs_write(sheet: &Sheet, address: CellAddress, value: &str) -> bool {
    !matches!(
        sheet.cell(address).map(|cell| cell.content()),
        Some(CellContent::Literal(CellValue::Text(current))) if current == value
    )
}

fn visit_resize_materialization_targets(
    old_table: &Table,
    resized: &Table,
    budget: &mut TableMaterializationBudget<'_>,
    charge_targets: bool,
    mut visit: impl FnMut(CellAddress, CellContent) -> Result<(), BatchExecutionError>,
) -> Result<(), BatchExecutionError> {
    budget.check_cancelled()?;
    if resized.header_row_count() > 0
        && resized.range().start().row() != old_table.range().start().row()
    {
        for (index, column) in resized.columns().iter().enumerate() {
            budget.check_cancelled()?;
            if charge_targets {
                budget.charge_cell()?;
            }
            visit(
                address_for_column(resized, resized.range().start().row(), index),
                CellContent::Literal(CellValue::Text(column.name().to_owned())),
            )?;
        }
    }
    if resized.totals_row_count() > 0
        && resized.range().end().row() != old_table.range().end().row()
    {
        for (index, column) in resized.columns().iter().enumerate() {
            budget.check_cancelled()?;
            let Some(content) = totals_content(resized.id(), column)? else {
                continue;
            };
            if charge_targets {
                budget.charge_cell()?;
            }
            visit(
                address_for_column(resized, resized.range().end().row(), index),
                content,
            )?;
        }
    }
    let data_range =
        resized
            .data_range()
            .ok_or(ValidationError::UnsupportedTableAuthoringMetadata {
                table_id: resized.id().get(),
            })?;
    let mut calculated_columns = Vec::new();
    for (index, column) in resized.columns().iter().enumerate() {
        budget.check_cancelled()?;
        if let Some(formula) = column.calculated_column_formula() {
            calculated_columns.push((index, formula));
        }
    }
    if calculated_columns.is_empty() {
        return Ok(());
    }
    for row in data_range.start().row().get()..=data_range.end().row().get() {
        budget.check_cancelled()?;
        if row >= old_table.range().start().row().get()
            && row <= old_table.range().end().row().get()
        {
            continue;
        }
        let row = Row::new(row).expect("validated data range row");
        for (index, formula) in &calculated_columns {
            budget.check_cancelled()?;
            if formula.is_array() {
                return Err(ValidationError::UnsupportedTableAuthoringMetadata {
                    table_id: resized.id().get(),
                }
                .into());
            }
            if charge_targets {
                budget.charge_cell()?;
            }
            visit(
                address_for_column(resized, row, *index),
                formula_content(formula.text().clone()),
            )?;
        }
    }
    Ok(())
}

fn address_for_column(table: &Table, row: Row, column_index: usize) -> CellAddress {
    let column = table.range().start().column().get()
        + u32::try_from(column_index).expect("table width fits usize");
    CellAddress::new(
        row,
        crate::Column::new(column).expect("validated table column"),
    )
}

fn totals_content(
    table_id: TableId,
    column: &TableColumn,
) -> Result<Option<CellContent>, ValidationError> {
    if let Some(label) = column.totals_row_label() {
        return Ok(Some(CellContent::Literal(CellValue::Text(
            label.to_owned(),
        ))));
    }
    if let Some(formula) = column.totals_row_formula() {
        if formula.is_array() {
            return Err(ValidationError::UnsupportedTableAuthoringMetadata {
                table_id: table_id.get(),
            });
        }
        return Ok(Some(formula_content(formula.text().clone())));
    }
    let Some(function) = column.totals_row_function() else {
        return Ok(None);
    };
    if function == TotalsRowFunction::Custom {
        return Ok(None);
    }
    let code = match function {
        TotalsRowFunction::Average => 101,
        TotalsRowFunction::CountNumbers => 102,
        TotalsRowFunction::Count => 103,
        TotalsRowFunction::Max => 104,
        TotalsRowFunction::Min => 105,
        TotalsRowFunction::StdDev => 107,
        TotalsRowFunction::Sum => 109,
        TotalsRowFunction::Var => 110,
        TotalsRowFunction::Custom => unreachable!("custom totals requires a stored formula"),
    };
    let column_reference = render_unqualified_structured_column(column.name());
    let formula = FormulaText::from_xlsx(format!("SUBTOTAL({code},{column_reference})"))?;
    Ok(Some(formula_content(formula)))
}

fn formula_content(text: FormulaText) -> CellContent {
    CellContent::Formula(FormulaCell::new(
        FormulaDialect::ExcelA1,
        text,
        SavedResult::Missing,
        FormulaMetadata::Normal,
    ))
}

fn validate_materialization_target(
    sheet: &Sheet,
    table_id: TableId,
    address: CellAddress,
    expected: &CellContent,
) -> Result<(), ValidationError> {
    let current = sheet.cell(address).map(|cell| cell.content());
    if current.is_none()
        || matches!(current, Some(CellContent::Literal(CellValue::Blank)))
        || content_is_semantically_equal(current, expected)
    {
        Ok(())
    } else {
        Err(collision(sheet, table_id, address))
    }
}

fn content_is_semantically_equal(current: Option<&CellContent>, expected: &CellContent) -> bool {
    match (current, expected) {
        (None, _) => false,
        (Some(CellContent::Literal(CellValue::Blank)), _) => false,
        (Some(CellContent::Literal(current)), CellContent::Literal(expected)) => {
            current == expected
        }
        (Some(CellContent::Formula(current)), CellContent::Formula(expected)) => {
            current.dialect() == expected.dialect()
                && current.text() == expected.text()
                && current.metadata() == expected.metadata()
        }
        (Some(CellContent::Literal(_)), CellContent::Formula(_))
        | (Some(CellContent::Formula(_)), CellContent::Literal(_)) => false,
    }
}

fn collision(sheet: &Sheet, table_id: TableId, address: CellAddress) -> ValidationError {
    ValidationError::TableMaterializationCollision {
        table_id: table_id.get(),
        sheet_id: sheet.id().get(),
        row: address.row().get(),
        column: address.column().get(),
    }
}

fn ranges_overlap(first: CellRange, second: CellRange) -> bool {
    first.start().row() <= second.end().row()
        && second.start().row() <= first.end().row()
        && first.start().column() <= second.end().column()
        && second.start().column() <= first.end().column()
}
