mod conversion;
mod defined_name;
mod error;
mod task;
mod workbook;

use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi_derive::napi;

use crate::task::{OpenBytesTask, OpenPathTask};
use crate::workbook::NativeWorkbook;

#[napi]
pub fn schema_version() -> u32 {
    cellrune_interop::INTEROP_SCHEMA_VERSION
}

#[napi]
pub fn create_workbook() -> NativeWorkbook {
    NativeWorkbook::create()
}

#[napi(ts_return_type = "Promise<NativeWorkbook>")]
pub fn open_workbook_path(path: String) -> AsyncTask<OpenPathTask> {
    AsyncTask::new(OpenPathTask::new(path))
}

#[napi(ts_return_type = "Promise<NativeWorkbook>")]
pub fn open_workbook_bytes(bytes: Buffer) -> AsyncTask<OpenBytesTask> {
    AsyncTask::new(OpenBytesTask::new(bytes.into()))
}
