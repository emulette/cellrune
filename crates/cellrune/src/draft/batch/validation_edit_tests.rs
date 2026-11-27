use super::{clear_data_validations, set_data_validation};
use crate::{
    CellAddress, CellRange, DataValidation, DataValidationErrorStyle, DataValidationOperator,
    DataValidationType, EditBatch, Sheet, SheetId, SheetName, SheetVisibility, ValidationError,
    WorkbookChange, WorkbookDraft,
};

fn range(value: &str) -> CellRange {
    match value.split_once(':') {
        Some((start, end)) => CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range"),
        None => {
            let cell = CellAddress::from_a1(value).expect("cell");
            CellRange::new(cell, cell).expect("cell range")
        }
    }
}

fn ranges(values: &[&str]) -> Vec<CellRange> {
    values.iter().map(|value| range(value)).collect()
}

fn list(values: &[&str], source: &str) -> DataValidation {
    DataValidation::new(ranges(values), DataValidationType::List).with_formula1(source)
}

fn whole(values: &[&str]) -> DataValidation {
    DataValidation::new(ranges(values), DataValidationType::Whole)
        .with_formula1("1")
        .with_formula2("10")
}

fn rules(draft: &WorkbookDraft, sheet: SheetId) -> Vec<(Vec<CellRange>, Option<String>)> {
    draft
        .workbook()
        .sheet_by_id(sheet)
        .expect("sheet")
        .data_validations()
        .iter()
        .map(|validation| {
            (
                validation.ranges().to_vec(),
                validation.formula1().map(str::to_owned),
            )
        })
        .collect()
}

fn draft_with(validations: Vec<DataValidation>) -> (WorkbookDraft, SheetId) {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validations(
            sheet,
            validations,
        )]))
        .expect("initial rules");
    (draft, sheet)
}

#[test]
fn a_rule_takes_its_cells_from_every_other_rule_and_goes_last() {
    let (mut draft, sheet) = draft_with(vec![
        list(&["A1:A10"], "Names"),
        whole(&["B2"]),
        list(&["C1:C3"], "Other"),
    ]);
    let revision = draft.workbook().semantic_revision();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
            sheet,
            list(&["A4:A5", "B2"], "Roles"),
        )]))
        .expect("set");

    assert_eq!(
        rules(&draft, sheet),
        [
            (ranges(&["A1:A3", "A6:A10"]), Some("Names".to_owned())),
            (ranges(&["C1:C3"]), Some("Other".to_owned())),
            (ranges(&["A4:A5", "B2"]), Some("Roles".to_owned())),
        ],
        "B2's rule is left without cells and goes"
    );
    assert_eq!(
        draft.workbook().semantic_revision(),
        revision,
        "data validation is not a calculation input"
    );
    assert!(receipt.calculation_changed_cells().is_empty());
    assert!(!receipt.topology_changed());
}

#[test]
fn clearing_ranges_takes_cells_out_of_rules_split_across_elements() {
    // A rule split by copy and paste: the same rule on three pieces.
    let (mut draft, sheet) = draft_with(vec![
        list(&["A1:A3"], "Names"),
        list(&["A4:A6", "C1"], "Names"),
        list(&["A7:A9"], "Names"),
    ]);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::clear_data_validations(
            sheet,
            ranges(&["A2:A8"]),
        )]))
        .expect("clear");

    assert_eq!(
        rules(&draft, sheet),
        [
            (ranges(&["A1"]), Some("Names".to_owned())),
            (ranges(&["C1"]), Some("Names".to_owned())),
            (ranges(&["A9"]), Some("Names".to_owned())),
        ]
    );
}

#[test]
fn removing_a_rule_by_index_counts_the_batch_changes_before_it() {
    let (mut draft, sheet) = draft_with(vec![list(&["A1"], "First"), list(&["B1"], "Second")]);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::remove_data_validation(sheet, 0),
            WorkbookChange::remove_data_validation(sheet, 0),
        ]))
        .expect("remove both");
    assert!(rules(&draft, sheet).is_empty());

    let error = draft
        .apply_changes(EditBatch::new([WorkbookChange::remove_data_validation(
            sheet, 0,
        )]))
        .expect_err("no rule left");
    assert_eq!(
        error,
        ValidationError::DataValidationIndexOutOfRange {
            sheet_id: sheet.get(),
            index: 0,
        }
    );
}

