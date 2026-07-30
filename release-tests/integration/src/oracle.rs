//! Typed schema and comparison rules for the explicit Excel-oracle audit.

use std::collections::BTreeMap;

use cellrune::{CalculationCellResult, CellValue};
use serde::{Deserialize, Serialize};

/// Metadata schema accepted by the local oracle checker.
pub const METADATA_SCHEMA: &str = "cellrune_excel_oracle_metadata_v2";
/// Host-matrix suite schema accepted by the local oracle checker.
pub const SUITE_SCHEMA: &str = "cellrune_excel_oracle_suite_v2";
/// Stable case-manifest schema accepted by the local oracle checker.
pub const CASE_MANIFEST_SCHEMA: &str = "cellrune_excel_oracle_cases_v2";
/// Saved-host observation schema accepted by the local oracle checker.
pub const OBSERVATIONS_SCHEMA: &str = "cellrune_excel_oracle_observations_v2";
/// Default scale-relative tolerance for finite numeric results.
pub const DEFAULT_SCALED_EPSILON: f64 = 1e-8;

/// One workbook's provenance and audit selection.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub schema: String,
    pub workbook: String,
    pub sha256: Option<String>,
    pub source_workbook_sha256: Option<String>,
    pub case_manifest_sha256: Option<String>,
    pub formula_cells: usize,
    pub selected_cases: Option<usize>,
    pub date_system: String,
    pub iterative_calculation: bool,
    pub case_selection: CaseSelection,
    pub source: SourceMetadata,
    pub generator: GeneratorMetadata,
    pub oracle: OracleMetadata,
}

/// How the checker derives the complete set of expected case keys.
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaseSelection {
    /// Every cell that directly stores formula content.
    AllFormulaCells,
    /// Formula anchors plus every cell in a declared array result range.
    AllFormulaResults,
    /// Formula cells in the named sheets and one one-based column.
    ListedSheetsColumn { sheets: Vec<String>, column: u32 },
    /// Formula anchors are selected by the active case manifest addresses.
    ManifestAddresses,
}

/// Workbook-content provenance.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub name: String,
    pub license: String,
    pub url: Option<String>,
    pub revision: Option<String>,
}

/// Generator provenance, when the project authored the workbook.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorMetadata {
    pub name: Option<String>,
    pub revision: Option<String>,
    pub harness_sha256: Option<String>,
}

/// Excel host that wrote the saved calculation cache.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleMetadata {
    pub application: String,
    pub version: String,
    pub channel: Option<String>,
    pub os: Option<String>,
    pub locale: Option<String>,
    pub saved_at: String,
    pub suite_id: Option<String>,
    pub feature_set_id: Option<String>,
    pub host_profile_id: Option<String>,
    pub product_tier: Option<String>,
    pub host_build: Option<String>,
}

/// Required host profiles that jointly cover one stable case manifest.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleSuite {
    pub schema: String,
    pub suite_id: String,
    pub case_manifest: CaseManifestReference,
    pub feature_set_id: Option<String>,
    pub planned_release_range: Option<PlannedReleaseRange>,
    pub state: Option<String>,
    pub active_case_count: Option<usize>,
    pub source_workbook: Option<SourceWorkbookMetadata>,
    pub case_selection: Option<CaseSelection>,
    pub generator: Option<GeneratorMetadata>,
    pub profiles: Vec<HostProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceWorkbookMetadata {
    pub sha256: String,
    pub formula_cells: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseManifestReference {
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedReleaseRange {
    pub first: String,
    pub last: String,
}

/// One exact Excel product/locale/add-in profile.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostProfile {
    pub profile_id: String,
    pub directory: String,
    pub application: String,
    pub os: String,
    pub product_tier: String,
    pub locale: String,
    pub add_ins: BTreeMap<String, bool>,
    pub artifacts: Option<ProfileArtifacts>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileArtifacts {
    pub workbook: ArtifactReference,
    pub metadata: ArtifactReference,
    pub observations: ArtifactReference,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub file: String,
    pub sha256: String,
    pub formula_cells: Option<usize>,
    pub selected_cases: Option<usize>,
}

/// Stable identity and authored formula inventory for one suite.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseManifest {
    pub schema: String,
    pub suite_id: String,
    pub generator: GeneratorMetadata,
    pub feature_set_id: Option<String>,
    pub case_count: Option<usize>,
    pub active_case_count: Option<usize>,
    pub cases: Vec<ManifestCase>,
}

/// One active or inactive catalog/curated case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestCase {
    pub case_key: String,
    pub function: String,
    pub category: String,
    pub scenario: String,
    pub authored_formula: String,
    pub storage_formula: String,
    pub allowed_host_rewrites: Vec<HostFormulaRewrite>,
    pub catalog_address: String,
    pub active: bool,
    pub formula_address: Option<String>,
    pub seed_classification: String,
    pub inactive_reason: Option<String>,
    pub exclusion: Option<OracleExclusion>,
}

