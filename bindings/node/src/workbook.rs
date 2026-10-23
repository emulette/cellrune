use std::sync::Arc;

use cellrune_binding_support::{SharedWorkbookSession, WorkbookSessionGuard};
use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationLimitOptionsDto, CalculationOptionsDto,
    DefinedNameInspectionRequestDto, EditBatchDto, EditBatchV2Dto, EditWriteOptionsDto,
    FinancialSolverSemanticsDto, InteropError, PreviewCursorDto, RangeRequestDto,
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
use crate::task::{
    BytesTask, CalculateTask, CapabilityTask, EditsBytesTask, PreviewTask, RecalculateTask,
    SaveEditsPathTask, SavePathTask,
};

#[napi]
pub struct NativeWorkbook {
    session: Arc<SharedWorkbookSession>,
}

#[napi]
impl NativeWorkbook {
    #[napi(ts_return_type = "Promise<string>", catch_unwind)]
    pub fn calculate_targets(
        &self,
        request_json: String,
    ) -> napi::Result<AsyncTask<crate::targeted::TargetCalculationTask>> {
        let request =
            serde_json::from_str::<cellrune_interop::TargetCalculationRequestDto>(&request_json)
                .map_err(|error| {
                    napi_error(InteropError::invalid_target_payload(error.to_string()))
                })?;
        Ok(AsyncTask::new(crate::targeted::TargetCalculationTask {
            session: Arc::clone(&self.session),
            request,
        }))
    }

    #[napi(getter, catch_unwind)]
    pub fn closed(&self) -> bool {
        self.session.is_closed()
    }

    #[napi(catch_unwind)]
    pub fn close(&self) {
        self.session.close();
    }

    #[napi(catch_unwind)]
    pub fn summary(&self) -> napi::Result<NativeWorkbookSummary> {
        Ok(crate::conversion::workbook_summary(self.lock()?.summary()))
    }

    #[napi(catch_unwind)]
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

    #[napi(ts_return_type = "Promise<NativeCapabilityPage>", catch_unwind)]
    pub fn scan_capabilities(
        &self,
        offset: f64,
        limit: f64,
        today_serial: Option<f64>,
        now_serial: Option<f64>,
    ) -> napi::Result<AsyncTask<CapabilityTask>> {
        Ok(AsyncTask::new(CapabilityTask {
            session: Arc::clone(&self.session),
            options: CalculationOptionsDto {
                today_serial,
                now_serial,
                ..CalculationOptionsDto::default()
            },
            offset: offset_from_number(offset)?,
            limit: limit_from_number(limit)?,
        }))
    }

    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
    pub fn function_usage(&self) -> napi::Result<NativeFunctionUsageReport> {
        Ok(crate::conversion::function_usage(
            self.lock()?.function_usage(),
        ))
    }

    #[napi(ts_return_type = "Promise<NativeCalculationReport>", catch_unwind)]
    pub fn calculate(
        &self,
        options: Option<NativeCalculationOptions>,
        limits_json: Option<String>,
    ) -> napi::Result<AsyncTask<CalculateTask>> {
        Ok(AsyncTask::new(CalculateTask {
            session: Arc::clone(&self.session),
            options: calculation_options(options, limits_json.as_deref())?,
        }))
    }

    #[napi(ts_return_type = "Promise<NativeCalculationDelta>", catch_unwind)]
    pub fn recalculate(
        &self,
        mode: String,
        options: Option<NativeCalculationOptions>,
        limits_json: Option<String>,
    ) -> napi::Result<AsyncTask<RecalculateTask>> {
        Ok(AsyncTask::new(RecalculateTask {
            session: Arc::clone(&self.session),
            mode: recalculation_mode(&mode)?,
            options: calculation_options(options, limits_json.as_deref())?,
        }))
    }

