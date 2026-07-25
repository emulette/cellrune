use super::WorkbookDraft;
use crate::{
    CalculationOptions, CellAddress, CellContent, CellValue, EditBatch, FiniteNumber, FrozenPane,
    PhoneticRun, PhoneticTextRange, PhoneticWriteOptions, SheetName, SheetVisibility,
    ValidationError, WorkbookChange, calculate_workbook,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid cell address")
}

#[test]
fn literal_blank_and_clear_keep_sparse_state_and_revision_consistent() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let a1 = address("A1");
    let b1 = address("B1");

    draft
        .set_cell_value(
            sheet_id,
            a1,
            CellValue::Number(FiniteNumber::new(3.0).expect("finite")),
        )
        .expect("set literal");
    assert!(matches!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(a1)
            .expect("A1")
            .content(),
        CellContent::Literal(CellValue::Number(value)) if value.get() == 3.0
    ));

    draft
        .set_cell_value(sheet_id, a1, CellValue::Blank)
        .expect("blank clears the cell");
    assert!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(a1)
            .is_none()
    );

    draft
        .set_cell_value(sheet_id, b1, CellValue::Text("temporary".to_owned()))
        .expect("set B1");
    let before_clear = draft.semantic_revision();
    assert!(draft.clear_cell(sheet_id, b1).expect("clear existing B1"));
    assert_eq!(draft.semantic_revision(), before_clear + 1);
    assert!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(b1)
            .is_none()
    );

    let before_missing_clear = draft.semantic_revision();
    assert!(!draft.clear_cell(sheet_id, b1).expect("clear missing B1"));
    assert_eq!(draft.semantic_revision(), before_missing_clear);
}

#[test]
fn visibility_rules_cover_noop_last_visible_and_hidden_transitions() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();

    let initial_revision = draft.semantic_revision();
    draft
        .set_sheet_visibility(first, SheetVisibility::Visible)
        .expect("visible noop");
    assert_eq!(draft.semantic_revision(), initial_revision);

    assert!(matches!(
        draft.set_sheet_visibility(first, SheetVisibility::Hidden),
        Err(ValidationError::LastVisibleSheet)
    ));

    let second = draft
        .add_sheet(SheetName::new("Second").expect("sheet name"))
        .expect("second sheet");
    draft
        .set_sheet_visibility(first, SheetVisibility::Hidden)
        .expect("hide one of two visible sheets");
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(first)
            .expect("first sheet")
            .visibility(),
        SheetVisibility::Hidden
    );

    draft
        .set_sheet_visibility(first, SheetVisibility::VeryHidden)
        .expect("hidden-to-very-hidden does not affect the visible-sheet invariant");
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(first)
            .expect("first sheet")
            .visibility(),
        SheetVisibility::VeryHidden
    );
    assert!(matches!(
        draft.set_sheet_visibility(second, SheetVisibility::Hidden),
        Err(ValidationError::LastVisibleSheet)
    ));
}

#[test]
fn annotated_text_edits_are_atomic_and_require_explicit_replacement_or_clear() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let cell = address("A1");
    draft
        .set_annotated_text(
            sheet_id,
            cell,
            "明日",
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "あした")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("annotated text");
    let semantic_revision = draft.semantic_revision();
    let presentation_revision = draft.presentation_revision();

    let invalid =
        PhoneticRun::new(PhoneticTextRange::new(1, 3).expect("range"), "invalid").expect("run");
    assert!(matches!(
        draft.set_phonetics(sheet_id, cell, vec![invalid], PhoneticWriteOptions::show()),
        Err(ValidationError::PhoneticRangeOutOfBounds { .. })
    ));
    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(draft.presentation_revision(), presentation_revision);
    assert_eq!(
        draft
            .presentation()
            .cell_phonetics(sheet_id, cell)
            .expect("phonetics")
            .runs()[0]
            .text(),
        "あした"
    );

    assert!(matches!(
        draft.set_cell_value(sheet_id, cell, CellValue::Text("翌日".to_owned())),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));
    assert!(matches!(
        draft.apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id,
            cell,
            CellValue::Text("翌日".to_owned()),
        )])),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));
    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(draft.presentation_revision(), presentation_revision);

    assert!(
        draft
            .clear_cell(sheet_id, cell)
            .expect("explicit clear succeeds")
    );
    assert!(
        draft
            .presentation()
            .cell_phonetics(sheet_id, cell)
            .is_none()
    );
}

#[test]
fn presentation_only_edits_reuse_the_existing_calculation_identity() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let cell = address("A1");
    draft
        .set_cell_value(sheet_id, cell, CellValue::Text("明日".to_owned()))
        .expect("text");
    let semantic_revision = draft.semantic_revision();
    let fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    draft
        .set_phonetics(
            sheet_id,
            cell,
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "あした")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("phonetics");
    draft
        .set_frozen_pane(sheet_id, FrozenPane::new(1, 0).expect("pane"))
        .expect("pane");

    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(
        crate::calculation::workbook_fingerprint(draft.workbook()),
        fingerprint
    );
    assert!(calculation.matches_workbook(draft.workbook()));
}
