//! Reads payload values that must be JSON objects.
//!
//! Serde's derived `Deserialize` also fills a struct from a JSON array by field position and reads
//! an internally tagged enum's tag from an array's first element. These functions accept only an
//! object and hand it to the derived reading unchanged, so an object keeps the same errors.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};

/// Reads `T` only from a JSON object.
pub fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(ObjectDeserializer(deserializer))
}

/// Reads a sequence whose every item is read only from a JSON object.
pub fn deserialize_objects<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Vec::<Object<T>>::deserialize(deserializer)
        .map(|items| items.into_iter().map(|Object(item)| item).collect())
}

/// Reads `null` as `None` and any other value only from a JSON object.
pub fn deserialize_optional_object<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Object<T>>::deserialize(deserializer).map(|value| value.map(|Object(value)| value))
}

/// A `T` read only from a JSON object, for use inside other containers.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_object(deserializer).map(Self)
    }
}

/// Passes each request to the wrapped deserializer with a visitor that accepts only a map.
struct ObjectDeserializer<D>(D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for ObjectDeserializer<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_any(MapVisitor(visitor))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_struct(name, fields, MapVisitor(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option
        unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
}

/// Forwards a map to the wrapped visitor; any other value fails as an invalid type for what the
/// wrapped visitor expects.
struct MapVisitor<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for MapVisitor<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.0.visit_map(map)
    }
}

#[cfg(test)]
mod tests {
    use super::{deserialize_object, deserialize_objects, deserialize_optional_object};
    use crate::{CalculationTargetDto, WritableCellValueDto};
    use serde::Deserialize;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    /// Reads `T` through [`deserialize_object`] from JSON text, as `serde_json::from_str` would.
    fn object_from_str<T: DeserializeOwned>(json: &str) -> serde_json::Result<T> {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let value = deserialize_object(&mut deserializer)?;
        deserializer.end()?;
        Ok(value)
    }

    /// Asserts that the helper reads `json` exactly as the derived reading does, error text and
    /// position included, both from text and from a parsed value.
    fn assert_same_reading<T: DeserializeOwned + PartialEq + std::fmt::Debug>(json: &str) {
        let derived = serde_json::from_str::<T>(json).map_err(|error| error.to_string());
        assert_eq!(
            object_from_str::<T>(json).map_err(|error| error.to_string()),
            derived
        );
        let value = serde_json::from_str::<Value>(json).expect("test JSON parses");
        assert_eq!(
            deserialize_object::<_, T>(value.clone()).map_err(|error| error.to_string()),
            serde_json::from_value::<T>(value).map_err(|error| error.to_string())
        );
    }

    #[test]
    fn only_an_object_is_read() {
        // The derived readings also accept an array.
        serde_json::from_str::<CalculationTargetDto>(r#"["Sheet1", "A1"]"#)
            .expect("a struct reads an array by position");
        serde_json::from_str::<WritableCellValueDto>(r#"["number", 1.0]"#)
            .expect("an internally tagged enum reads its tag from an array");
        for (json, unexpected) in [
            (r#"["Sheet1", "A1"]"#, "sequence"),
            ("[]", "sequence"),
            (r#""Sheet1""#, r#"string "Sheet1""#),
            ("1", "integer `1`"),
            ("null", "null"),
        ] {
            let error = object_from_str::<CalculationTargetDto>(json)
                .expect_err("only an object is a target")
                .to_string();
            assert!(
                error.starts_with(&format!(
                    "invalid type: {unexpected}, expected struct CalculationTargetDto at line 1 "
                )),
                "{error}"
            );
            let value = serde_json::from_str::<Value>(json).expect("test JSON parses");
            assert_eq!(
                deserialize_object::<_, WritableCellValueDto>(value)
                    .expect_err("only an object is a value")
                    .to_string(),
                format!(
                    "invalid type: {unexpected}, expected internally tagged enum WritableCellValueDto"
                ),
            );
        }
    }

    #[test]
    fn an_object_reads_as_the_derived_reading_does() {
        for json in [
            r#"{"sheet": "Sheet1", "start": "A1"}"#,
            r#"{"sheet": "Sheet1", "start": "A1", "end": "B2"}"#,
            r#"{"sheet": "Sheet1"}"#,
            r#"{"sheet": "Sheet1", "start": "A1", "unknown": 1}"#,
            r#"{"sheet": "Sheet1", "start": 1}"#,
            r#"{"sheet": "Sheet1", "sheet": "Sheet2", "start": "A1"}"#,
            "{}",
        ] {
            assert_same_reading::<CalculationTargetDto>(json);
        }
        for json in [
            r#"{"kind": "number", "value": 1.5}"#,
            r#"{"kind": "blank"}"#,
            r#"{"kind": "number"}"#,
            r#"{"kind": "number", "value": 1.5, "unknown": 1}"#,
            r#"{"kind": "number", "value": "1.5"}"#,
            r#"{"kind": "unsupported"}"#,
            r#"{"value": 1.5}"#,
        ] {
            assert_same_reading::<WritableCellValueDto>(json);
        }
    }

    #[test]
    fn sequence_items_and_optional_values_are_read_only_from_objects() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            #[serde(deserialize_with = "deserialize_objects")]
            targets: Vec<CalculationTargetDto>,
            #[serde(default, deserialize_with = "deserialize_optional_object")]
            value: Option<WritableCellValueDto>,
        }

        let request = serde_json::from_value::<Request>(json!({
            "targets": [{"sheet": "Sheet1", "start": "A1"}],
            "value": {"kind": "number", "value": 1.5}
        }))
        .expect("objects are read");
        assert_eq!(request.targets[0].start, "A1");
        assert_eq!(
            request.value,
            Some(WritableCellValueDto::Number { value: 1.5 })
        );
        let request = serde_json::from_value::<Request>(json!({"targets": [], "value": null}))
            .expect("null is no value");
        assert_eq!(request.value, None);
        let request =
            serde_json::from_value::<Request>(json!({"targets": []})).expect("omitted is no value");
        assert_eq!(request.value, None);
        for (request, message) in [
            (
                json!({"targets": [["Sheet1", "A1"]]}),
                "invalid type: sequence, expected struct CalculationTargetDto",
            ),
            (
                json!({"targets": ["Sheet1"]}),
                r#"invalid type: string "Sheet1", expected struct CalculationTargetDto"#,
            ),
            (
                json!({"targets": {"sheet": "Sheet1", "start": "A1"}}),
                "invalid type: map, expected a sequence",
            ),
            (
                json!({"targets": [], "value": ["number", 1.5]}),
                "invalid type: sequence, expected internally tagged enum WritableCellValueDto",
            ),
            (
                json!({"targets": [], "value": 1.5}),
                "invalid type: floating point `1.5`, expected internally tagged enum WritableCellValueDto",
            ),
            (
                json!({"targets": [{"sheet": "Sheet1"}]}),
                "missing field `start`",
            ),
            (json!({}), "missing field `targets`"),
        ] {
            assert_eq!(
                serde_json::from_value::<Request>(request)
                    .expect_err("only objects are read")
                    .to_string(),
                message
            );
        }
    }
}
