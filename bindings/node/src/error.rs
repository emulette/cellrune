use cellrune_interop::InteropError;
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