/// One explicitly reviewed formula spelling change that a host may make.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostFormulaRewrite {
    pub kind: String,
    pub profile_ids: Vec<String>,
}

/// Why a catalog case does not participate in the active host matrix.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OracleExclusion {
    Volatile {
        reason: String,
        evidence: String,
    },
    NativeAutomation {
        reason: String,
        evidence: String,
    },
    ExternalService {
        reason: String,
        evidence: String,
    },
    ExternalConnection {
        reason: String,
        evidence: String,
    },
    HostDependent {
        reason: String,
        evidence: String,
    },
    FixtureUnavailable {
        reason: String,
        evidence: String,
    },
    HostMatrixUnavailable {
        reason: String,
        evidence: String,
        unsupported_profile_ids: Vec<String>,
    },
}

impl OracleExclusion {
    /// Returns the reviewed rationale and evidence shared by every exclusion kind.
    pub fn rationale(&self) -> (&str, &str) {
        match self {
            Self::Volatile { reason, evidence }
            | Self::NativeAutomation { reason, evidence }
            | Self::ExternalService { reason, evidence }
            | Self::ExternalConnection { reason, evidence }
            | Self::HostDependent { reason, evidence }
            | Self::FixtureUnavailable { reason, evidence }
            | Self::HostMatrixUnavailable {
                reason, evidence, ..
            } => (reason, evidence),
        }
    }
}

/// Facts extracted from one Excel-saved workbook.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observations {
    pub schema: String,
    pub suite_id: String,
    pub host_profile_id: String,
    pub saved_at: String,
    pub workbook_sha256: Option<String>,
    pub source_workbook_sha256: Option<String>,
    pub case_manifest_sha256: Option<String>,
    pub harness_sha256: Option<String>,
    pub feature_set_id: Option<String>,
    pub case_count: Option<usize>,
    pub cases: Vec<ObservedCase>,
}

/// One saved formula/cache observation, joined by stable case key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCase {
    pub case_key: String,
    pub address: String,
    pub saved_formula: String,
    pub formula_rewrites: Vec<String>,
    pub cache_status: CacheStatus,
    pub cache_value: Option<String>,
    pub cache_type: String,
    pub rich_error: RichErrorObservation,
    #[serde(default)]
    pub result: Option<ObservedResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedResult {
    pub range: String,
    pub rows: u32,
    pub columns: u32,
    pub cells: Vec<ObservedResultCell>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedResultCell {
    pub address: String,
    pub cache_status: CacheStatus,
    pub cache_value: Option<String>,
    pub cache_type: String,
    pub rich_error: RichErrorObservation,
}

/// Whether the saved cell carries a host-computed value that can be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    Semantic,
    MissingSemanticCache,
    Circular,
}

/// Raw modern-error metadata retained alongside the legacy cache fallback.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichErrorObservation {
    pub present: bool,
    pub raw_error: Option<String>,
    pub vm_index: Option<u32>,
    pub record_index: Option<u32>,
    pub structure_index: Option<u32>,
    pub error_type_code: Option<u32>,
    pub resolved_error: Option<String>,
    pub fallback_error: Option<String>,
}

/// Every reviewed case, keyed as `Sheet!A1`.
pub type Expectations = BTreeMap<String, Expectation>;

/// Reviewed relation between CellRune and one Excel-saved value.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub classification: Classification,
    pub excel_value: String,
    pub excel_type: String,
    #[serde(default)]
    pub excel_rich_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cellrune_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cellrune_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Comparator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Reviewed case state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Match,
    Divergent,
    NotImplemented,
    Unreadable,
    HostUnsupported,
    Excluded,
    Unclassified,
}

/// Numeric or structural comparison policy.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Comparator {
    Scaled { epsilon: f64 },
    Exact {},
    ExactBits {},
    AbsRel { abs: f64, rel: f64 },
}

/// JSON-friendly scalar observed from a saved cache or calculation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservedValue {
    pub value: String,
    pub value_type: String,
}

impl ObservedValue {
    /// Encodes a public scalar cell value without losing its type.
    pub fn from_cell(value: &CellValue) -> Result<Self, String> {
        match value {
            CellValue::Blank => Ok(Self::new(String::new(), "blank")),
            CellValue::Number(number) => Ok(Self::new(number.get().to_string(), "n")),
            CellValue::Text(text) => Ok(Self::new(text.clone(), "str")),
            CellValue::Logical(value) => {
                Ok(Self::new(if *value { "1" } else { "0" }.to_owned(), "b"))
            }
            CellValue::Error(error) => Ok(Self::new(error.as_str().to_owned(), "e")),
            other => Err(format!("unsupported oracle scalar value: {other:?}")),
        }
    }

