use std::sync::Arc;

use cellrune_binding_support::{SharedWorkbookSession, WorkbookSessionGuard};
use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationOptionsDto, DefinedNameInspectionRequestDto, EditBatchDto,
    EditBatchV2Dto, FinancialSolverSemanticsDto, InteropError, PreviewCursorDto, RangeRequestDto,
    RecalculationModeDto, TransactionDetailSectionDto, WorkbookSession, WritableCellValueDto,
    WriteOptionsDto,
};
use napi::bindgen_prelude::AsyncTask;
use napi_derive::napi;
use serde::Deserialize;

use crate::conversion::{
    NativeCalculationDeltaPage, NativeEditReceipt, NativeEditReceiptV2, NativeFunctionUsageReport,
    NativeRangePage, NativeWorkbookSummary,
};
use crate::defined_name::NativeDefinedNameInspection;
use crate::error::napi_error;
use crate::task::{BytesTask, CalculateTask, PreviewTask, RecalculateTask, SavePathTask};

#[napi]
pub struct NativeWorkbook {
    session: Arc<SharedWorkbookSession>,
}

#[napi]
impl NativeWorkbook {
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        self.session.is_closed()
    }

    #[napi]
    pub fn close(&self) {
        self.session.close();
    }

    #[napi]
    pub fn summary(&self) -> napi::Result<NativeWorkbookSummary> {
        Ok(crate::conversion::workbook_summary(self.lock()?.summary()))
    }

    #[napi]
    pub fn read_range(
        &self,
        sheet: String,
        start: String,
        end: String,
        offset: f64,
        limit: f64,
    ) -> napi::Result<NativeRangePage> {
        let offset = offset_from_number(offset)?;
        let limit = limit_from_number(limit)?;
        self.lock()?
            .read_range(&RangeRequestDto {
                sheet,
                start,
                end,
                offset,
                limit,
            })
            .map(crate::conversion::range_page)
            .map_err(napi_error)
    }

    #[napi]
    pub fn inspect_defined_name(
        &self,
        name: String,
        current_sheet: Option<String>,
    ) -> napi::Result<NativeDefinedNameInspection> {
        self.lock()?
            .inspect_defined_name(&DefinedNameInspectionRequestDto {
                name,
                current_sheet,
            })
            .map(crate::defined_name::defined_name_inspection)
            .map_err(napi_error)
    }

    #[napi]
    pub fn function_usage(&self) -> napi::Result<NativeFunctionUsageReport> {
        Ok(crate::conversion::function_usage(
            self.lock()?.function_usage(),
        ))
    }

    #[napi(ts_return_type = "Promise<NativeCalculationReport>")]
    pub fn calculate(
        &self,
        today_serial: Option<f64>,
        now_serial: Option<f64>,
        arithmetic_semantics: Option<String>,
        financial_solver_semantics: Option<String>,
    ) -> napi::Result<AsyncTask<CalculateTask>> {
        Ok(AsyncTask::new(CalculateTask {
            session: Arc::clone(&self.session),
            options: calculation_options(
                today_serial,
                now_serial,
                arithmetic_semantics.as_deref(),
                financial_solver_semantics.as_deref(),
            )?,
        }))
    }

    #[napi(ts_return_type = "Promise<NativeCalculationDelta>")]
    pub fn recalculate(
        &self,
        mode: String,
        today_serial: Option<f64>,
        now_serial: Option<f64>,
        arithmetic_semantics: Option<String>,
        financial_solver_semantics: Option<String>,
    ) -> napi::Result<AsyncTask<RecalculateTask>> {
        Ok(AsyncTask::new(RecalculateTask {
            session: Arc::clone(&self.session),
            mode: recalculation_mode(&mode)?,
            options: calculation_options(
                today_serial,
                now_serial,
                arithmetic_semantics.as_deref(),
                financial_solver_semantics.as_deref(),
            )?,
        }))
    }

    #[napi(ts_return_type = "Promise<string>")]
    pub fn preview_changes(
        &self,
        expected_revision: String,
        batch_json: String,
        options_json: String,
    ) -> napi::Result<AsyncTask<PreviewTask>> {
        let expected_revision = parse_u64(&expected_revision)?;
        let batch = serde_json::from_str::<EditBatchV2Dto>(&batch_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        let options = serde_json::from_str::<PreviewOptionsInput>(&options_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        Ok(AsyncTask::new(PreviewTask {
            session: Arc::clone(&self.session),
            expected_revision,
            batch,
            mode: options.mode,
            options: options.calculation_options(),
        }))
    }

    #[napi(ts_return_type = "string")]
    pub fn preview_changes_page(
        &self,
        preview_id: String,
        section: String,
        cursor_json: Option<String>,
        limit: f64,
    ) -> napi::Result<String> {
        let preview_id = parse_u64(&preview_id)?;
        let cursor = cursor_json
            .map(|value| {
                serde_json::from_str::<PreviewCursorDto>(&value)
                    .map_err(|_| napi_error(InteropError::preview_cursor_invalid()))
            })
            .transpose()?;
        let page = self
            .lock()?
            .preview_changes_page(
                preview_id,
                preview_section(&section)?,
                cursor,
                limit_from_number(limit)?,
            )
            .map_err(napi_error)?;
        crate::preview_json::serialize(&page)
    }

    #[napi(ts_return_type = "string")]
    pub fn commit_preview(&self, preview_id: String) -> napi::Result<String> {
        let preview_id = parse_u64(&preview_id)?;
        let receipt = self
            .lock()?
            .commit_preview(preview_id)
            .map_err(napi_error)?;
        crate::preview_json::serialize(&receipt)
    }

    #[napi]
    pub fn discard_preview(&self, preview_id: String) -> napi::Result<()> {
        self.lock()?
            .discard_preview(parse_u64(&preview_id)?)
            .map_err(napi_error)
    }

    #[napi]
    pub fn apply_changes(
        &self,
        expected_revision: String,
        batch_json: String,
    ) -> napi::Result<NativeEditReceipt> {
        let expected_revision = parse_u64(&expected_revision)?;
        let batch = serde_json::from_str::<EditBatchDto>(&batch_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        self.lock()?
            .apply_changes(expected_revision, batch)
            .map(crate::conversion::edit_receipt)
            .map_err(napi_error)
    }

    #[napi]
    pub fn apply_changes_v2(
        &self,
        expected_revision: String,
        batch_json: String,
    ) -> napi::Result<NativeEditReceiptV2> {
        let expected_revision = parse_u64(&expected_revision)?;
        let batch = serde_json::from_str::<EditBatchV2Dto>(&batch_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        self.lock()?
            .apply_changes_v2(expected_revision, batch)
            .map(crate::conversion::edit_receipt_v2)
            .map_err(napi_error)
    }

    #[napi]
    pub fn changes_since(
        &self,
        cursor: String,
        limit: f64,
    ) -> napi::Result<NativeCalculationDeltaPage> {
        let cursor = parse_u64(&cursor)?;
        let limit = limit_from_number(limit)?;
        self.lock()?
            .changes_since(cursor, limit)
            .map(crate::conversion::calculation_delta_page)
            .map_err(napi_error)
    }

    #[napi]
    pub fn cancel_calculation(&self) -> napi::Result<bool> {
        Ok(self.lock()?.cancel_calculation())
    }

    #[napi]
    pub fn calculation_active(&self) -> napi::Result<bool> {
        Ok(self.lock()?.calculation_active())
    }

    #[napi]
    pub fn set_blank(&self, sheet: String, address: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Blank)
    }

    #[napi]
    pub fn set_number(&self, sheet: String, address: String, value: f64) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Number { value })
    }

    #[napi]
    pub fn set_text(&self, sheet: String, address: String, value: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Text { value })
    }

    #[napi]
    pub fn set_logical(&self, sheet: String, address: String, value: bool) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Logical { value })
    }

    #[napi]
    pub fn set_error(&self, sheet: String, address: String, value: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Error { value })
    }

    #[napi]
    pub fn set_formula(
        &self,
        sheet: String,
        address: String,
        formula: String,
        dynamic_range: Option<String>,
    ) -> napi::Result<()> {
        self.lock()?
            .set_formula(&sheet, &address, &formula, dynamic_range.as_deref())
            .map_err(napi_error)
    }

    #[napi]
    pub fn clear_cell(&self, sheet: String, address: String) -> napi::Result<bool> {
        self.lock()?
            .clear_cell(&sheet, &address)
            .map_err(napi_error)
    }

    #[napi]
    pub fn add_sheet(&self, name: String) -> napi::Result<u32> {
        self.lock()?.add_sheet(&name).map_err(napi_error)
    }

    #[napi]
    pub fn rename_sheet(&self, current_name: String, new_name: String) -> napi::Result<()> {
        self.lock()?
            .rename_sheet(&current_name, &new_name)
            .map_err(napi_error)
    }

    #[napi(ts_return_type = "Promise<Buffer>")]
    pub fn to_bytes(&self, invalidate_unavailable: bool) -> AsyncTask<BytesTask> {
        AsyncTask::new(BytesTask {
            session: Arc::clone(&self.session),
            options: WriteOptionsDto {
                invalidate_unavailable,
                replace_existing: false,
            },
        })
    }

    #[napi(ts_return_type = "Promise<NativeWriteReport>")]
    pub fn save_path(
        &self,
        path: String,
        invalidate_unavailable: bool,
        replace_existing: bool,
    ) -> AsyncTask<SavePathTask> {
        AsyncTask::new(SavePathTask {
            session: Arc::clone(&self.session),
            path,
            options: WriteOptionsDto {
                invalidate_unavailable,
                replace_existing,
            },
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewOptionsInput {
    #[serde(default)]
    mode: RecalculationModeDto,
    today_serial: Option<f64>,
    now_serial: Option<f64>,
    #[serde(default)]
    arithmetic_semantics: ArithmeticSemanticsDto,
    #[serde(default)]
    financial_solver_semantics: FinancialSolverSemanticsDto,
}

impl PreviewOptionsInput {
    const fn calculation_options(&self) -> CalculationOptionsDto {
        CalculationOptionsDto {
            today_serial: self.today_serial,
            now_serial: self.now_serial,
            arithmetic_semantics: self.arithmetic_semantics,
            financial_solver_semantics: self.financial_solver_semantics,
        }
    }
}

impl NativeWorkbook {
    pub(crate) fn create() -> Self {
        Self::new(WorkbookSession::create())
    }

    pub(crate) fn new(session: WorkbookSession) -> Self {
        Self {
            session: Arc::new(SharedWorkbookSession::new(session)),
        }
    }

    fn set_value(
        &self,
        sheet: &str,
        address: &str,
        value: WritableCellValueDto,
    ) -> napi::Result<()> {
        self.lock()?
            .set_value(sheet, address, value)
            .map_err(napi_error)
    }

    fn lock(&self) -> napi::Result<WorkbookSessionGuard<'_>> {
        self.session.try_lock().map_err(napi_error)
    }
}

fn offset_from_number(value: f64) -> napi::Result<u64> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > u64::MAX as f64 {
        return Err(napi_error(InteropError::invalid_page_offset()));
    }
    Ok(value as u64)
}

fn limit_from_number(value: f64) -> napi::Result<u32> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > f64::from(u32::MAX) {
        return Err(napi_error(InteropError::invalid_page_limit()));
    }
    Ok(value as u32)
}

