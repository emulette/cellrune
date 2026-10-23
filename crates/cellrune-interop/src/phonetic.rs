//! Frozen-pane and phonetic-annotation DTOs and their conversions.

use cellrune::{
    CellPhonetics, FrozenPane, PhoneticAlignment, PhoneticProperties, PhoneticRun,
    PhoneticTextRange, PhoneticType, PhoneticWriteOptions,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::InteropError;
use crate::error::{DETAIL_PHONETIC_ALIGNMENT, DETAIL_PHONETIC_TYPE};

/// Rows and columns a worksheet keeps fixed above and to the left of its scrollable pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrozenPaneDto {
    /// Frozen rows.
    pub rows: u32,
    /// Frozen columns.
    pub columns: u32,
}

/// One phonetic run displayed over a half-open range of the cell text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneticRunDto {
    /// Inclusive zero-based start of the range, in UTF-16 code units of the cell text.
    pub start_utf16: u32,
    /// Exclusive zero-based end of the range, in UTF-16 code units of the cell text.
    pub end_utf16: u32,
    /// Phonetic text.
    pub text: String,
}

/// Display properties of a cell's phonetic text.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneticPropertiesDto {
    /// Zero-based workbook font record of the phonetic text. Authoring accepts only `0`.
    #[serde(default)]
    pub font_id: u32,
    /// OOXML conversion token: `halfwidthKatakana`, `fullwidthKatakana`, `Hiragana`, or
    /// `noConversion`; `None` when not declared.
    #[serde(default)]
    pub phonetic_type: Option<String>,
    /// OOXML alignment token: `noControl`, `left`, `center`, or `distributed`; `None` when not
    /// declared.
    #[serde(default)]
    pub alignment: Option<String>,
}

/// One cell's phonetic annotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneticCellDto {
    /// Unqualified A1 address of the annotated text cell.
    pub address: String,
    /// Runs in source order.
    pub runs: Vec<PhoneticRunDto>,
    /// Display properties, when the annotation declares them.
    pub properties: Option<PhoneticPropertiesDto>,
    /// Whether the phonetic text is shown, resolved from the cell, row, and column declarations.
    pub visible: bool,
}

/// One page of a sheet's phonetic annotations, in row-major address order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneticPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Number of cells on the sheet that carry at least one phonetic run.
    pub total: u64,
    /// Offset represented by the first returned cell.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned cells.
    pub entries: Vec<PhoneticCellDto>,
}

pub(crate) fn frozen_pane_dto(pane: FrozenPane) -> FrozenPaneDto {
    FrozenPaneDto {
        rows: pane.frozen_rows(),
        columns: pane.frozen_columns(),
    }
}

pub(crate) fn phonetic_cell_dto(address: String, phonetics: CellPhonetics<'_>) -> PhoneticCellDto {
    PhoneticCellDto {
        address,
        runs: phonetics
            .runs()
            .iter()
            .map(|run| PhoneticRunDto {
                start_utf16: run.base_range().start_utf16(),
                end_utf16: run.base_range().end_utf16(),
                text: run.text().to_owned(),
            })
            .collect(),
        properties: phonetics
            .properties()
            .map(|properties| PhoneticPropertiesDto {
                font_id: properties.font_id(),
                phonetic_type: properties.phonetic_type().map(|value| {
                    match value {
                        PhoneticType::HalfWidthKatakana => "halfwidthKatakana",
                        PhoneticType::FullWidthKatakana => "fullwidthKatakana",
                        PhoneticType::Hiragana => "Hiragana",
                        _ => "noConversion",
                    }
                    .to_owned()
                }),
                alignment: properties.alignment().map(|value| {
                    match value {
                        PhoneticAlignment::NoControl => "noControl",
                        PhoneticAlignment::Left => "left",
                        PhoneticAlignment::Center => "center",
                        _ => "distributed",
                    }
                    .to_owned()
                }),
            }),
        visible: phonetics.effective_visibility(),
    }
}

pub(crate) fn phonetic_runs(runs: Vec<PhoneticRunDto>) -> Result<Vec<PhoneticRun>, InteropError> {
    runs.into_iter()
        .map(|run| {
            Ok(PhoneticRun::new(
                PhoneticTextRange::new(run.start_utf16, run.end_utf16)?,
                run.text,
            )?)
        })
        .collect()
}

pub(crate) fn phonetic_write_options(
    visible: bool,
    properties: Option<PhoneticPropertiesDto>,
) -> Result<PhoneticWriteOptions, InteropError> {
    let options = PhoneticWriteOptions::new(visible);
    let Some(properties) = properties else {
        return Ok(options);
    };
    let mut converted = PhoneticProperties::new(properties.font_id);
    if let Some(token) = properties.phonetic_type {
        converted = converted.with_phonetic_type(match token.as_str() {
            "halfwidthKatakana" => PhoneticType::HalfWidthKatakana,
            "fullwidthKatakana" => PhoneticType::FullWidthKatakana,
            "Hiragana" => PhoneticType::Hiragana,
            "noConversion" => PhoneticType::NoConversion,
            _ => {
                return Err(InteropError::invalid_change(
                    DETAIL_PHONETIC_TYPE.to_owned(),
                ));
            }
        });
    }
    if let Some(token) = properties.alignment {
        converted = converted.with_alignment(match token.as_str() {
            "noControl" => PhoneticAlignment::NoControl,
            "left" => PhoneticAlignment::Left,
            "center" => PhoneticAlignment::Center,
            "distributed" => PhoneticAlignment::Distributed,
            _ => {
                return Err(InteropError::invalid_change(
                    DETAIL_PHONETIC_ALIGNMENT.to_owned(),
                ));
            }
        });
    }
    Ok(options.with_properties(converted))
}