#[test]
fn rules_are_checked_as_excel_keeps_them() {
    let (mut draft, sheet) = draft_with(Vec::new());
    let id = sheet.get();
    let refused = |draft: &mut WorkbookDraft, validation: DataValidation| {
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
                sheet, validation,
            )]))
            .expect_err("refused rule")
    };
    let at_limit = "가".repeat(32);
    let over_limit = "😀".repeat(17);
    let cases = [
        (
            DataValidation::new(Vec::new(), DataValidationType::AnyValue),
            ValidationError::DataValidationRangesInvalid {
                sheet_id: id,
                count: 0,
            },
        ),
        (
            DataValidation::new(ranges(&["A1"]), DataValidationType::List),
            ValidationError::DataValidationFormulaMissing {
                sheet_id: id,
                slot: 1,
            },
        ),
        (
            DataValidation::new(ranges(&["A1"]), DataValidationType::Whole).with_formula1("1"),
            ValidationError::DataValidationFormulaMissing {
                sheet_id: id,
                slot: 2,
            },
        ),
        (
            list(&["A1"], "SUM("),
            ValidationError::DataValidationFormulaInvalid {
                sheet_id: id,
                slot: 1,
            },
        ),
        (
            list(&["A1"], "Names").with_input_message(true, Some(over_limit.as_str()), None),
            ValidationError::DataValidationTextTooLong {
                sheet_id: id,
                attribute: "promptTitle",
                limit: 32,
            },
        ),
        (
            list(&["A1"], "Names").with_error_alert(
                true,
                DataValidationErrorStyle::Stop,
                None,
                Some("x".repeat(225).as_str()),
            ),
            ValidationError::DataValidationTextTooLong {
                sheet_id: id,
                attribute: "error",
                limit: 224,
            },
        ),
        (
            list(&["A1"], "Names").with_input_message(
                true,
                None,
                Some("tab\tok, vertical\u{0B}not"),
            ),
            ValidationError::DataValidationTextInvalid {
                sheet_id: id,
                attribute: "prompt",
            },
        ),
    ];
    for (validation, expected) in cases {
        assert_eq!(refused(&mut draft, validation), expected);
    }

    let accepted = DataValidation::new(ranges(&["A1"]), DataValidationType::Whole)
        .with_operator(DataValidationOperator::GreaterThan)
        .with_formula1("0")
        .with_input_message(
            true,
            Some(at_limit.as_str()),
            Some("y".repeat(255).as_str()),
        );
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
            sheet, accepted,
        )]))
        .expect("one-sided comparison with titles at Excel's limits");
}

#[test]
fn the_inverse_brings_back_every_rule_in_order() {
    let original = vec![
        list(&["A1:A10"], "Names"),
        whole(&["B2"]),
        list(&["C1:C3"], "Other"),
    ];
    let (mut draft, sheet) = draft_with(original.clone());
    let receipt = draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_data_validation(sheet, list(&["A4:A5", "B2"], "Roles")),
            WorkbookChange::insert_rows(sheet, crate::Row::new(2).expect("row"), 1)
                .expect("insert"),
            WorkbookChange::remove_data_validation(sheet, 0),
            WorkbookChange::clear_data_validations(sheet, ranges(&["C2"])),
        ]))
        .expect("edits");
    let inverse = receipt.inverse().expect("inverse").clone();
    assert!(inverse.unrestored().is_empty());
    draft
        .apply_changes(EditBatch::new(inverse.changes().to_vec()))
        .expect("undo");
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(sheet)
            .expect("sheet")
            .data_validations(),
        original.as_slice()
    );
}

#[test]
fn a_replaced_list_checks_the_rules_the_sheet_did_not_hold() {
    let (mut draft, sheet) = draft_with(vec![list(&["A1"], "Names")]);
    let partial =
        DataValidation::new(ranges(&["B1"]), DataValidationType::Whole).with_formula1("1");
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validations(
                sheet,
                vec![list(&["A1:A2"], "Names"), partial],
            )]))
            .expect_err("a new rule lacking its second bound"),
        ValidationError::DataValidationFormulaMissing {
            sheet_id: sheet.get(),
            slot: 2,
        }
    );
    let long_list = format!("\"{}\"", "x".repeat(256));
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
                sheet,
                list(&["C1"], &long_list),
            )]))
            .expect_err("a list literal longer than Excel keeps"),
        ValidationError::DataValidationTextTooLong {
            sheet_id: sheet.get(),
            attribute: "formula1",
            limit: 255,
        }
    );
}

#[test]
fn setting_or_clearing_cells_of_an_unmodeled_rule_is_refused() {
    let mut sheet = Sheet::new(
        SheetId::new(1).expect("sheet ID"),
        SheetName::new("Rules").expect("sheet name"),
        SheetVisibility::Visible,
    );
    sheet.set_unmodeled_validations(vec![Some(ranges(&["B1:B5"]))]);
    let overlap = ValidationError::DataValidationOverlapsUnmodeledRule { sheet_id: 1 };

    assert_eq!(
        set_data_validation(&mut sheet, &list(&["B5:C5"], "Names")),
        Err(overlap.clone())
    );
    assert_eq!(
        clear_data_validations(&mut sheet, &ranges(&["A1:B1"])),
        Err(overlap.clone())
    );
    assert_eq!(
        set_data_validation(&mut sheet, &list(&["C1:C5"], "Names")),
        Ok(true),
        "cells beside the unmodeled rule take a rule"
    );

    sheet.set_unmodeled_validations(vec![None]);
    assert_eq!(
        clear_data_validations(&mut sheet, &ranges(&["Z100"])),
        Err(overlap),
        "a rule whose ranges could not be read reaches every cell"
    );
}