fn parse_u64(value: &str) -> napi::Result<u64> {
    value
        .parse::<u64>()
        .map_err(|_| napi_error(InteropError::invalid_revision_or_cursor()))
}

fn recalculation_mode(value: &str) -> napi::Result<RecalculationModeDto> {
    match value {
        "auto" => Ok(RecalculationModeDto::Auto),
        "incremental" => Ok(RecalculationModeDto::Incremental),
        "full" => Ok(RecalculationModeDto::Full),
        _ => Err(napi_error(InteropError::invalid_recalculation_mode())),
    }
}

fn preview_section(value: &str) -> napi::Result<TransactionDetailSectionDto> {
    match value {
        "affected" => Ok(TransactionDetailSectionDto::Affected),
        "evaluated" => Ok(TransactionDetailSectionDto::Evaluated),
        "preview_results" => Ok(TransactionDetailSectionDto::PreviewResults),
        "preview_issues" => Ok(TransactionDetailSectionDto::PreviewIssues),
        "install_results" => Ok(TransactionDetailSectionDto::InstallResults),
        _ => Err(napi_error(InteropError::invalid_preview_section())),
    }
}

fn calculation_options(
    today_serial: Option<f64>,
    now_serial: Option<f64>,
    arithmetic_semantics: Option<&str>,
    financial_solver_semantics: Option<&str>,
) -> napi::Result<CalculationOptionsDto> {
    let arithmetic_semantics = match arithmetic_semantics.unwrap_or("excel_near_zero") {
        "excel_near_zero" => ArithmeticSemanticsDto::ExcelNearZero,
        "ieee_754" => ArithmeticSemanticsDto::Ieee754,
        _ => return Err(napi_error(InteropError::invalid_arithmetic_semantics())),
    };
    let financial_solver_semantics =
        match financial_solver_semantics.unwrap_or("excel_iteration_budget") {
            "excel_iteration_budget" => FinancialSolverSemanticsDto::ExcelIterationBudget,
            "extended_search" => FinancialSolverSemanticsDto::ExtendedSearch,
            _ => {
                return Err(napi_error(
                    InteropError::invalid_financial_solver_semantics(),
                ));
            }
        };
    Ok(CalculationOptionsDto {
        today_serial,
        now_serial,
        arithmetic_semantics,
        financial_solver_semantics,
    })
}
