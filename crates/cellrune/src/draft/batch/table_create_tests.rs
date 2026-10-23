use crate::{
    CellAddress, CellContent, CellRange, CellValue, EditBatch, SheetId, TableId, TableName,
    ValidationError, WorkbookChange, WorkbookDraft,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(address(start), address(end)).expect("range")
}

/// `Sheet1` with headers `key` and `val` over two data rows, and a blank third header.
fn draft() -> (WorkbookDraft, SheetId) {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (cell, value) in [
        ("A1", CellValue::Text("key".to_owned())),
        ("B1", CellValue::Text("val".to_owned())),
        ("A2", CellValue::Text("a".to_owned())),
        ("B2", CellValue::number(1.0).expect("number")),
        ("A3", CellValue::Text("b".to_owned())),
        ("B3", CellValue::number(2.0).expect("number")),
    ] {
        draft
            .set_cell_value(sheet, address(cell), value)
            .expect("cell");
    }
    (draft, sheet)
}

#[test]
fn a_new_table_takes_its_header_from_the_first_row_and_the_default_name() {
    let (mut draft, sheet) = draft();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            sheet,
            range("A1:C3"),
            None,
        )]))
        .expect("create");
    assert!(receipt.topology_changed());
    let table_id = TableId::new(1).expect("table ID");
    assert_eq!(receipt.changed_table_ids(), [table_id]);
    let table = draft.workbook().table("Table1").expect("Table1");
    assert_eq!(table.id(), table_id);
    assert_eq!(table.range(), range("A1:C3"));
    let names = table
        .columns()
        .iter()
        .map(|column| column.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["key", "val", "Column3"]);
    // Excel writes the generated name into the blank header cell.
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(sheet)
            .expect("sheet")
            .cell(address("C1"))
            .map(|cell| cell.content().clone()),
        Some(CellContent::Literal(CellValue::Text("Column3".to_owned())))
    );
    assert!(table.auto_filter().is_some());

    // Excel 16.114 names a table made beside one other `Table2`.
    draft
        .set_cell_value(sheet, address("E1"), CellValue::Text("x".to_owned()))
        .expect("header");
    draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            sheet,
            range("E1:E2"),
            None,
        )]))
        .expect("second");
    assert!(draft.workbook().table("Table2").is_some());
}

#[test]
fn a_named_table_and_the_ranges_a_table_cannot_take() {
    let (mut draft, sheet) = draft();
    let things = TableName::new("Things").expect("name");
    draft
        .apply_changes(EditBatch::new([WorkbookChange::create_table(
            sheet,
            range("A1:B3"),
            Some(things.clone()),
        )]))
        .expect("named");
    assert!(draft.workbook().table("Things").is_some());
    let revision = draft.semantic_revision();
    for (target, name, error) in [
        ("A1:A1", None, ValidationError::TableRangeInvalid),
        (
            "B2:C3",
            None,
            ValidationError::OverlappingTables {
                sheet_id: sheet.get(),
                first_table_id: 1,
                second_table_id: 2,
            },
        ),
        (
            "D1:D2",
            Some(things),
            ValidationError::DuplicateTableDisplayName {
                name: "Things".to_owned(),
            },
        ),
    ] {
        assert_eq!(
            draft.apply_changes(EditBatch::new([WorkbookChange::create_table(
                sheet,
                range(target),
                name,
            )])),
            Err(error),
            "{target}"
        );
    }
    draft
        .set_cell_value(
            sheet,
            address("D1"),
            CellValue::number(5.0).expect("number"),
        )
        .expect("number header");
    let revision_after_cell = draft.semantic_revision();
    assert_eq!(
        draft.apply_changes(EditBatch::new([WorkbookChange::create_table(
            sheet,
            range("D1:D2"),
            None,
        )])),
        Err(ValidationError::TableHeaderRequiresText {
            table_id: 2,
            row: 1,
            column: 4,
        })
    );
    assert_eq!(draft.semantic_revision(), revision_after_cell);
    assert!(revision_after_cell > revision);
}