    /// Encodes a calculation result when it contains a scalar value.
    pub fn from_result(result: &CalculationCellResult) -> Result<Option<Self>, String> {
        match result {
            CalculationCellResult::Value(value) => Self::from_cell(value).map(Some),
            CalculationCellResult::Unavailable(_) => Ok(None),
        }
    }

    pub fn from_expectation(expectation: &Expectation) -> Self {
        Self::new(
            expectation.excel_value.clone(),
            expectation.excel_type.clone(),
        )
    }

    pub fn from_recorded_cellrune(expectation: &Expectation) -> Option<Self> {
        Some(Self::new(
            expectation.cellrune_value.clone()?,
            expectation.cellrune_type.clone()?,
        ))
    }

    fn new(value: String, value_type: impl Into<String>) -> Self {
        Self {
            value,
            value_type: value_type.into(),
        }
    }

    fn number(&self) -> Result<f64, String> {
        if self.value_type != "n" {
            return Err(format!(
                "expected numeric oracle value, found type {}",
                self.value_type
            ));
        }
        self.value
            .parse::<f64>()
            .map_err(|error| format!("invalid oracle number {}: {error}", self.value))
    }
}

/// Compares two typed scalar values under the reviewed policy.
pub fn values_match(
    actual: &ObservedValue,
    expected: &ObservedValue,
    comparator: Option<Comparator>,
) -> Result<bool, String> {
    if actual.value_type != expected.value_type {
        return Ok(false);
    }
    let policy = comparator.unwrap_or_else(|| {
        if expected.value_type == "n" {
            Comparator::Scaled {
                epsilon: DEFAULT_SCALED_EPSILON,
            }
        } else {
            Comparator::Exact {}
        }
    });
    match policy {
        Comparator::Exact {} => {
            if expected.value_type == "n" {
                Ok(actual.number()? == expected.number()?)
            } else {
                Ok(actual.value == expected.value)
            }
        }
        Comparator::ExactBits {} => Ok(actual.number()?.to_bits() == expected.number()?.to_bits()),
        Comparator::Scaled { epsilon } => {
            validate_tolerance("epsilon", epsilon)?;
            let actual = actual.number()?;
            let expected = expected.number()?;
            let scale = actual.abs().max(expected.abs()).max(1.0);
            Ok((actual - expected).abs() <= epsilon * scale)
        }
        Comparator::AbsRel { abs, rel } => {
            validate_tolerance("abs", abs)?;
            validate_tolerance("rel", rel)?;
            let actual = actual.number()?;
            let expected = expected.number()?;
            let scale = actual.abs().max(expected.abs());
            Ok((actual - expected).abs() <= abs + rel * scale)
        }
    }
}

fn validate_tolerance(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} tolerance must be finite and non-negative"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Comparator, Metadata, ObservedValue, values_match};

    fn number(value: &str) -> ObservedValue {
        ObservedValue {
            value: value.to_owned(),
            value_type: "n".to_owned(),
        }
    }

    #[test]
    fn scaled_comparison_does_not_protect_near_zero_cases() {
        assert!(
            values_match(
                &number("0.000000009"),
                &number("0"),
                Some(Comparator::Scaled { epsilon: 1e-8 })
            )
            .expect("valid comparison")
        );
        assert!(
            !values_match(
                &number("0.000000009"),
                &number("0"),
                Some(Comparator::ExactBits {})
            )
            .expect("valid comparison")
        );
    }

    #[test]
    fn typed_values_do_not_coerce() {
        let text = ObservedValue {
            value: "1".to_owned(),
            value_type: "str".to_owned(),
        };
        assert!(!values_match(&number("1"), &text, None).expect("valid comparison"));
    }

    #[test]
    fn legacy_metadata_does_not_require_host_matrix_provenance() {
        let metadata: Metadata = serde_json::from_value(serde_json::json!({
            "schema": "cellrune_excel_oracle_metadata_v1",
            "workbook": "workbook.xlsx",
            "sha256": "a".repeat(64),
            "formula_cells": 1,
            "date_system": "excel1900",
            "iterative_calculation": false,
            "case_selection": {"mode": "all_formula_cells"},
            "source": {
                "name": "legacy",
                "license": "Apache-2.0",
                "url": null,
                "revision": null
            },
            "generator": {"name": null, "revision": null},
            "oracle": {
                "application": "Microsoft Excel",
                "version": "AppVersion 16",
                "channel": null,
                "os": null,
                "locale": null,
                "saved_at": "2017-01-01T00:00:00Z",
                "suite_id": null,
                "host_profile_id": null,
                "product_tier": null
            }
        }))
        .expect("legacy metadata");

        assert_eq!(metadata.source_workbook_sha256, None);
        assert_eq!(metadata.oracle.host_build, None);
    }
}
