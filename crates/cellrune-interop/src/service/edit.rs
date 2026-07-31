//! Typed edit conversion, staging, and guarded installation.

use std::collections::BTreeMap;

use cellrune::{
    CalculationHints, CalculationMode, CancellationToken, CellAddress, CellRange, DateSystem,
    DefinedName, DefinedNameScope, EditBatch, FormulaText, NumberFormat, NumberFormatKind,
    PreparedEditBatch, Row, SheetId, SheetName, SheetVisibility, TableColumnId, TableColumnName,
    TableId, TableName, WorkbookChange, WorkbookSnapshot,
};

use super::WorkbookSession;
use crate::convert::{edit_receipt, edit_receipt_v2, value_from_dto};
use crate::{
    EditBatchDto, EditBatchV2Dto, EditReceiptDto, EditReceiptV2Dto, InteropError, TableChangeV2Dto,
    WorkbookChangeDto, WorkbookChangeV2Dto, WritableCellValueDto,
};

impl WorkbookSession {
    /// Applies a typed edit batch atomically after checking the expected revision.
    ///
    /// # Errors
    ///
    /// Returns a stable revision, input, resource, or workbook-validation error. A failure leaves
    /// the workbook and installed calculation unchanged.
    pub fn apply_changes(
        &mut self,
        expected_revision: u64,
        batch: EditBatchDto,
    ) -> Result<EditReceiptDto, InteropError> {
        let prepared = self.prepare_changes(expected_revision, batch)?;
        self.install_changes(prepared)
    }

    /// Stages a typed edit batch without changing the live workbook session.
    ///
    /// # Errors
    ///
    /// Returns the same stable validation and revision errors as [`Self::apply_changes`].
    pub fn prepare_changes(
        &self,
        expected_revision: u64,
        batch: EditBatchDto,
    ) -> Result<PreparedChanges, InteropError> {
        let batch = convert_edit_batch(self.engine.workbook(), batch, &|| false)?;
        let prepared = self.engine.prepare_changes(expected_revision, batch)?;
        let receipt = edit_receipt(prepared.workbook(), prepared.receipt());
        Ok(PreparedChanges { prepared, receipt })
    }

    /// Installs a previously staged edit batch if its source revision remains current.
    ///
    /// # Errors
    ///
    /// Returns a stable revision error without changing the session when the prepared batch is
    /// stale.
    pub fn install_changes(
        &mut self,
        prepared: PreparedChanges,
    ) -> Result<EditReceiptDto, InteropError> {
        let PreparedChanges { prepared, receipt } = prepared;
        self.engine.install_changes(prepared)?;
        Ok(receipt)
    }

    /// Applies an edit-schema-v2 batch, including stable-ID table authoring.
    ///
    /// # Errors
    ///
    /// Returns the same stable revision, input, resource, and workbook-validation errors as
    /// [`Self::apply_changes`].
    pub fn apply_changes_v2(
        &mut self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
    ) -> Result<EditReceiptV2Dto, InteropError> {
        let prepared = self.prepare_changes_v2(expected_revision, batch)?;
        self.install_changes_v2(prepared)
    }

    /// Stages an edit-schema-v2 batch without changing the live workbook session.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::apply_changes_v2`].
    pub fn prepare_changes_v2(
        &self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
    ) -> Result<PreparedChangesV2, InteropError> {
        let batch = convert_edit_batch_v2(self.engine.workbook(), batch, &|| false)?;
        let prepared = self.engine.prepare_changes(expected_revision, batch)?;
        let receipt = edit_receipt_v2(prepared.workbook(), prepared.receipt());
        Ok(PreparedChangesV2 { prepared, receipt })
    }

    /// Stages an edit-schema-v2 batch with cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::prepare_changes_v2`] plus `session.cancelled` when
    /// cancellation is requested before staging completes.
    pub fn prepare_changes_v2_cancellable(
        &self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
        cancellation: &CancellationToken,
    ) -> Result<PreparedChangesV2, InteropError> {
        let cancelled = || cancellation.is_cancelled();
        let batch = convert_edit_batch_v2(self.engine.workbook(), batch, &cancelled)?;
        let prepared =
            self.engine
                .prepare_changes_cancellable(expected_revision, batch, cancellation)?;
        let receipt = edit_receipt_v2(prepared.workbook(), prepared.receipt());
        Ok(PreparedChangesV2 { prepared, receipt })
    }

    /// Installs a previously staged edit-schema-v2 batch.
    ///
    /// # Errors
    ///
    /// Returns a stable revision error without changing the session when the prepared batch is
    /// stale.
    pub fn install_changes_v2(
        &mut self,
        prepared: PreparedChangesV2,
    ) -> Result<EditReceiptV2Dto, InteropError> {
        let PreparedChangesV2 { prepared, receipt } = prepared;
        self.engine.install_changes(prepared)?;
        Ok(receipt)
    }

