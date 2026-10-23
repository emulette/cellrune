use super::{UnrestoredContent, UnrestoredContentKind};
use crate::workbook::{NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationHints, CellAddress, CellContent, CellRange, CellValue, Column, DateSystem,
    DefinedName, DefinedNameScope, EditBatch, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, NumberFormat, Provenance, Row, SavedResult, Sheet, SheetId, SheetName,
    SheetVisibility, Table, TableColumn, TableId, TableName, WorkbookChange, WorkbookDraft,
    WorkbookSnapshot, WorkbookSource,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').unwrap_or((value, value));
    CellRange::new(address(start), address(end)).expect("range")
}

fn id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
}

fn row(value: u32) -> Row {
    Row::new(value).expect("row")
}

fn sheet(sheet_id: u32, name: &str, cells: &[(&str, &str)]) -> Sheet {
    let mut sheet = Sheet::new(
        id(sheet_id),
        SheetName::new(name).expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (cell, input) in cells {
        let content = match input.strip_prefix('=') {
            Some(text) => CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(text).expect("formula"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
            None => CellContent::Literal(
                CellValue::number(input.parse().expect("number")).expect("number"),
            ),
        };
        sheet.upsert_cell(address(cell), content, NumberFormat::default());
    }
    sheet
}

/// `S1` holds values in `A1:A6`, formulas reading them, the `Things` table over `C1:D5`, and a
/// merged range; `S2` and a name read `S1`.
fn probe() -> WorkbookDraft {
    let mut s1 = sheet(
        1,
        "S1",
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("A4", "4"),
            ("A5", "5"),
            ("A6", "6"),
            ("B1", "=SUM(A1:A6)"),
            ("B2", "=A3*2"),
            ("B6", "=A6"),
            ("C2", "1"),
            ("C3", "2"),
            ("C4", "3"),
            ("C5", "4"),
        ],
    );
    s1.upsert_cell(
        address("C1"),
        CellContent::Literal(CellValue::Text("key".to_owned())),
        NumberFormat::default(),
    );
    s1.upsert_cell(
        address("D1"),
        CellContent::Literal(CellValue::Text("val".to_owned())),
        NumberFormat::default(),
    );
    s1.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Things").expect("name"),
            TableName::new("Things").expect("name"),
            range("C1:D5"),
            1,
            0,
            vec![
                TableColumn::new(1, "key", None).expect("column"),
                TableColumn::new(2, "val", None).expect("column"),
            ],
        )
        .expect("table"),
    ]);
    s1.set_merged_ranges(vec![range("F3:F4")]);
    let s2 = sheet(2, "S2", &[("A1", "='S1'!A3+'S1'!A4")]);
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets: vec![s1, s2],
        non_worksheet_tabs: NonWorksheetTabs::default(),
        defined_names: vec![
            DefinedName::new(
                "Block",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("'S1'!$A$3:$A$4").expect("name"),
                false,
            )
            .expect("name"),
        ],
        diagnostics: Vec::new(),
        date_system: DateSystem::Excel1900,
        calculation_hints: CalculationHints::default(),
        source: WorkbookSource::default(),
        provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
    })
    .expect("workbook");
    let mut draft = WorkbookDraft::from_snapshot_for_test(workbook);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_row_height(id(1), row(3), Some(30.0)),
            WorkbookChange::set_cell_format(
                id(1),
                address("A4"),
                crate::CellFormatEdit::new().with_font(
                    crate::Font::new()
                        .with_name("Calibri")
                        .and_then(|font| font.with_size(14.0))
                        .expect("font"),
                ),
            ),
        ]))
        .expect("layout");
    draft
}

/// The shape of a table that a recreated one must match: its range, names, and columns.
fn table_shape(table: &Table) -> (CellRange, String, Vec<String>) {
    (
        table.range(),
        table.display_name().as_str().to_owned(),
        table
            .columns()
            .iter()
            .map(|column| column.name().to_owned())
            .collect(),
    )
}