    #[napi(ts_return_type = "Promise<string>", catch_unwind)]
    pub fn preview_changes(
        &self,
        expected_revision: String,
        batch_json: String,
        options_json: String,
        limits_json: Option<String>,
    ) -> napi::Result<AsyncTask<PreviewTask>> {
        let expected_revision = parse_u64(&expected_revision)?;
        let batch = serde_json::from_str::<EditBatchV2Dto>(&batch_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        let options = serde_json::from_str::<PreviewOptionsInput>(&options_json)
            .map_err(|error| napi_error(InteropError::invalid_change_payload(error.to_string())))?;
        let calculation_limits =
            limit_options::<CalculationLimitOptionsDto>(limits_json.as_deref())?;
        Ok(AsyncTask::new(PreviewTask {
            session: Arc::clone(&self.session),
            expected_revision,
            batch,
            mode: options.mode,
            options: options.calculation_options(calculation_limits),
        }))
    }

    #[napi(ts_return_type = "string", catch_unwind)]
    pub fn preview_changes_page(
        &self,
        preview_id: String,
        section: String,
        cursor_json: Option<String>,
        limit: f64,
    ) -> napi::Result<String> {
        let preview_id = parse_u64(&preview_id)?;
        let cursor = cursor_json
            .map(|value| preview_cursor(&value))
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

    #[napi(ts_return_type = "string", catch_unwind)]
    pub fn commit_preview(&self, preview_id: String) -> napi::Result<String> {
        let preview_id = parse_u64(&preview_id)?;
        let receipt = self
            .lock()?
            .commit_preview(preview_id)
            .map_err(napi_error)?;
        crate::preview_json::serialize(&receipt)
    }

    #[napi(catch_unwind)]
    pub fn discard_preview(&self, preview_id: String) -> napi::Result<()> {
        self.lock()?
            .discard_preview(parse_u64(&preview_id)?)
            .map_err(napi_error)
    }

    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
    pub fn cancel_calculation(&self) -> napi::Result<bool> {
        Ok(self.lock()?.cancel_calculation())
    }

    #[napi(catch_unwind)]
    pub fn calculation_active(&self) -> napi::Result<bool> {
        Ok(self.lock()?.calculation_active())
    }

    #[napi(catch_unwind)]
    pub fn set_blank(&self, sheet: String, address: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Blank)
    }

    #[napi(catch_unwind)]
    pub fn set_number(&self, sheet: String, address: String, value: f64) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Number { value })
    }

    #[napi(catch_unwind)]
    pub fn set_text(&self, sheet: String, address: String, value: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Text { value })
    }

    #[napi(catch_unwind)]
    pub fn set_logical(&self, sheet: String, address: String, value: bool) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Logical { value })
    }

    #[napi(catch_unwind)]
    pub fn set_error(&self, sheet: String, address: String, value: String) -> napi::Result<()> {
        self.set_value(&sheet, &address, WritableCellValueDto::Error { value })
    }

    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
    pub fn clear_cell(&self, sheet: String, address: String) -> napi::Result<bool> {
        self.lock()?
            .clear_cell(&sheet, &address)
            .map_err(napi_error)
    }

    #[napi(catch_unwind)]
    pub fn add_sheet(&self, name: String) -> napi::Result<u32> {
        self.lock()?.add_sheet(&name).map_err(napi_error)
    }

    #[napi(catch_unwind)]
    pub fn rename_sheet(&self, current_name: String, new_name: String) -> napi::Result<()> {
        self.lock()?
            .rename_sheet(&current_name, &new_name)
            .map_err(napi_error)
    }

    #[napi(catch_unwind)]
    pub fn delete_sheet(&self, name: String) -> napi::Result<()> {
        self.lock()?.delete_sheet(&name).map_err(napi_error)
    }

    #[napi(ts_return_type = "Promise<Buffer>", catch_unwind)]
    pub fn to_bytes(&self, invalidate_unavailable: bool) -> AsyncTask<BytesTask> {
        AsyncTask::new(BytesTask {
            session: Arc::clone(&self.session),
            options: WriteOptionsDto {
                invalidate_unavailable,
                replace_existing: false,
            },
        })
    }

    #[napi(ts_return_type = "Promise<NativeWriteReport>", catch_unwind)]
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

    #[napi(ts_return_type = "Promise<Buffer>", catch_unwind)]
    pub fn edits_to_bytes(&self) -> AsyncTask<EditsBytesTask> {
        AsyncTask::new(EditsBytesTask {
            session: Arc::clone(&self.session),
        })
    }

    #[napi(ts_return_type = "Promise<NativeEditWriteReport>", catch_unwind)]
    pub fn save_edits_path(
        &self,
        path: String,
        replace_existing: bool,
    ) -> AsyncTask<SaveEditsPathTask> {
        AsyncTask::new(SaveEditsPathTask {
            session: Arc::clone(&self.session),
            path,
            options: EditWriteOptionsDto { replace_existing },
        })
    }
}

