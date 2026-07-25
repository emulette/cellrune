use std::any::Any;
use std::sync::Arc;

use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde_json::Value;

pub(crate) fn mcp_schema<T>() -> Arc<JsonObject>
where
    T: JsonSchema + Any,
{
    let source = rmcp::handler::server::common::schema_for_type::<T>();
    let mut schema = source.as_ref().clone();
    strip_nonstandard_integer_formats_from_object(&mut schema);
    Arc::new(schema)
}

fn strip_nonstandard_integer_formats_from_object(object: &mut JsonObject) {
    if object
        .get("format")
        .and_then(Value::as_str)
        .is_some_and(|format| {
            matches!(format, "uint8" | "uint16" | "uint32" | "uint64" | "uint128")
        })
    {
        object.remove("format");
    }
    for value in object.values_mut() {
        strip_nonstandard_integer_formats(value);
    }
}

fn strip_nonstandard_integer_formats(value: &mut Value) {
    match value {
        Value::Object(object) => strip_nonstandard_integer_formats_from_object(object),
        Value::Array(values) => {
            for value in values {
                strip_nonstandard_integer_formats(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;

    use super::*;

    #[derive(JsonSchema)]
    struct NumericSchema {
        count: u64,
        ratio: f64,
    }

    #[test]
    fn strips_only_nonstandard_unsigned_integer_formats() {
        let sample = NumericSchema {
            count: 1,
            ratio: 0.5,
        };
        assert_eq!(sample.count, 1);
        assert_eq!(sample.ratio, 0.5);
        let schema = mcp_schema::<NumericSchema>();
        let count = &schema["properties"]["count"];
        let ratio = &schema["properties"]["ratio"];

        assert!(count.get("format").is_none());
        assert_eq!(count["type"], "integer");
        assert_eq!(count["minimum"], 0);
        assert_eq!(ratio["format"], "double");
    }
}
