//! Non-binary conformance expectation matrices.
//!
//! A matrix is a redistributable description of one workbook — its literals, formulas, and the
//! values a recorded oracle saved for those formulas — precise enough to reconstruct the
//! workbook in memory and compare `CellRune`'s calculation against the oracle, without
//! redistributing the workbook file itself. External workbook redistribution terms vary, so the
//! suite commits only expectation data with explicit source licensing and attribution.
//!
//! The `cellrune_status` field keeps the comparison honest: a documented, deliberate divergence
//! is recorded as such instead of being silently skipped or dressed up as a pass. The same
//! data-driven test also asserts that the divergence still holds, so a behavior change cannot hide
//! behind a stale status.

use cellrune::{CellValue, ExcelError, FiniteNumber};
use serde::{Deserialize, Serialize};

/// Identifier every matrix file must carry in its `schema` field.
pub const MATRIX_SCHEMA: &str = "cellrune_conformance_matrix_v1";

/// The scale-relative epsilon shared with the external-corpus audit harness.
pub const SCALED_EPSILON: f64 = 1e-8;

const MESSAGE_NON_FINITE_NUMBER: &str = "matrix number values must be finite";
const MESSAGE_UNKNOWN_ERROR_LITERAL: &str = "matrix error values must use the Excel display form";

/// One conformance matrix: a reconstructible workbook plus oracle expectations.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    /// Must equal [`MATRIX_SCHEMA`].
    pub schema: String,
    /// The recorded oracle that produced every expected value in `cases`.
    pub oracle: OracleMetadata,
    /// Where the workbook content came from, and under which terms it is redistributable.
    pub source: SourceMetadata,
    /// `excel1900` or `excel1904`; the workbook's serial date epoch.
    pub date_system: String,
    /// Sheet names in workbook order. Sheet identity in `literals` and `cases` is by name.
    pub sheets: Vec<String>,
    /// Workbook-scoped defined names, in workbook order.
    pub defined_names: Vec<DefinedNameEntry>,
    /// Every non-formula cell of the workbook.
    pub literals: Vec<LiteralEntry>,
    /// Every formula cell of the workbook, with the value the oracle saved for it.
    pub cases: Vec<CaseEntry>,
}

/// The exact oracle build the expectations were recorded from.
///
/// "Matches Excel" is not a checkable claim; "matches Microsoft Excel for Mac 16.111 under a
/// Korean-locale host" is. Every field here exists to keep the claim falsifiable.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleMetadata {
    pub product: String,
    pub version: String,
    pub locale: String,
    pub date_system: String,
    pub recorded_at: String,
    pub recorded_by: String,
}

/// Provenance and licensing of the workbook content the matrix describes.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub workbook: String,
    pub license: String,
    pub attribution: String,
}

/// A workbook-scoped defined name.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefinedNameEntry {
    pub name: String,
    pub formula: String,
}

/// A non-formula cell.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralEntry {
    pub sheet: String,
    pub cell: String,
    pub value: ValueEncoding,
}

/// A formula cell together with the oracle's saved value and the comparison policy.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseEntry {
    pub sheet: String,
    pub cell: String,
    pub formula: String,
    /// The value the oracle saved for this formula.
    pub expected: ValueEncoding,
    pub tolerance: Tolerance,
    pub cellrune_status: CellruneStatus,
    /// What `CellRune` returns instead, recorded whenever the status is not `match` so the test can
    /// also detect an undocumented change on the `CellRune` side of a divergence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cellrune_value: Option<ValueEncoding>,
    /// Why a non-`match` status is correct. The test requires this for every divergence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A cell value in matrix encoding, shared by literals and expectations.
///
/// Variants without data are zero-field struct variants, not unit variants: serde does not
/// apply `deny_unknown_fields` to unit variants of internally tagged enums, and hand-edited
/// matrices must fail loudly on a stray field instead of silently dropping it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueEncoding {
    Blank {},
    Number {
        value: f64,
    },
    Text {
        value: String,
    },
    Logical {
        value: bool,
    },
    /// The Excel display form, e.g. `#DIV/0!`.
    Error {
        value: String,
    },
}

/// How a case's actual value is compared against its expected value.
///
/// `Exact` is a zero-field struct variant for the same reason as [`ValueEncoding::Blank`]:
/// unit variants opt out of `deny_unknown_fields` under internal tagging.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Tolerance {
    /// Structural equality; finite numbers compare with exact `f64` equality.
    Exact {},
    /// `|actual - expected| <= epsilon * max(|actual|, |expected|, 1)` — relative with a unit
    /// floor, the same rule the external-corpus audit applies.
    Scaled { epsilon: f64 },
}

impl Tolerance {
    /// Returns whether this comparison policy is bounded and meaningful.
    pub fn is_valid(self) -> bool {
        match self {
            Self::Exact {} => true,
            Self::Scaled { epsilon } => epsilon.is_finite() && (0.0..=1.0).contains(&epsilon),
        }
    }
}

/// How `CellRune` relates to the oracle for one case.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellruneStatus {
    /// `CellRune` must reproduce the expected value within the tolerance.
    Match,
    /// `CellRune` deliberately returns a different value; `note` explains why.
    Divergent,
    /// `CellRune` exceeds the oracle's documented precision; `note` explains why.
    IntentionallyMoreAccurate,
    /// `CellRune` reports the cell as unavailable; `note` names the missing capability.
    NotImplemented,
}

