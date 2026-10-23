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

#[cfg(test)]
mod tests {
    use super::SERIALIZATION_FALLBACK;

    #[test]
    fn the_serialization_fallback_code_is_documented() {
        let payload: serde_json::Value =
            serde_json::from_str(SERIALIZATION_FALLBACK).expect("fallback payload is JSON");
        let code = payload["code"]
            .as_str()
            .expect("fallback payload has a code");
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ERROR_CODES.md");
        let reference = std::fs::read_to_string(path).expect("docs/ERROR_CODES.md is readable");
        let section = reference
            .split_once("## Node.js binding")
            .map(|(_, section)| section)
            .expect("the reference has a Node.js section");
        assert!(
            section.contains(&format!("| `{code}` |")),
            "{code} is undocumented"
        );
    }
}