/// Preview cursor as the JavaScript wrapper sends it. The preview id travels as a decimal string,
/// as Node preview JSON emits it, because a JavaScript number cannot hold every `u64`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewCursorInput {
    preview_id: String,
    token: String,
}

fn preview_cursor(value: &str) -> napi::Result<PreviewCursorDto> {
    let invalid = || napi_error(InteropError::preview_cursor_invalid());
    let input = serde_json::from_str::<PreviewCursorInput>(value).map_err(|_| invalid())?;
    let preview_id = input.preview_id.parse::<u64>().map_err(|_| invalid())?;
    Ok(PreviewCursorDto {
        preview_id,
        token: input.token,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewOptionsInput {
    #[serde(default)]
    mode: RecalculationModeDto,
    today_serial: Option<f64>,
    now_serial: Option<f64>,
    random_seed: Option<u32>,
    #[serde(default)]
    arithmetic_semantics: ArithmeticSemanticsDto,
    #[serde(default)]
    financial_solver_semantics: FinancialSolverSemanticsDto,
}

impl PreviewOptionsInput {
    const fn calculation_options(
        &self,
        calculation_limits: CalculationLimitOptionsDto,
    ) -> CalculationOptionsDto {
        CalculationOptionsDto {
            today_serial: self.today_serial,
            now_serial: self.now_serial,
            random_seed: self.random_seed,
            arithmetic_semantics: self.arithmetic_semantics,
            financial_solver_semantics: self.financial_solver_semantics,
            calculation_limits,
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

/// Deterministic inputs and compatibility policies for `calculate` and `recalculate`, passed as
/// JavaScript values so serials keep every bit; `None` keeps every default. Limits travel as their
/// own object so their errors keep the limit codes, and the policies stay strings so an unknown
/// one keeps its own code.
#[napi(object)]
#[derive(Default)]
pub struct NativeCalculationOptions {
    pub today_serial: Option<f64>,
    pub now_serial: Option<f64>,
    pub random_seed: Option<u32>,
    pub arithmetic_semantics: Option<String>,
    pub financial_solver_semantics: Option<String>,
}

fn calculation_options(
    options: Option<NativeCalculationOptions>,
    limits_json: Option<&str>,
) -> napi::Result<CalculationOptionsDto> {
    let calculation_limits = limit_options::<CalculationLimitOptionsDto>(limits_json)?;
    let NativeCalculationOptions {
        today_serial,
        now_serial,
        random_seed,
        arithmetic_semantics,
        financial_solver_semantics,
    } = options.unwrap_or_default();
    let arithmetic_semantics = match arithmetic_semantics.as_deref().unwrap_or("excel_near_zero") {
        "excel_near_zero" => ArithmeticSemanticsDto::ExcelNearZero,
        "ieee_754" => ArithmeticSemanticsDto::Ieee754,
        _ => return Err(napi_error(InteropError::invalid_arithmetic_semantics())),
    };
    let financial_solver_semantics = match financial_solver_semantics
        .as_deref()
        .unwrap_or("excel_iteration_budget")
    {
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
        random_seed,
        arithmetic_semantics,
        financial_solver_semantics,
        calculation_limits,
    })
}

/// Reads a limit object serialized by the JavaScript wrapper; `None` keeps every default.
pub(crate) fn limit_options<T: serde::de::DeserializeOwned + Default>(
    json: Option<&str>,
) -> napi::Result<T> {
    json.map_or_else(
        || Ok(T::default()),
        |json| {
            serde_json::from_str(json)
                .map_err(|error| napi_error(InteropError::invalid_limit_payload(error.to_string())))
        },
    )
}
