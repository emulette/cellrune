//! Data validation edits with the semantics of Excel's Data Validation dialog: a cell has at most
//! one rule.

use crate::grid::{intersect, subtract};
use crate::{CellRange, DataValidation, Sheet, ValidationError};

/// Applies `validation` to its ranges: the cells leave every other rule, a rule left without cells
/// goes, and the rule is appended. Returns whether the sheet's rules changed.
pub(super) fn set_data_validation(
    sheet: &mut Sheet,
    validation: &DataValidation,
) -> Result<bool, ValidationError> {
    validation.check(sheet.id().get())?;
    check_unmodeled(sheet, validation.ranges())?;
    let mut validations = without_ranges(sheet.data_validations(), validation.ranges());
    validations.push(validation.clone());
    Ok(replace(sheet, validations))
}

/// Takes the cells of `ranges` out of every rule, removing a rule left without cells. Returns
/// whether the sheet's rules changed.
pub(super) fn clear_data_validations(
    sheet: &mut Sheet,
    ranges: &[CellRange],
) -> Result<bool, ValidationError> {
    check_unmodeled(sheet, ranges)?;
    let validations = without_ranges(sheet.data_validations(), ranges);
    Ok(replace(sheet, validations))
}

/// Removes the rule at `index`. Returns whether the sheet's rules changed, which they always do.
pub(super) fn remove_data_validation(
    sheet: &mut Sheet,
    index: usize,
) -> Result<bool, ValidationError> {
    if index >= sheet.data_validations().len() {
        return Err(ValidationError::DataValidationIndexOutOfRange {
            sheet_id: sheet.id().get(),
            index,
        });
    }
    let mut validations = sheet.data_validations().to_vec();
    validations.remove(index);
    Ok(replace(sheet, validations))
}

/// Replaces the sheet's rules with `validations`. Returns whether they changed.
///
/// A rule the sheet holds, or its source sheet `read` held, whatever its ranges, need only be
/// writable, since an edit inverse sets back rules read from a file that a new rule would not be
/// allowed to be; any other rule is checked as a new one.
pub(super) fn set_data_validations(
    sheet: &mut Sheet,
    validations: &[DataValidation],
    read: &[DataValidation],
) -> Result<bool, ValidationError> {
    for validation in validations {
        let held = sheet
            .data_validations()
            .iter()
            .chain(read)
            .any(|held| held.same_rule(validation));
        if held {
            validation.check_writable(sheet.id().get())?;
        } else {
            validation.check(sheet.id().get())?;
        }
    }
    Ok(replace(sheet, validations.to_vec()))
}

/// Refuses an edit of `ranges` that reaches a rule the reader could not model: the model cannot
/// take its cells out of it, so the cells would keep two rules. A rule whose ranges could not be
/// read reaches every cell.
fn check_unmodeled(sheet: &Sheet, ranges: &[CellRange]) -> Result<(), ValidationError> {
    let reached = sheet.unmodeled_validations().iter().any(|unmodeled| {
        unmodeled.as_ref().is_none_or(|unmodeled| {
            unmodeled.iter().any(|left| {
                ranges
                    .iter()
                    .any(|right| intersect(*left, *right).is_some())
            })
        })
    });
    if reached && !ranges.is_empty() {
        return Err(ValidationError::DataValidationOverlapsUnmodeledRule {
            sheet_id: sheet.id().get(),
        });
    }
    Ok(())
}

/// Returns the rules with the cells of `ranges` taken out, dropping a rule left without cells.
fn without_ranges(validations: &[DataValidation], ranges: &[CellRange]) -> Vec<DataValidation> {
    validations
        .iter()
        .filter_map(|validation| {
            let mut kept = validation.ranges().to_vec();
            for removed in ranges {
                kept = kept
                    .into_iter()
                    .flat_map(|range| subtract(range, *removed))
                    .collect();
            }
            if kept.is_empty() {
                return None;
            }
            let mut validation = validation.clone();
            if kept != validation.ranges() {
                validation.set_ranges(kept);
            }
            Some(validation)
        })
        .collect()
}

fn replace(sheet: &mut Sheet, validations: Vec<DataValidation>) -> bool {
    if sheet.data_validations() == validations.as_slice() {
        return false;
    }
    sheet.set_data_validations(validations);
    true
}

#[cfg(test)]
#[path = "validation_edit_tests.rs"]
mod tests;