    /// Sets a typed literal value or clears a cell when the value is blank.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet, invalid address, non-finite
    /// number, or unrecognized Excel error value.
    pub fn set_value(
        &mut self,
        sheet: &str,
        address: &str,
        value: WritableCellValueDto,
    ) -> Result<(), InteropError> {
        let revision = self.engine.workbook().semantic_revision();
        self.apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::SetValue {
                    sheet: sheet.to_owned(),
                    address: address.to_owned(),
                    value,
                }],
            },
        )?;
        Ok(())
    }

    /// Sets a user-entered formula and optionally marks it as a dynamic-array formula.
    ///
    /// `formula` must begin with `=`. `dynamic_range`, when supplied, uses `A1:B2` notation and
    /// must start at `address`.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet, invalid address or range,
    /// or invalid formula text.
    pub fn set_formula(
        &mut self,
        sheet: &str,
        address: &str,
        formula: &str,
        dynamic_range: Option<&str>,
    ) -> Result<(), InteropError> {
        let revision = self.engine.workbook().semantic_revision();
        self.apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::SetFormula {
                    sheet: sheet.to_owned(),
                    address: address.to_owned(),
                    formula: formula.to_owned(),
                    dynamic_range: dynamic_range.map(str::to_owned),
                }],
            },
        )?;
        Ok(())
    }

    /// Clears one sparse cell and returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet or invalid address.
    pub fn clear_cell(&mut self, sheet: &str, address: &str) -> Result<bool, InteropError> {
        let existed = self
            .engine
            .workbook()
            .sheet_by_name(sheet)
            .and_then(|sheet| {
                CellAddress::from_a1(address)
                    .ok()
                    .and_then(|address| sheet.cell(address))
            })
            .is_some();
        let revision = self.engine.workbook().semantic_revision();
        self.apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::ClearCell {
                    sheet: sheet.to_owned(),
                    address: address.to_owned(),
                }],
            },
        )?;
        Ok(existed)
    }

    /// Adds a visible worksheet and returns its stable workbook-local ID.
    ///
    /// # Errors
    ///
    /// Returns a validation error for an invalid or duplicate sheet name.
    pub fn add_sheet(&mut self, name: &str) -> Result<u32, InteropError> {
        let revision = self.engine.workbook().semantic_revision();
        let receipt = self.apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::AddSheet {
                    name: name.to_owned(),
                }],
            },
        )?;
        receipt
            .created_sheet_ids
            .first()
            .copied()
            .ok_or_else(InteropError::sheet_creation_failed)
    }

    /// Renames a worksheet and rewrites matching formula and defined-name references.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet or invalid new name.
    pub fn rename_sheet(&mut self, current_name: &str, new_name: &str) -> Result<(), InteropError> {
        let revision = self.engine.workbook().semantic_revision();
        self.apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::RenameSheet {
                    sheet: current_name.to_owned(),
                    new_name: new_name.to_owned(),
                }],
            },
        )?;
        Ok(())
    }
}

/// A typed workbook edit batch staged for guarded installation.
#[derive(Debug)]
pub struct PreparedChanges {
    prepared: PreparedEditBatch,
    receipt: EditReceiptDto,
}

/// A typed edit-schema-v2 batch staged for guarded installation.
#[derive(Debug)]
pub struct PreparedChangesV2 {
    prepared: PreparedEditBatch,
    receipt: EditReceiptV2Dto,
}

impl PreparedChangesV2 {
    /// Returns the exact v2 receipt that installation would commit.
    pub const fn receipt(&self) -> &EditReceiptV2Dto {
        &self.receipt
    }
}

impl PreparedChanges {
    /// Returns the exact receipt that installation would commit.
    pub const fn receipt(&self) -> &EditReceiptDto {
        &self.receipt
    }
}

fn parse_range(value: &str) -> Result<CellRange, InteropError> {
    let (start, end) = value.split_once(':').unwrap_or((value, value));
    Ok(CellRange::new(
        CellAddress::from_a1(start)?,
        CellAddress::from_a1(end)?,
    )?)
}

