use std::panic::{AssertUnwindSafe, catch_unwind};

use cellrune_interop::InteropError;
use napi::bindgen_prelude::panic_to_error;
use napi::{Error, Status};

const ERROR_PREFIX: &str = "CELLRUNE_ERROR:";
const SERIALIZATION_FALLBACK: &str = r#"{"kind":"state","code":"interop.error.serialization","message":"failed to serialize CellRune error","details":{"source_code":null,"source_id":null,"detail":null}}"#;

pub(crate) fn napi_error(error: InteropError) -> Error {
    let serialized =
        serde_json::to_string(&error).unwrap_or_else(|_| SERIALIZATION_FALLBACK.to_owned());
    Error::new(
        Status::GenericFailure,
        format!("{ERROR_PREFIX}{serialized}"),
    )
}

/// Gives async task work the panic boundary that `#[napi(catch_unwind)]` gives exported calls.
pub(crate) fn catch_panic<T>(operation: impl FnOnce() -> napi::Result<T>) -> napi::Result<T> {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|payload| Err(panic_to_error(payload)))
}
