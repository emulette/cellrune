//! Data validation operations of edit schema v2 and their conversions.

use cellrune::{
    CellRange, DataValidation, DataValidationErrorStyle, DataValidationOperator,
    DataValidationType, SheetId, WorkbookChange,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{
    DETAIL_VALIDATION_ERROR_STYLE, DETAIL_VALIDATION_INDEX, DETAIL_VALIDATION_OPERATOR,
    DETAIL_VALIDATION_TYPE,
};
use crate::service::{data_validation_dto, parse_range};
use crate::{DataValidationDto, InteropError};

/// Data validation operations added to edit schema v2, with the semantics of Excel's Data
/// Validation dialog: a cell has at most one rule. They are not calculation inputs and keep the
/// semantic revision. A rule takes the shape `data_validations` returns, so a rule read can be
/// written back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataValidationChangeV2Dto {
    /// Applies a rule to its ranges: the cells leave every other rule, a rule left without cells
    /// goes, and the rule becomes the sheet's last. Fails when the ranges reach a rule the reader
    /// could not model.
    SetDataValidation {
        /// Case-insensitive sheet name.
        sheet: String,
        /// The rule and the ranges it applies to.
        rule: Box<DataValidationDto>,
    },
    /// Takes the cells of the ranges out of every rule, as Excel's Clear All does. Fails when the
    /// ranges reach a rule the reader could not model.
    ClearDataValidations {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 ranges or addresses.
        ranges: Vec<String>,
    },
    /// Removes the rule at a zero-based index of the sheet's rules, counted after the earlier
    /// changes of the batch.
    RemoveDataValidation {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Zero-based rule index.
        index: u64,
    },
    /// Replaces the sheet's whole list of rules, in order; an edit inverse uses it.
    SetDataValidations {
        /// Case-insensitive sheet name.
        sheet: String,
        /// The rules the sheet holds afterwards.
        rules: Vec<DataValidationDto>,
    },
}

impl DataValidationChangeV2Dto {
    /// Returns the case-insensitive sheet name the operation targets.
    pub fn sheet(&self) -> &str {
        match self {
            Self::SetDataValidation { sheet, .. }
            | Self::ClearDataValidations { sheet, .. }
            | Self::RemoveDataValidation { sheet, .. }
            | Self::SetDataValidations { sheet, .. } => sheet,
        }
    }
}

/// Converts one edit-schema-v2 data validation operation on a resolved sheet.
pub(crate) fn validation_change(
    sheet_id: SheetId,
    change: DataValidationChangeV2Dto,
) -> Result<WorkbookChange, InteropError> {
    Ok(match change {
        DataValidationChangeV2Dto::SetDataValidation { rule, .. } => {
            WorkbookChange::set_data_validation(sheet_id, validation_from_dto(*rule)?)
        }
        DataValidationChangeV2Dto::ClearDataValidations { ranges, .. } => {
            WorkbookChange::clear_data_validations(sheet_id, parse_ranges(&ranges)?)
        }
        DataValidationChangeV2Dto::RemoveDataValidation { index, .. } => {
            WorkbookChange::remove_data_validation(
                sheet_id,
                usize::try_from(index).map_err(|_| {
                    InteropError::invalid_change(DETAIL_VALIDATION_INDEX.to_owned())
                })?,
            )
        }
        DataValidationChangeV2Dto::SetDataValidations { rules, .. } => {
            WorkbookChange::set_data_validations(
                sheet_id,
                rules
                    .into_iter()
                    .map(validation_from_dto)
                    .collect::<Result<_, _>>()?,
            )
        }
    })
}

/// Spells the replacement of a sheet's rules, for an edit inverse.
pub(crate) fn set_data_validations_dto(
    sheet: String,
    validations: &[DataValidation],
) -> DataValidationChangeV2Dto {
    DataValidationChangeV2Dto::SetDataValidations {
        sheet,
        rules: validations.iter().map(data_validation_dto).collect(),
    }
}

fn validation_from_dto(rule: DataValidationDto) -> Result<DataValidation, InteropError> {
    let mut validation = DataValidation::new(
        parse_ranges(&rule.ranges)?,
        token(
            DataValidationType::from_token(&rule.validation_type),
            DETAIL_VALIDATION_TYPE,
        )?,
    )
    .with_operator(token(
        DataValidationOperator::from_token(&rule.operator),
        DETAIL_VALIDATION_OPERATOR,
    )?)
    .with_allow_blank(rule.allow_blank)
    .with_in_cell_dropdown(rule.in_cell_dropdown)
    .with_input_message(
        rule.show_input_message,
        rule.prompt_title.as_deref(),
        rule.prompt_message.as_deref(),
    )
    .with_error_alert(
        rule.show_error_message,
        token(
            DataValidationErrorStyle::from_token(&rule.error_style),
            DETAIL_VALIDATION_ERROR_STYLE,
        )?,
        rule.error_title.as_deref(),
        rule.error_message.as_deref(),
    );
    if let Some(formula) = rule.formula1 {
        validation = validation.with_formula1(formula);
    }
    if let Some(formula) = rule.formula2 {
        validation = validation.with_formula2(formula);
    }
    Ok(validation)
}

fn token<T>(parsed: Option<T>, detail: &str) -> Result<T, InteropError> {
    parsed.ok_or_else(|| InteropError::invalid_change(detail.to_owned()))
}

fn parse_ranges(ranges: &[String]) -> Result<Vec<CellRange>, InteropError> {
    ranges.iter().map(|range| parse_range(range)).collect()
}
