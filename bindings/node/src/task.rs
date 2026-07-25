use std::sync::Arc;

use cellrune_binding_support::SharedWorkbookSession;
use cellrune_interop::{
    CalculationOptionsDto, RecalculationModeDto, WorkbookSession, WriteOptionsDto,
};
use napi::bindgen_prelude::Buffer;
use napi::{Env, Task};

use crate::conversion::{NativeCalculationDelta, NativeCalculationReport, NativeWriteReport};
use crate::error::napi_error;
use crate::workbook::NativeWorkbook;

pub struct OpenPathTask {
    path: String,
}

impl OpenPathTask {
    pub const fn new(path: String) -> Self {
        Self { path }
    }
}

impl Task for OpenPathTask {
    type Output = WorkbookSession;
    type JsValue = NativeWorkbook;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        WorkbookSession::open_path(&self.path).map_err(napi_error)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(NativeWorkbook::new(output))
    }
}

pub struct OpenBytesTask {
    bytes: Vec<u8>,
}

impl OpenBytesTask {
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl Task for OpenBytesTask {
    type Output = WorkbookSession;
    type JsValue = NativeWorkbook;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        WorkbookSession::open_bytes(&self.bytes).map_err(napi_error)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(NativeWorkbook::new(output))
    }
}

pub struct CalculateTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) options: CalculationOptionsDto,
}

impl Task for CalculateTask {
    type Output = cellrune_interop::CalculationReportDto;
    type JsValue = NativeCalculationReport;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let prepared = {
            self.session
                .lock()
                .map_err(napi_error)?
                .prepare_recalculation(RecalculationModeDto::Auto, self.options)
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
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::calculation_report(output))
    }
}

pub struct RecalculateTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) mode: RecalculationModeDto,
    pub(crate) options: CalculationOptionsDto,
}

impl Task for RecalculateTask {
    type Output = cellrune_interop::CalculationDeltaDto;
    type JsValue = NativeCalculationDelta;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let prepared = {
            self.session
                .lock()
                .map_err(napi_error)?
                .prepare_recalculation(self.mode, self.options)
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
        self.session
            .lock()
            .map_err(napi_error)?
            .save_bytes(self.options)
            .map(|(bytes, _)| bytes)
            .map_err(napi_error)
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
        self.session
            .lock()
            .map_err(napi_error)?
            .save_path(&self.path, self.options)
            .map_err(napi_error)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(crate::conversion::write_report(output))
    }
}