fn convert_edit_batch(
    workbook: &WorkbookSnapshot,
    batch: EditBatchDto,
    cancelled: &impl Fn() -> bool,
) -> Result<EditBatch, InteropError> {
    let mut sheets = workbook
        .sheets()
        .iter()
        .map(|sheet| (case_insensitive_key(sheet.name().as_str()), sheet.id()))
        .collect::<BTreeMap<_, _>>();
    let mut maximum_sheet_id = workbook
        .sheets()
        .iter()
        .map(|sheet| sheet.id().get())
        .max()
        .unwrap_or(0);
    let mut changes = Vec::with_capacity(batch.changes.len());
    for change in batch.changes {
        if cancelled() {
            return Err(InteropError::edit_cancelled());
        }
        let converted = match change {
            WorkbookChangeDto::SetValue {
                sheet,
                address,
                value,
            } => WorkbookChange::set_cell_value(
                resolve_sheet_map(&sheets, &sheet)?,
                CellAddress::from_a1(&address)?,
                value_from_dto(value)?,
            ),
            WorkbookChangeDto::SetFormula {
                sheet,
                address,
                formula,
                dynamic_range,
            } => {
                let sheet_id = resolve_sheet_map(&sheets, &sheet)?;
                let address = CellAddress::from_a1(&address)?;
                let formula = FormulaText::from_user_input(&formula)?;
                match dynamic_range {
                    Some(range) => WorkbookChange::set_cell_dynamic_formula(
                        sheet_id,
                        address,
                        formula,
                        Some(parse_range(&range)?),
                    )?,
                    None => WorkbookChange::set_cell_formula(sheet_id, address, formula),
                }
            }
            WorkbookChangeDto::ClearCell { sheet, address } => WorkbookChange::clear_cell(
                resolve_sheet_map(&sheets, &sheet)?,
                CellAddress::from_a1(&address)?,
            ),
            WorkbookChangeDto::SetNumberFormat {
                sheet,
                address,
                id,
                code,
                format_kind,
            } => {
                let kind = parse_number_format_kind(&format_kind)?;
                let number_format = match code {
                    Some(code) => NumberFormat::custom(id, code, kind)?,
                    None => NumberFormat::built_in(id, kind)?,
                };
                WorkbookChange::set_cell_number_format(
                    resolve_sheet_map(&sheets, &sheet)?,
                    CellAddress::from_a1(&address)?,
                    number_format,
                )
            }
            WorkbookChangeDto::AddSheet { name } => {
                let name = SheetName::new(name)?;
                let key = case_insensitive_key(name.as_str());
                if sheets.contains_key(&key) {
                    return Err(InteropError::invalid_change(
                        "added sheet name is already present".to_owned(),
                    ));
                }
                maximum_sheet_id = maximum_sheet_id
                    .checked_add(1)
                    .ok_or_else(InteropError::sheet_creation_failed)?;
                let sheet_id = SheetId::new(maximum_sheet_id)?;
                sheets.insert(key, sheet_id);
                WorkbookChange::add_sheet(name)
            }
            WorkbookChangeDto::RenameSheet { sheet, new_name } => {
                let sheet_id = resolve_sheet_map(&sheets, &sheet)?;
                let new_name = SheetName::new(new_name)?;
                let old_key = case_insensitive_key(&sheet);
                let new_key = case_insensitive_key(new_name.as_str());
                if sheets
                    .get(&new_key)
                    .is_some_and(|existing| *existing != sheet_id)
                {
                    return Err(InteropError::invalid_change(
                        "renamed sheet name is already present".to_owned(),
                    ));
                }
                sheets.remove(&old_key);
                sheets.insert(new_key, sheet_id);
                WorkbookChange::rename_sheet(sheet_id, new_name)
            }
            WorkbookChangeDto::SetSheetVisibility { sheet, visibility } => {
                WorkbookChange::set_sheet_visibility(
                    resolve_sheet_map(&sheets, &sheet)?,
                    parse_visibility(&visibility)?,
                )
            }
            WorkbookChangeDto::SetDefinedName {
                name,
                scope_sheet,
                formula,
                hidden,
            } => {
                let scope = parse_defined_name_scope(&sheets, scope_sheet.as_deref())?;
                WorkbookChange::set_defined_name(DefinedName::new(
                    name,
                    scope,
                    FormulaText::from_user_input(&formula)?,
                    hidden,
                )?)
            }
            WorkbookChangeDto::RemoveDefinedName { name, scope_sheet } => {
                WorkbookChange::remove_defined_name(
                    parse_defined_name_scope(&sheets, scope_sheet.as_deref())?,
                    name,
                )
            }
            WorkbookChangeDto::SetDateSystem { date_system } => {
                WorkbookChange::set_date_system(match date_system.as_str() {
                    "excel_1900" => DateSystem::Excel1900,
                    "excel_1904" => DateSystem::Excel1904,
                    _ => {
                        return Err(InteropError::invalid_change(
                            "date_system must be excel_1900 or excel_1904".to_owned(),
                        ));
                    }
                })
            }
            WorkbookChangeDto::SetCalculationHints {
                mode,
                calculation_id,
                full_calculation_on_load,
                force_full_calculation,
                iterative_calculation,
            } => WorkbookChange::set_calculation_hints(
                CalculationHints::new(
                    mode.as_deref().map(parse_calculation_mode).transpose()?,
                    calculation_id,
                    full_calculation_on_load,
                    force_full_calculation,
                )
                .with_iterative_calculation(iterative_calculation),
            ),
        };
        changes.push(converted);
    }
    Ok(EditBatch::new(changes))
}

