use crate::{
    CellAddress, CellRange, CellValue, DefinedNameScope, EditBatch, SheetId, ValidationError,
    WorkbookChange, WorkbookDraft,
};

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn filter_database(draft: &WorkbookDraft, sheet_id: SheetId) -> Option<(String, bool)> {
    draft
        .workbook()
        .defined_names()
        .iter()
        .find(|name| {
            name.scope() == DefinedNameScope::Sheet(sheet_id)
                && name.name() == "_xlnm._FilterDatabase"
        })
        .map(|name| (name.formula().as_str().to_owned(), name.hidden()))
}

#[test]
fn a_sheet_filter_is_created_changed_and_removed_with_its_filter_database_name() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (cell, value) in [("A1", "k"), ("B1", "v")] {
        draft
            .set_cell_value(
                sheet,
                CellAddress::from_a1(cell).expect("cell"),
                CellValue::Text(value.to_owned()),
            )
            .expect("header");
    }
    // Excel 16.114 adds a hidden `_xlnm._FilterDatabase` scoped to the sheet.
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet,
            Some(range("A1:B4")),
        )]))
        .expect("create");
    assert!(receipt.topology_changed());
    let filter = draft
        .workbook()
        .sheet_by_id(sheet)
        .expect("sheet")
        .auto_filter()
        .expect("filter");
    assert_eq!(filter.range(), range("A1:B4"));
    assert_eq!(
        filter_database(&draft, sheet),
        Some(("Sheet1!$A$1:$B$4".to_owned(), true))
    );

    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet,
            Some(range("A1:C9")),
        )]))
        .expect("change");
    assert_eq!(
        filter_database(&draft, sheet),
        Some(("Sheet1!$A$1:$C$9".to_owned(), true))
    );

    let revision = draft.semantic_revision();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet,
            Some(range("A1:C9")),
        )]))
        .expect("unchanged");
    assert_eq!(draft.semantic_revision(), revision);

    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet, None,
        )]))
        .expect("remove");
    assert!(
        draft
            .workbook()
            .sheet_by_id(sheet)
            .expect("sheet")
            .auto_filter()
            .is_none()
    );
    assert_eq!(filter_database(&draft, sheet), None);
}

#[test]
fn a_changed_filter_keeps_the_settings_of_the_columns_it_still_covers() {
    let filter = crate::SheetAutoFilter::from_xlsx(range("B1:E5"), vec![0, 2, 3], vec![2]);
    let changed = filter.respanned(range("C1:D5"));
    // Worksheet columns B, D, and E carried settings; only D stays, now the second column.
    assert_eq!(changed.filter_column_ids(), [1]);
    assert_eq!(changed.filtered_column_ids(), [1]);
}

#[test]
fn removing_a_missing_filter_is_a_no_op_and_an_unknown_sheet_fails() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (cell, value) in [("A1", "k"), ("A2", "x")] {
        draft
            .set_cell_value(
                sheet,
                CellAddress::from_a1(cell).expect("cell"),
                CellValue::Text(value.to_owned()),
            )
            .expect("cell");
    }
    // Without a table there is nothing to overlap; a missing filter's removal is a no-op.
    let revision = draft.semantic_revision();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            sheet, None,
        )]))
        .expect("no filter to remove");
    assert_eq!(draft.semantic_revision(), revision);
    assert_eq!(
        draft.apply_changes(EditBatch::new([WorkbookChange::set_sheet_auto_filter(
            SheetId::new(9).expect("sheet ID"),
            Some(range("A1:A2")),
        )])),
        Err(ValidationError::UnknownSheetId { value: 9 })
    );
}
