use std::sync::Arc;

use cellrune_binding_support::SharedWorkbookSession;
use cellrune_interop::{
    CalculationOptionsDto, CancellationToken, EditBatchV2Dto, EditWriteOptionsDto,
    ReadLimitOptionsDto, RecalculationModeDto, WorkbookSession, WriteOptionsDto,
};
use napi::bindgen_prelude::Buffer;
use napi::{Env, Task};

use crate::conversion::{
    NativeCalculationDelta, NativeCalculationReport, NativeCapabilityPage, NativeEditWriteReport,
    NativeWriteReport,
};
use crate::error::{catch_panic, napi_error};
use crate::workbook::NativeWorkbook;

pub struct OpenPathTask {
    path: String,
    limits: ReadLimitOptionsDto,
}

impl OpenPathTask {
    pub const fn new(path: String, limits: ReadLimitOptionsDto) -> Self {
        Self { path, limits }
    }
}

impl Task for OpenPathTask {
    type Output = WorkbookSession;
    type JsValue = NativeWorkbook;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            WorkbookSession::open_path_with_limits(&self.path, self.limits).map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(NativeWorkbook::new(output))
    }
}

pub struct OpenBytesTask {
    bytes: Vec<u8>,
    limits: ReadLimitOptionsDto,
}

impl OpenBytesTask {
    pub const fn new(bytes: Vec<u8>, limits: ReadLimitOptionsDto) -> Self {
        Self { bytes, limits }
    }
}

impl Task for OpenBytesTask {
    type Output = WorkbookSession;
    type JsValue = NativeWorkbook;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            WorkbookSession::open_bytes_with_limits(&self.bytes, self.limits).map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(NativeWorkbook::new(output))
    }
}

pub struct CalculateTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) options: CalculationOptionsDto,
    pub(crate) cancellation: CancellationToken,
}

impl Task for CalculateTask {
    type Output = cellrune_interop::CalculationReportDto;
    type JsValue = NativeCalculationReport;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            let prepared = {
                self.session
                    .lock()
                    .map_err(napi_error)?
                    .prepare_recalculation_cancellable(
                        RecalculationModeDto::Auto,
                        self.options,
                        self.cancellation.clone(),
                    )
                    .map_err(napi_error)?
            };
            let request_id = prepared.request_id();
            let completed = match prepared.run() {
                Ok(completed) => completed,
                Err(error) => {
                    match self.session.lock() {
                        Ok(mut session) => session.abandon_recalculation(request_id),
                        Err(lifecycle_error) => return Err(napi_error(lifecycle_error)),
                    }
                    return Err(napi_error(error));
                }
            };
            let mut session = self.session.lock().map_err(napi_error)?;
            session
                .install_recalculation(completed)
                .map_err(napi_error)?;
            session.calculation_report().map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::calculation_report(output))
    }
}

pub struct RecalculateTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) mode: RecalculationModeDto,
    pub(crate) options: CalculationOptionsDto,
    pub(crate) cancellation: CancellationToken,
}

pub struct PreviewTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) expected_revision: u64,
    pub(crate) batch: EditBatchV2Dto,
    pub(crate) mode: RecalculationModeDto,
    pub(crate) options: CalculationOptionsDto,
    pub(crate) cancellation: CancellationToken,
}

impl Task for PreviewTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            let operation = self
                .session
                .cancellable_operation_with(self.cancellation.clone())
                .map_err(napi_error)?;
            let cancellation = operation.token().clone();
            let prepared = {
                self.session
                    .lock()
                    .map_err(napi_error)?
                    .prepare_preview_changes_cancellable(
                        self.expected_revision,
                        self.batch.clone(),
                        self.mode,
                        self.options,
                        &cancellation,
                    )
                    .map_err(napi_error)?
            };
            let request_id = prepared.request_id();
            let completed = match prepared.run() {
                Ok(completed) => completed,
                Err(error) => {
                    match self.session.lock() {
                        Ok(mut session) => session.abandon_recalculation(request_id),
                        Err(lifecycle_error) => return Err(napi_error(lifecycle_error)),
                    }
                    return Err(napi_error(error));
                }
            };
            let serialized = match crate::preview_json::serialize(completed.summary()) {
                Ok(serialized) => serialized,
                Err(error) => {
                    match self.session.lock() {
                        Ok(mut session) => session.abandon_recalculation(request_id),
                        Err(lifecycle_error) => return Err(napi_error(lifecycle_error)),
                    }
                    return Err(error);
                }
            };
            self.session
                .lock()
                .map_err(napi_error)?
                .publish_preview(completed)
                .map_err(napi_error)?;
            drop(operation);
            Ok(serialized)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }
}

impl Task for RecalculateTask {
    type Output = cellrune_interop::CalculationDeltaDto;
    type JsValue = NativeCalculationDelta;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            let prepared = {
                self.session
                    .lock()
                    .map_err(napi_error)?
                    .prepare_recalculation_cancellable(
                        self.mode,
                        self.options,
                        self.cancellation.clone(),
                    )
                    .map_err(napi_error)?
            };
            let request_id = prepared.request_id();
            let completed = match prepared.run() {
                Ok(completed) => completed,
                Err(error) => {
                    match self.session.lock() {
                        Ok(mut session) => session.abandon_recalculation(request_id),
                        Err(lifecycle_error) => return Err(napi_error(lifecycle_error)),
                    }
                    return Err(napi_error(error));
                }
            };
            self.session
                .lock()
                .map_err(napi_error)?
                .install_recalculation(completed)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::calculation_delta(output))
    }
}

pub struct BytesTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) options: WriteOptionsDto,
}

impl Task for BytesTask {
    type Output = Vec<u8>;
    type JsValue = Buffer;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            self.session
                .lock()
                .map_err(napi_error)?
                .save_bytes(self.options)
                .map(|(bytes, _)| bytes)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into())
    }
}

pub struct SavePathTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) path: String,
    pub(crate) options: WriteOptionsDto,
}

impl Task for SavePathTask {
    type Output = cellrune_interop::WriteReportDto;
    type JsValue = NativeWriteReport;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            self.session
                .lock()
                .map_err(napi_error)?
                .save_path(&self.path, self.options)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::write_report(output))
    }
}

pub struct EditsBytesTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
}

impl Task for EditsBytesTask {
    type Output = Vec<u8>;
    type JsValue = Buffer;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            self.session
                .lock()
                .map_err(napi_error)?
                .save_edits_bytes()
                .map(|(bytes, _)| bytes)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into())
    }
}

pub struct SaveEditsPathTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) path: String,
    pub(crate) options: EditWriteOptionsDto,
}

impl Task for SaveEditsPathTask {
    type Output = cellrune_interop::EditWriteReportDto;
    type JsValue = NativeEditWriteReport;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            self.session
                .lock()
                .map_err(napi_error)?
                .save_edits_path(&self.path, self.options)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::edit_write_report(output))
    }
}

pub struct CapabilityTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) options: CalculationOptionsDto,
    pub(crate) offset: u64,
    pub(crate) limit: u32,
}

impl Task for CapabilityTask {
    type Output = cellrune_interop::CapabilityPageDto;
    type JsValue = NativeCapabilityPage;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            self.session
                .lock()
                .map_err(napi_error)?
                .capabilities(self.options, self.offset, self.limit)
                .map_err(napi_error)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::capability_page(output))
    }
}
