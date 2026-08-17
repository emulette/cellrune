use cellrune_interop::{
    CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto, CalculationReportDto,
    CalculationResultDto, CellDto, CellReferenceDto, CellValueDto, EditReceiptDto,
    EditReceiptV2Dto, FunctionCatalogReportDto, FunctionUsageReportDto, RangePageDto,
    SavedValueStateDto, WorkbookSummaryDto, WriteReportDto,
};
use napi_derive::napi;

#[napi(object)]
pub struct NativeCellValue {
    pub kind: String,
    pub number_value: Option<f64>,
    pub text_value: Option<String>,
    pub logical_value: Option<bool>,
    pub error_value: Option<String>,
}

#[napi(object)]
pub struct NativeCalculationResult {
    pub kind: String,
    pub value: Option<NativeCellValue>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub detail: Option<String>,
}

#[napi(object)]
pub struct NativeCell {
    pub address: String,
    pub formula: Option<String>,
    pub source_value: NativeCellValue,
    pub source_value_state: String,
    pub calculated: Option<NativeCalculationResult>,
}

#[napi(object)]
pub struct NativeRangePage {
    pub schema_version: u32,
    pub sheet: String,
    pub start: String,
    pub end: String,
    pub total_cells: f64,
    pub offset: f64,
    pub next_offset: Option<f64>,
    pub cells: Vec<NativeCell>,
}

#[napi(object)]
pub struct NativeTableColumn {
    pub id: u32,
    pub name: String,
    pub totals_row_function: Option<String>,
}

#[napi(object)]
pub struct NativeTableSummary {
    pub id: u32,
    pub name: String,
    pub display_name: String,
    pub range: String,
    pub header_row_count: u32,
    pub totals_row_count: u32,
    pub columns: Vec<NativeTableColumn>,
}

#[napi(object)]
pub struct NativeSheetSummary {
    pub id: u32,
    pub name: String,
    pub visibility: String,
    pub cell_count: f64,
    pub used_range: Option<String>,
    pub merged_ranges: Vec<String>,
    pub tables: Vec<NativeTableSummary>,
}

#[napi(object)]
pub struct NativeWorkbookSummary {
    pub schema_version: u32,
    pub semantic_revision: String,
    pub fingerprint: NativeWorkbookFingerprint,
    pub document_backed: bool,
    pub document_kind: String,
    pub date_system: String,
    pub diagnostic_count: f64,
    pub sheets: Vec<NativeSheetSummary>,
}

#[napi(object)]
pub struct NativeWorkbookFingerprint {
    pub schema_version: u16,
    pub digest_hex: String,
}

#[napi(object)]
pub struct NativeCalculationReport {
    pub schema_version: u32,
    pub semantic_revision: String,
    pub formula_count: f64,
    pub value_count: f64,
    pub unavailable_count: f64,
    pub materialized_cell_count: f64,
}

#[napi(object)]
pub struct NativeCalculationDeltaCell {
    pub cell: NativeCellReference,
    pub origin: String,
    pub anchor: Option<NativeCellReference>,
    pub range: Option<String>,
    pub result: NativeCalculationResult,
}

#[napi(object)]
pub struct NativeCalculationDelta {
    pub schema_version: u32,
    pub cursor: String,
    pub base_revision: String,
    pub result_revision: String,
    pub mode: String,
    pub reason: String,
    pub dirty_count: f64,
    pub evaluated_count: f64,
    pub parsed_formula_count: f64,
    pub changed_cells: Vec<NativeCalculationDeltaCell>,
    pub removed_materialized_cells: Vec<NativeCellReference>,
}

#[napi(object)]
pub struct NativeCalculationDeltaPage {
    pub schema_version: u32,
    pub requested_cursor: String,
    pub next_cursor: Option<String>,
    pub deltas: Vec<NativeCalculationDelta>,
}

#[napi(object)]
pub struct NativeEditReceipt {
    pub schema_version: u32,
    pub base_revision: String,
    pub result_revision: String,
    pub applied_change_count: f64,
    pub changed_cells: Vec<NativeCellReference>,
    pub calculation_changed_cells: Vec<NativeCellReference>,
    pub created_sheet_ids: Vec<u32>,
    pub topology_changed: bool,
    pub calculation_metadata_changed: bool,
}

#[napi(object)]
pub struct NativeEditReceiptV2 {
    pub schema_version: u32,
    pub base_revision: String,
    pub result_revision: String,
    pub applied_change_count: f64,
    pub changed_cells: Vec<NativeCellReference>,
    pub calculation_changed_cells: Vec<NativeCellReference>,
    pub created_sheet_ids: Vec<u32>,
    pub topology_changed: bool,
    pub calculation_metadata_changed: bool,
    pub changed_table_ids: Vec<u32>,
}

#[napi(object)]
pub struct NativeCellReference {
    pub sheet_id: u32,
    pub sheet_name: String,
    pub address: String,
}

#[napi(object)]
pub struct NativeFunctionUsageEntry {
    pub name: String,
    pub supported: bool,
    pub call_count: f64,
    pub formula_count: f64,
    pub sample_cells: Vec<NativeCellReference>,
}

