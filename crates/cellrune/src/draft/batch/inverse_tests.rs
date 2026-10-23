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

/// Applies `changes`, then the inverse its receipt carries, and checks that the cells, names,
/// tables, row heights, and cell formats are back. Returns what the inverse could not restore.
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
    for (before, after) in original
        .workbook()
        .sheets()
        .iter()
        .zip(draft.workbook().sheets())
    {
        assert_eq!(
            before.cells().collect::<Vec<_>>(),
            after.cells().collect::<Vec<_>>(),
            "{}",
            before.name().as_str()
        );
        assert!(
            before
                .tables()
                .iter()
                .zip(after.tables())
                .all(|(before, after)| before.semantic_eq(after))
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
    assert_eq!(
        original.workbook().defined_names(),
        draft.workbook().defined_names()
    );
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