fn convert_edit_batch_v2(
    workbook: &WorkbookSnapshot,
    batch: EditBatchV2Dto,
    cancelled: &impl Fn() -> bool,
) -> Result<EditBatch, InteropError> {
    enum Marker {
        V1,
        Table(TableChangeV2Dto),
    }

    let mut markers = Vec::with_capacity(batch.changes.len());
    let mut v1_changes = Vec::new();
    for change in batch.changes {
        if cancelled() {
            return Err(InteropError::edit_cancelled());
        }
        match change {
            WorkbookChangeV2Dto::V1(change) => {
                markers.push(Marker::V1);
                v1_changes.push(change);
            }
            WorkbookChangeV2Dto::Table(change) => markers.push(Marker::Table(change)),
        }
    }
    let converted_v1 = convert_edit_batch(
        workbook,
        EditBatchDto {
            changes: v1_changes,
        },
        cancelled,
    )?;
    let mut converted_v1 = converted_v1.changes().iter().cloned();
    let mut changes = Vec::with_capacity(markers.len());
    for marker in markers {
        if cancelled() {
            return Err(InteropError::edit_cancelled());
        }
        let change = match marker {
            Marker::V1 => converted_v1
                .next()
                .expect("one converted v1 operation for each marker"),
            Marker::Table(TableChangeV2Dto::RenameTable {
                table_id,
                new_display_name,
            }) => WorkbookChange::rename_table(
                TableId::new(table_id)?,
                TableName::new(new_display_name)?,
            ),
            Marker::Table(TableChangeV2Dto::RenameTableColumn {
                table_id,
                column_id,
                new_name,
            }) => WorkbookChange::rename_table_column(
                TableId::new(table_id)?,
                TableColumnId::new(column_id)?,
                TableColumnName::new(new_name)?,
            ),
            Marker::Table(TableChangeV2Dto::ResizeTableRows {
                table_id,
                first_data_row,
                last_data_row,
            }) => WorkbookChange::resize_table_rows(
                TableId::new(table_id)?,
                Row::new(first_data_row)?,
                Row::new(last_data_row)?,
            )?,
        };
        changes.push(change);
    }
    debug_assert!(converted_v1.next().is_none());
    Ok(EditBatch::new(changes))
}

fn resolve_sheet_map(
    sheets: &BTreeMap<String, SheetId>,
    name: &str,
) -> Result<SheetId, InteropError> {
    sheets
        .get(&case_insensitive_key(name))
        .copied()
        .ok_or_else(InteropError::sheet_not_found)
}

fn parse_number_format_kind(value: &str) -> Result<NumberFormatKind, InteropError> {
    match value {
        "general" => Ok(NumberFormatKind::General),
        "number" => Ok(NumberFormatKind::Number),
        "date" => Ok(NumberFormatKind::Date),
        "time" => Ok(NumberFormatKind::Time),
        "date_time" => Ok(NumberFormatKind::DateTime),
        "duration" => Ok(NumberFormatKind::Duration),
        _ => Err(InteropError::invalid_change(
            "format_kind is not recognized".to_owned(),
        )),
    }
}

fn parse_visibility(value: &str) -> Result<SheetVisibility, InteropError> {
    match value {
        "visible" => Ok(SheetVisibility::Visible),
        "hidden" => Ok(SheetVisibility::Hidden),
        "very_hidden" => Ok(SheetVisibility::VeryHidden),
        _ => Err(InteropError::invalid_change(
            "visibility is not recognized".to_owned(),
        )),
    }
}

fn parse_defined_name_scope(
    sheets: &BTreeMap<String, SheetId>,
    scope_sheet: Option<&str>,
) -> Result<DefinedNameScope, InteropError> {
    scope_sheet.map_or(Ok(DefinedNameScope::Workbook), |sheet| {
        resolve_sheet_map(sheets, sheet).map(DefinedNameScope::Sheet)
    })
}

fn parse_calculation_mode(value: &str) -> Result<CalculationMode, InteropError> {
    match value {
        "automatic" => Ok(CalculationMode::Automatic),
        "automatic_except_data_tables" => Ok(CalculationMode::AutomaticExceptDataTables),
        "manual" => Ok(CalculationMode::Manual),
        _ => Err(InteropError::invalid_change(
            "calculation mode is not recognized".to_owned(),
        )),
    }
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}