/// Applies `changes`, then the inverse its receipt carries, and checks that the sheets in tab
/// order, their names, visibility, cells, tables, auto-filters, row heights, and cell formats, and
/// the names are back; a recreated sheet or table may hold another ID. Returns what the inverse
/// could not restore.
fn round_trip(changes: Vec<WorkbookChange>) -> Vec<UnrestoredContent> {
    let original = probe();
    let mut draft = original.clone();
    let receipt = draft
        .apply_changes(EditBatch::new(changes))
        .expect("structural batch");
    let inverse = receipt
        .inverse()
        .expect("structural batches carry an inverse")
        .clone();
    draft
        .apply_changes(EditBatch::new(inverse.changes().to_vec()))
        .expect("inverse applies");
    let names = |draft: &WorkbookDraft| {
        draft
            .workbook()
            .sheets()
            .iter()
            .map(|sheet| (sheet.name().as_str().to_owned(), sheet.visibility()))
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&original), names(&draft));
    for (before, after) in original
        .workbook()
        .sheets()
        .iter()
        .zip(draft.workbook().sheets())
    {
        assert_eq!(
            before
                .cells()
                .map(|cell| (cell.address(), cell.content()))
                .collect::<Vec<_>>(),
            after
                .cells()
                .map(|cell| (cell.address(), cell.content()))
                .collect::<Vec<_>>(),
            "{}",
            before.name().as_str()
        );
        assert_eq!(
            before.tables().iter().map(table_shape).collect::<Vec<_>>(),
            after.tables().iter().map(table_shape).collect::<Vec<_>>()
        );
        assert_eq!(
            before.auto_filter().map(crate::SheetAutoFilter::range),
            after.auto_filter().map(crate::SheetAutoFilter::range)
        );
        for line in 1..=10 {
            assert_eq!(
                original.presentation().row_height(before.id(), row(line)),
                draft.presentation().row_height(after.id(), row(line)),
                "row {line}"
            );
            for column in 1..=6 {
                let cell = CellAddress::new(row(line), Column::new(column).expect("column"));
                assert_eq!(
                    original.cell_format(before.id(), cell),
                    draft.cell_format(after.id(), cell),
                    "{cell}"
                );
            }
        }
    }
    let defined_names = |draft: &WorkbookDraft| {
        draft
            .workbook()
            .defined_names()
            .iter()
            .map(|name| {
                let scope = match name.scope() {
                    DefinedNameScope::Workbook => None,
                    DefinedNameScope::Sheet(sheet) => draft
                        .workbook()
                        .sheet_by_id(sheet)
                        .map(|sheet| sheet.name().as_str().to_owned()),
                };
                (name.name().to_owned(), scope, name.formula().clone())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(defined_names(&original), defined_names(&draft));
    inverse.unrestored().to_vec()
}

#[test]
fn an_insertion_is_undone_by_deleting_the_inserted_rows() {
    let mut draft = probe();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::insert_rows(
            id(1),
            row(3),
            2,
        )
        .expect("insertion")]))
        .expect("insertion");
    assert_eq!(
        receipt.inverse().expect("inverse").changes(),
        &[WorkbookChange::delete_rows(id(1), row(3), row(4)).expect("deletion")]
    );
    assert!(
        round_trip(vec![
            WorkbookChange::insert_rows(id(1), row(3), 2).expect("insertion")
        ])
        .is_empty()
    );
}

#[test]
fn a_deletion_is_undone_by_inserting_rows_and_restoring_what_they_held() {
    let unrestored = round_trip(vec![
        WorkbookChange::delete_rows(id(1), row(3), row(4)).expect("deletion"),
    ]);
    // The merged range lost both of its rows, which no change can merge again.
    assert_eq!(
        unrestored
            .iter()
            .map(UnrestoredContent::kind)
            .collect::<Vec<_>>(),
        [UnrestoredContentKind::MergedRanges]
    );
    // Deleting the last data row leaves the table short after the rows come back, so the
    // inverse resizes it.
    round_trip(vec![
        WorkbookChange::delete_rows(id(1), row(5), row(5)).expect("deletion"),
    ]);
}

