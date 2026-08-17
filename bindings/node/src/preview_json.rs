//! Lossless JSON transport for preview DTOs containing unsigned 64-bit identities.

use cellrune_interop::InteropError;
use serde::Serialize;
use serde_json::Value;

use crate::error::napi_error;

const U64_IDENTITY_FIELDS: &[&str] = &[
    "preview_id",
    "base_revision",
    "result_revision",
    "installed_calculation_revision",
    "cursor",
];

pub(crate) fn serialize<T: Serialize>(value: &T) -> napi::Result<String> {
    serialize_json(value).map_err(|_| napi_error(InteropError::serialization()))
}

fn serialize_json<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let mut value = serde_json::to_value(value)?;
    stringify_u64_identities(&mut value);
    serde_json::to_string(&value)
}

fn stringify_u64_identities(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                stringify_u64_identities(value);
            }
        }
        Value::Object(entries) => {
            for (key, value) in entries {
                if U64_IDENTITY_FIELDS.contains(&key.as_str())
                    && let Some(identity) = value.as_u64()
                {
                    *value = Value::String(identity.to_string());
                } else {
                    stringify_u64_identities(value);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    #[derive(Serialize)]
    struct NestedIdentity<'a> {
        preview_id: u64,
        detail: &'a str,
        nested: Revision,
    }

    #[derive(Serialize)]
    struct Revision {
        result_revision: u64,
        count: u64,
    }

    #[test]
    fn identities_are_stringified_without_rewriting_string_contents() {
        let serialized = super::serialize_json(&NestedIdentity {
            preview_id: u64::MAX,
            detail: r#"embedded \"base_revision\":123 remains text"#,
            nested: Revision {
                result_revision: u64::MAX - 1,
                count: u64::MAX,
            },
        })
        .expect("preview JSON must serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("preview JSON must remain valid");

        assert_eq!(parsed["preview_id"], u64::MAX.to_string());
        assert_eq!(
            parsed["nested"]["result_revision"],
            (u64::MAX - 1).to_string()
        );
        assert_eq!(parsed["nested"]["count"], serde_json::json!(u64::MAX));
        assert_eq!(
            parsed["detail"],
            r#"embedded \"base_revision\":123 remains text"#
        );
    }
}