#[napi(object)]
pub struct NativeFunctionUsageReport {
    pub schema_version: u32,
    pub formula_count: f64,
    pub parsed_formula_count: f64,
    pub unparsed_formula_count: f64,
    pub entries: Vec<NativeFunctionUsageEntry>,
}

#[napi(object)]
pub struct NativeFunctionCatalogEntry {
    pub name: String,
    pub canonical_name: String,
    pub alias: bool,
    pub returns_array: bool,
    pub official: bool,
}

#[napi(object)]
pub struct NativeFunctionCatalogReport {
    pub schema_version: u32,
    pub entries: Vec<NativeFunctionCatalogEntry>,
}

#[napi(object)]
pub struct NativeWriteReport {
    pub schema_version: u32,
    pub complete: bool,
    pub policy: String,
    pub materialized_count: f64,
    pub invalidated_cells: Vec<NativeCellReference>,
    pub changed_parts: Vec<String>,
    pub removed_parts: Vec<String>,
    pub diagnostic_count: f64,
    pub output_sha256: String,
}

pub(crate) fn workbook_summary(value: WorkbookSummaryDto) -> NativeWorkbookSummary {
    NativeWorkbookSummary {
        schema_version: value.schema_version,
        semantic_revision: value.semantic_revision.to_string(),
        fingerprint: NativeWorkbookFingerprint {
            schema_version: value.fingerprint.schema_version,
            digest_hex: value.fingerprint.digest_hex,
        },
        document_backed: value.document_backed,
        document_kind: value.document_kind,
        date_system: value.date_system,
        diagnostic_count: value.diagnostic_count as f64,
        sheets: value
            .sheets
            .into_iter()
            .map(|sheet| NativeSheetSummary {
                id: sheet.id,
                name: sheet.name,
                visibility: sheet.visibility,
                cell_count: sheet.cell_count as f64,
                used_range: sheet.used_range,
                merged_ranges: sheet.merged_ranges,
                tables: sheet
                    .tables
                    .into_iter()
                    .map(|table| NativeTableSummary {
                        id: table.id,
                        name: table.name,
                        display_name: table.display_name,
                        range: table.range,
                        header_row_count: table.header_row_count,
                        totals_row_count: table.totals_row_count,
                        columns: table
                            .columns
                            .into_iter()
                            .map(|column| NativeTableColumn {
                                id: column.id,
                                name: column.name,
                                totals_row_function: column.totals_row_function,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(crate) fn calculation_report(value: CalculationReportDto) -> NativeCalculationReport {
    NativeCalculationReport {
        schema_version: value.schema_version,
        semantic_revision: value.semantic_revision.to_string(),
        formula_count: value.formula_count as f64,
        value_count: value.value_count as f64,
        unavailable_count: value.unavailable_count as f64,
        materialized_cell_count: value.materialized_cell_count as f64,
    }
}

pub(crate) fn calculation_delta(value: CalculationDeltaDto) -> NativeCalculationDelta {
    NativeCalculationDelta {
        schema_version: value.schema_version,
        cursor: value.cursor.to_string(),
        base_revision: value.base_revision.to_string(),
        result_revision: value.result_revision.to_string(),
        mode: value.mode,
        reason: value.reason,
        dirty_count: value.dirty_count as f64,
        evaluated_count: value.evaluated_count as f64,
        parsed_formula_count: value.parsed_formula_count as f64,
        changed_cells: value
            .changed_cells
            .into_iter()
            .map(calculation_delta_cell)
            .collect(),
        removed_materialized_cells: value
            .removed_materialized_cells
            .into_iter()
            .map(cell_reference)
            .collect(),
    }
}

pub(crate) fn calculation_delta_page(value: CalculationDeltaPageDto) -> NativeCalculationDeltaPage {
    NativeCalculationDeltaPage {
        schema_version: value.schema_version,
        requested_cursor: value.requested_cursor.to_string(),
        next_cursor: value.next_cursor.map(|cursor| cursor.to_string()),
        deltas: value.deltas.into_iter().map(calculation_delta).collect(),
    }
}

pub(crate) fn edit_receipt(value: EditReceiptDto) -> NativeEditReceipt {
    NativeEditReceipt {
        schema_version: value.schema_version,
        base_revision: value.base_revision.to_string(),
        result_revision: value.result_revision.to_string(),
        applied_change_count: value.applied_change_count as f64,
        changed_cells: value
            .changed_cells
            .into_iter()
            .map(cell_reference)
            .collect(),
        calculation_changed_cells: value
            .calculation_changed_cells
            .into_iter()
            .map(cell_reference)
            .collect(),
        created_sheet_ids: value.created_sheet_ids,
        topology_changed: value.topology_changed,
        calculation_metadata_changed: value.calculation_metadata_changed,
    }
}

pub(crate) fn edit_receipt_v2(value: EditReceiptV2Dto) -> NativeEditReceiptV2 {
    let receipt = edit_receipt(value.receipt);
    NativeEditReceiptV2 {
        schema_version: receipt.schema_version,
        base_revision: receipt.base_revision,
        result_revision: receipt.result_revision,
        applied_change_count: receipt.applied_change_count,
        changed_cells: receipt.changed_cells,
        calculation_changed_cells: receipt.calculation_changed_cells,
        created_sheet_ids: receipt.created_sheet_ids,
        topology_changed: receipt.topology_changed,
        calculation_metadata_changed: receipt.calculation_metadata_changed,
        changed_table_ids: value.changed_table_ids,
    }
}

pub(crate) fn range_page(value: RangePageDto) -> NativeRangePage {
    NativeRangePage {
        schema_version: value.schema_version,
        sheet: value.sheet,
        start: value.start,
        end: value.end,
        total_cells: value.total_cells as f64,
        offset: value.offset as f64,
        next_offset: value.next_offset.map(|offset| offset as f64),
        cells: value.cells.into_iter().map(cell).collect(),
    }
}

pub(crate) fn function_usage(value: FunctionUsageReportDto) -> NativeFunctionUsageReport {
    NativeFunctionUsageReport {
        schema_version: value.schema_version,
        formula_count: value.formula_count as f64,
        parsed_formula_count: value.parsed_formula_count as f64,
        unparsed_formula_count: value.unparsed_formula_count as f64,
        entries: value
            .entries
            .into_iter()
            .map(|entry| NativeFunctionUsageEntry {
                name: entry.name,
                supported: entry.supported,
                call_count: entry.call_count as f64,
                formula_count: entry.formula_count as f64,
                sample_cells: entry.sample_cells.into_iter().map(cell_reference).collect(),
            })
            .collect(),
    }
}

pub(crate) fn function_catalog(value: FunctionCatalogReportDto) -> NativeFunctionCatalogReport {
    NativeFunctionCatalogReport {
        schema_version: value.schema_version,
        entries: value
            .entries
            .into_iter()
            .map(|entry| NativeFunctionCatalogEntry {
                name: entry.name,
                canonical_name: entry.canonical_name,
                alias: entry.alias,
                returns_array: entry.returns_array,
                official: entry.official,
            })
            .collect(),
    }
}

pub(crate) fn write_report(value: WriteReportDto) -> NativeWriteReport {
    NativeWriteReport {
        schema_version: value.schema_version,
        complete: value.complete,
        policy: value.policy,
        materialized_count: value.materialized_count as f64,
        invalidated_cells: value
            .invalidated_cells
            .into_iter()
            .map(cell_reference)
            .collect(),
        changed_parts: value.changed_parts,
        removed_parts: value.removed_parts,
        diagnostic_count: value.diagnostic_count as f64,
        output_sha256: value.output_sha256,
    }
}

fn cell(value: CellDto) -> NativeCell {
    NativeCell {
        address: value.address,
        formula: value.formula,
        source_value: cell_value(value.source_value),
        source_value_state: match value.source_value_state {
            SavedValueStateDto::Literal => "literal",
            SavedValueStateDto::Saved => "saved",
            SavedValueStateDto::Missing => "missing",
            SavedValueStateDto::Invalid => "invalid",
        }
        .to_owned(),
        calculated: value.calculated.map(calculation_result),
    }
}

fn calculation_result(value: CalculationResultDto) -> NativeCalculationResult {
    match value {
        CalculationResultDto::Value { value } => NativeCalculationResult {
            kind: "value".to_owned(),
            value: Some(cell_value(value)),
            code: None,
            message: None,
            detail: None,
        },
        CalculationResultDto::Unavailable {
            code,
            message,
            detail,
        } => NativeCalculationResult {
            kind: "unavailable".to_owned(),
            value: None,
            code: Some(code),
            message: Some(message),
            detail,
        },
    }
}

fn calculation_delta_cell(value: CalculationDeltaCellDto) -> NativeCalculationDeltaCell {
    NativeCalculationDeltaCell {
        cell: cell_reference(value.cell),
        origin: value.origin,
        anchor: value.anchor.map(cell_reference),
        range: value.range,
        result: calculation_result(value.result),
    }
}

fn cell_value(value: CellValueDto) -> NativeCellValue {
    match value {
        CellValueDto::Blank => native_value("blank"),
        CellValueDto::Number { value } => NativeCellValue {
            number_value: Some(value),
            ..native_value("number")
        },
        CellValueDto::Text { value } => NativeCellValue {
            text_value: Some(value),
            ..native_value("text")
        },
        CellValueDto::Logical { value } => NativeCellValue {
            logical_value: Some(value),
            ..native_value("logical")
        },
        CellValueDto::Error { value } => NativeCellValue {
            error_value: Some(value),
            ..native_value("error")
        },
        CellValueDto::Unsupported => native_value("unsupported"),
    }
}

fn native_value(kind: &str) -> NativeCellValue {
    NativeCellValue {
        kind: kind.to_owned(),
        number_value: None,
        text_value: None,
        logical_value: None,
        error_value: None,
    }
}

fn cell_reference(value: CellReferenceDto) -> NativeCellReference {
    NativeCellReference {
        sheet_id: value.sheet_id,
        sheet_name: value.sheet_name,
        address: value.address,
    }
}