#[test]
fn moves_and_copies_are_undone_by_changes_too() {
    round_trip(vec![
        WorkbookChange::move_rows(id(1), row(7), row(8), row(1)).expect("move"),
    ]);
    round_trip(vec![
        WorkbookChange::move_columns(
            id(1),
            Column::new(1).expect("A"),
            Column::new(1).expect("A"),
            Column::new(5).expect("E"),
        )
        .expect("move"),
    ]);
    round_trip(vec![
        WorkbookChange::move_range(id(1), range("A2:A5"), address("H2")).expect("move"),
    ]);
    round_trip(vec![
        WorkbookChange::copy_range(id(1), range("B1:B2"), id(1), range("A3:A6")).expect("copy"),
        WorkbookChange::set_cell_value(id(1), address("A1"), CellValue::number(9.0).expect("9")),
        WorkbookChange::insert_columns(id(1), Column::new(2).expect("B"), 1).expect("insertion"),
    ]);
}

#[test]
fn batches_without_structural_edits_carry_no_inverse() {
    let mut draft = probe();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            id(1),
            address("A1"),
            CellValue::number(2.0).expect("2"),
        )]))
        .expect("value");
    assert!(receipt.inverse().is_none());
}

#[test]
fn sheet_name_and_table_edits_are_undone_by_changes_too() {
    let things = TableId::new(1).expect("table ID");
    for changes in [
        vec![WorkbookChange::move_sheet(id(1), 1)],
        vec![WorkbookChange::copy_sheet(id(1), 2)],
        vec![WorkbookChange::rename_defined_name(
            DefinedNameScope::Workbook,
            "Block",
            "Blk",
        )],
        vec![WorkbookChange::convert_table_to_range(things)],
        vec![WorkbookChange::set_table_range(things, range("C1:D6"))],
        vec![WorkbookChange::set_sheet_auto_filter(
            id(1),
            Some(range("A1:B6")),
        )],
        vec![
            WorkbookChange::set_cell_value(
                id(2),
                address("B1"),
                CellValue::Text("head".to_owned()),
            ),
            WorkbookChange::create_table(
                id(2),
                range("B1:B3"),
                Some(TableName::new("Fresh").expect("name")),
            ),
        ],
    ] {
        let unrestored = round_trip(changes.clone());
        assert!(unrestored.is_empty(), "{changes:?}: {unrestored:?}");
    }
}

#[test]
fn a_deleted_sheet_comes_back_with_its_cells_tables_names_and_formats() {
    let unrestored = round_trip(vec![WorkbookChange::delete_sheet(id(1))]);
    // A merged range is the one structure of the sheet no change can bring back.
    assert_eq!(
        unrestored
            .iter()
            .map(UnrestoredContent::kind)
            .collect::<Vec<_>>(),
        [UnrestoredContentKind::MergedRanges]
    );
    assert!(round_trip(vec![WorkbookChange::delete_sheet(id(2))]).is_empty());
}

#[test]
fn mixed_batches_undo_every_structural_edit_in_reverse() {
    let things = TableId::new(1).expect("table ID");
    let insert = || WorkbookChange::insert_rows(id(1), row(2), 1).expect("insertion");
    for changes in [
        vec![WorkbookChange::move_sheet(id(2), 0), insert()],
        vec![WorkbookChange::convert_table_to_range(things), insert()],
        vec![insert(), WorkbookChange::convert_table_to_range(things)],
        vec![
            WorkbookChange::insert_rows(id(2), row(1), 2).expect("insertion"),
            WorkbookChange::delete_sheet(id(2)),
        ],
        vec![WorkbookChange::delete_sheet(id(2)), insert()],
        vec![
            WorkbookChange::rename_defined_name(DefinedNameScope::Workbook, "Block", "Blk"),
            insert(),
            WorkbookChange::move_sheet(id(1), 1),
        ],
    ] {
        let unrestored = round_trip(changes.clone());
        assert!(unrestored.is_empty(), "{changes:?}: {unrestored:?}");
    }
}

#[test]
fn a_column_inserted_inside_a_table_is_undone_by_deleting_it() {
    let unrestored = round_trip(vec![
        WorkbookChange::insert_columns(id(1), Column::new(4).expect("D"), 1).expect("insertion"),
    ]);
    assert!(unrestored.is_empty(), "{unrestored:?}");
}
