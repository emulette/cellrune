mod conversion;
mod defined_name;
mod error;
mod preview_json;
mod sheet_metadata;
mod targeted;
mod task;
mod workbook;

use cellrune_interop::InteropError;
use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi_derive::napi;

use crate::task::{OpenBytesTask, OpenPathTask};
use crate::workbook::NativeWorkbook;

#[napi(catch_unwind)]
pub fn schema_version() -> u32 {
    cellrune_interop::INTEROP_SCHEMA_VERSION
}

#[napi(catch_unwind)]
pub fn function_catalog() -> conversion::NativeFunctionCatalogReport {
    conversion::function_catalog(cellrune_interop::function_catalog())
}

/// Returns the text a cell shows for a display-format request serialized by the JavaScript wrapper.
#[napi(catch_unwind)]
pub fn format_display_text(request_json: String) -> napi::Result<String> {
    let request = serde_json::from_str(&request_json).map_err(|error| {
        error::napi_error(InteropError::invalid_display_format_request(
            error.to_string(),
        ))
    })?;
    cellrune_interop::format_display_text(request)
        .map(|result| result.text)
        .map_err(error::napi_error)
}

#[napi(catch_unwind)]
pub fn create_workbook() -> NativeWorkbook {
    NativeWorkbook::create()
}

#[napi(ts_return_type = "Promise<NativeWorkbook>", catch_unwind)]
pub fn open_workbook_path(
    path: String,
    limits_json: Option<String>,
) -> napi::Result<AsyncTask<OpenPathTask>> {
    let limits = workbook::limit_options(limits_json.as_deref())?;
    Ok(AsyncTask::new(OpenPathTask::new(path, limits)))
}

#[napi(catch_unwind)]
pub fn open_workbook_csv(
    bytes: Buffer,
    sheet_name: Option<String>,
    delimiter: Option<String>,
) -> napi::Result<NativeWorkbook> {
    let options = cellrune_interop::CsvReadOptionsDto {
        sheet_name,
        delimiter,
    };
    cellrune_interop::WorkbookSession::open_csv_bytes(&bytes, &options)
        .map(NativeWorkbook::new)
        .map_err(error::napi_error)
}

#[napi(ts_return_type = "Promise<NativeWorkbook>", catch_unwind)]
pub fn open_workbook_bytes(
    bytes: Buffer,
    limits_json: Option<String>,
) -> napi::Result<AsyncTask<OpenBytesTask>> {
    let limits = workbook::limit_options(limits_json.as_deref())?;
    Ok(AsyncTask::new(OpenBytesTask::new(bytes.into(), limits)))
}