/// Encodes an engine cell value into the matrix representation.
///
/// # Panics
///
/// Panics when the engine grows a `CellValue` variant this schema does not describe yet; the
/// schema must be extended deliberately, not silently.
pub fn encode_cell_value(value: &CellValue) -> ValueEncoding {
    match value {
        CellValue::Blank => ValueEncoding::Blank {},
        CellValue::Number(number) => ValueEncoding::Number {
            value: number.get(),
        },
        CellValue::Text(text) => ValueEncoding::Text {
            value: text.clone(),
        },
        CellValue::Logical(logical) => ValueEncoding::Logical { value: *logical },
        CellValue::Error(error) => ValueEncoding::Error {
            value: error.as_str().to_owned(),
        },
        other => panic!("matrix schema v1 cannot encode cell value {other:?}"),
    }
}

/// Decodes a matrix value into an engine cell value.
///
/// # Errors
///
/// Returns a message when the number is not finite or the error literal is not one of the
/// Excel display forms.
pub fn decode_cell_value(value: &ValueEncoding) -> Result<CellValue, String> {
    match value {
        ValueEncoding::Blank {} => Ok(CellValue::Blank),
        ValueEncoding::Number { value } => FiniteNumber::new(*value)
            .map(CellValue::Number)
            .map_err(|_| format!("{MESSAGE_NON_FINITE_NUMBER}: {value}")),
        ValueEncoding::Text { value } => Ok(CellValue::Text(value.clone())),
        ValueEncoding::Logical { value } => Ok(CellValue::Logical(*value)),
        ValueEncoding::Error { value } => excel_error_from_display(value)
            .map(CellValue::Error)
            .ok_or_else(|| format!("{MESSAGE_UNKNOWN_ERROR_LITERAL}: {value}")),
    }
}

/// Parses the Excel display form of an error value.
pub fn excel_error_from_display(text: &str) -> Option<ExcelError> {
    const ALL: [ExcelError; 10] = [
        ExcelError::Null,
        ExcelError::DivisionByZero,
        ExcelError::Value,
        ExcelError::Reference,
        ExcelError::Name,
        ExcelError::Number,
        ExcelError::NotAvailable,
        ExcelError::GettingData,
        ExcelError::Spill,
        ExcelError::Calculation,
    ];
    ALL.into_iter().find(|error| error.as_str() == text)
}

/// Compares an actual engine value against an encoded expectation under a tolerance.
pub fn values_match(actual: &CellValue, expected: &ValueEncoding, tolerance: Tolerance) -> bool {
    if !tolerance.is_valid() {
        return false;
    }
    match (actual, expected) {
        (CellValue::Number(actual), ValueEncoding::Number { value: expected }) => match tolerance {
            Tolerance::Exact {} => actual.get() == *expected,
            Tolerance::Scaled { epsilon } => {
                let scale = actual.get().abs().max(expected.abs()).max(1.0);
                (actual.get() - *expected).abs() <= epsilon * scale
            }
        },
        (CellValue::Blank, ValueEncoding::Blank {}) => true,
        (CellValue::Text(actual), ValueEncoding::Text { value: expected }) => actual == expected,
        (CellValue::Logical(actual), ValueEncoding::Logical { value: expected }) => {
            actual == expected
        }
        (CellValue::Error(actual), ValueEncoding::Error { value: expected }) => {
            actual.as_str() == expected
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Tolerance, ValueEncoding};

    #[test]
    fn tolerance_accepts_only_bounded_finite_policies() {
        assert!(Tolerance::Exact {}.is_valid());
        assert!(Tolerance::Scaled { epsilon: 0.0 }.is_valid());
        assert!(Tolerance::Scaled { epsilon: 1.0 }.is_valid());
        assert!(
            !Tolerance::Scaled {
                epsilon: -f64::EPSILON
            }
            .is_valid()
        );
        assert!(
            !Tolerance::Scaled {
                epsilon: 1.0 + f64::EPSILON,
            }
            .is_valid()
        );
        assert!(!Tolerance::Scaled { epsilon: f64::NAN }.is_valid());
        assert!(
            !Tolerance::Scaled {
                epsilon: f64::INFINITY,
            }
            .is_valid()
        );
    }

    #[test]
    fn dataless_variants_reject_unknown_fields() {
        assert!(serde_json::from_str::<ValueEncoding>(r#"{"kind":"blank"}"#).is_ok());
        assert!(serde_json::from_str::<ValueEncoding>(r#"{"kind":"blank","value":5}"#).is_err());
        assert!(serde_json::from_str::<Tolerance>(r#"{"mode":"exact"}"#).is_ok());
        assert!(serde_json::from_str::<Tolerance>(r#"{"mode":"exact","epsilon":1e-8}"#).is_err());
    }

    #[test]
    fn data_variants_reject_unknown_fields() {
        assert!(serde_json::from_str::<ValueEncoding>(r#"{"kind":"number","value":1.5}"#).is_ok());
        assert!(
            serde_json::from_str::<ValueEncoding>(r#"{"kind":"number","value":1.5,"bogus":true}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<Tolerance>(r#"{"mode":"scaled","epsilon":1e-8}"#).is_ok());
        assert!(
            serde_json::from_str::<Tolerance>(r#"{"mode":"scaled","epsilon":1e-8,"bogus":1}"#)
                .is_err()
        );
    }
}
