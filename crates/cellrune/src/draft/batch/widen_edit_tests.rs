use crate::{
    CalculationHints, CellAddress, CellContent, CellRange, CellValue, DateSystem, DefinedName,
    DefinedNameScope, EditBatch, FiniteNumber, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, NumberFormat, Provenance, SavedResult, Sheet, SheetAutoFilter, SheetId, SheetName,
    SheetVisibility, Table, TableColumn, TableId, TableName, TableType, ValidationError,
    WorkbookChange, WorkbookDraft, WorkbookSnapshot, WorkbookSource,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid cell address")
}

fn range(start: &str, end: &str) -> CellRange {
    CellRange::new(address(start), address(end)).expect("valid range")
}

fn text(value: &str) -> CellContent {
    CellContent::Literal(CellValue::Text(value.to_owned()))
}

fn number(value: f64) -> CellContent {
    CellContent::Literal(CellValue::Number(
        FiniteNumber::new(value).expect("finite number"),
    ))
}

fn formula(text: &str, metadata: FormulaMetadata) -> CellContent {
    CellContent::Formula(FormulaCell::new(
        FormulaDialect::ExcelA1,
        FormulaText::from_xlsx(text).expect("formula"),
        SavedResult::Missing,
        metadata,
    ))
}

fn table(id: u32, name: &str, table_range: CellRange, totals_row_count: u32) -> Table {
    let columns = (0..table_range.width())
        .map(|index| TableColumn::new(index + 1, format!("{name}{index}"), None).expect("column"))
        .collect();
    let table_name = TableName::new(name).expect("table name");
    Table::new(
        TableId::new(id).expect("table ID"),
        table_name.clone(),
        table_name,
        table_range,
        1,
        totals_row_count,
        columns,
    )
    .expect("table")
}

/// Sheet 1 holds table 1 on `B1:C3` with columns `Item` and `Amount`, the text header `Margin`
/// in D1, and whatever `configure` adds.
fn draft(configure: impl FnOnce(&mut Sheet)) -> WorkbookDraft {
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Data").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let sales = Table::new(
        TableId::new(1).expect("table ID"),
        TableName::new("Sales").expect("table name"),
        TableName::new("Sales").expect("display name"),
        range("B1", "C3"),
        1,
        0,
        vec![
            TableColumn::new(1, "Item", None).expect("column"),
            TableColumn::new(2, "Amount", None).expect("column"),
        ],
    )
    .expect("table");
    sheet.set_tables(vec![sales]);
    for (cell, value) in [("B1", "Item"), ("C1", "Amount"), ("D1", "Margin")] {
        sheet.upsert_cell(address(cell), text(value), NumberFormat::default());
    }
    configure(&mut sheet);
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(crate::ProviderIdentity::writer(), None),
    )
    .expect("snapshot");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn put(sheet: &mut Sheet, cell: &str, content: CellContent) {
    sheet.upsert_cell(address(cell), content, NumberFormat::default());
}

fn replace_table(sheet: &mut Sheet, table: Table) {
    let mut tables = sheet.tables().to_vec();
    tables[0] = table;
    sheet.set_tables(tables);
}

fn widen_table(end: &str) -> WorkbookChange {
    WorkbookChange::widen_table_columns(TableId::new(1).expect("table ID"), range("B1", end))
}

fn widen_filter(start: &str, end: &str) -> WorkbookChange {
    WorkbookChange::widen_sheet_auto_filter(SheetId::new(1).expect("sheet ID"), range(start, end))
}

fn filter_on(sheet: &mut Sheet, start: &str, end: &str) {
    sheet.set_auto_filter(Some(SheetAutoFilter::from_xlsx(range(start, end), vec![0])));
}

#[test]
fn column_widening_rejects_invalid_ranges_headers_and_overlaps_atomically() {
    let sheet_id = 1;
    let blocked = |sheet: &mut Sheet| {
        sheet.set_merged_ranges(vec![range("D2", "D3")]);
    };
    let cases: Vec<(WorkbookDraft, WorkbookChange, ValidationError)> = vec![
        (
            draft(|_| {}),
            WorkbookChange::widen_table_columns(
                TableId::new(1).expect("table ID"),
                range("B1", "D4"),
            ),
            ValidationError::ColumnWideningRangeInvalid,
        ),
        (
            draft(|_| {}),
            WorkbookChange::widen_table_columns(
                TableId::new(1).expect("table ID"),
                range("C1", "D3"),
            ),
            ValidationError::ColumnWideningRangeInvalid,
        ),
        (
            draft(|sheet| {
                sheet.remove_cell_deferred(address("D1"));
                sheet.finish_deferred_cell_edits();
            }),
            widen_table("D3"),
            ValidationError::TableColumnNameEmpty,
        ),
        (
            draft(|sheet| put(sheet, "D1", number(5.0))),
            widen_table("D3"),
            ValidationError::TableHeaderRequiresText {
                table_id: 1,
                row: 1,
                column: 4,
            },
        ),
        (
            draft(|sheet| put(sheet, "D1", formula("\"Margin\"", FormulaMetadata::Normal))),
            widen_table("D3"),
            ValidationError::TableHeaderRequiresText {
                table_id: 1,
                row: 1,
                column: 4,
            },
        ),
        (
            draft(|sheet| put(sheet, "D1", text("item"))),
            widen_table("D3"),
            ValidationError::DuplicateTableColumnName {
                name: "item".to_owned(),
            },
        ),
        (
            draft(|sheet| {
                replace_table(sheet, table(1, "Sales", range("B1", "C3"), 1));
                put(sheet, "D3", number(1.0));
            }),
            widen_table("D3"),
            ValidationError::TableTotalsCellOccupied {
                table_id: 1,
                row: 3,
                column: 4,
            },
        ),
        (
            draft(|sheet| {
                let headerless = Table::new(
                    TableId::new(1).expect("table ID"),
                    TableName::new("Sales").expect("table name"),
                    TableName::new("Sales").expect("display name"),
                    range("B1", "C3"),
                    0,
                    0,
                    vec![
                        TableColumn::new(1, "Item", None).expect("column"),
                        TableColumn::new(2, "Amount", None).expect("column"),
                    ],
                )
                .expect("header-less table");
                replace_table(sheet, headerless);
            }),
            widen_table("D3"),
            ValidationError::UnsupportedTableAuthoringMetadata { table_id: 1 },
        ),
        (
            draft(|sheet| {
                let query = table(1, "Sales", range("B1", "C3"), 0)
                    .try_with_metadata(TableType::QueryTable, true, None, None, None, None)
                    .expect("query table");
                replace_table(sheet, query);
            }),
            widen_table("D3"),
            ValidationError::UnsupportedTableAuthoringMetadata { table_id: 1 },
        ),
        (
            draft(|sheet| {
                let mut tables = sheet.tables().to_vec();
                tables.push(table(2, "Other", range("E1", "F3"), 0));
                sheet.set_tables(tables);
            }),
            widen_table("E3"),
            ValidationError::OverlappingTables {
                sheet_id,
                first_table_id: 1,
                second_table_id: 2,
            },
        ),
        (
            draft(blocked),
            widen_table("D3"),
            ValidationError::ColumnWideningOverlap {
                sheet_id,
                row: 2,
                column: 4,
            },
        ),
        (
            draft(|sheet| {
                put(
                    sheet,
                    "D2",
                    formula(
                        "SEQUENCE(2)",
                        FormulaMetadata::Array {
                            range: range("D2", "D3"),
                            always_calculate: false,
                        },
                    ),
                );
            }),
            widen_table("D3"),
            ValidationError::ColumnWideningOverlap {
                sheet_id,
                row: 2,
                column: 4,
            },
        ),
        (
            draft(|sheet| {
                put(
                    sheet,
                    "D2",
                    formula(
                        "SEQUENCE(2)",
                        FormulaMetadata::DynamicArray {
                            range: Some(range("D2", "D3")),
                            always_calculate: false,
                        },
                    ),
                );
            }),
            widen_table("D3"),
            ValidationError::ColumnWideningOverlap {
                sheet_id,
                row: 2,
                column: 4,
            },
        ),
        (
            draft(|sheet| filter_on(sheet, "E1", "F3")),
            widen_table("E3"),
            ValidationError::SheetAutoFilterOverlapsTable {
                sheet_id,
                table_id: 1,
            },
        ),
        (
            draft(|sheet| filter_on(sheet, "E1", "F3")),
            widen_filter("C1", "F3"),
            ValidationError::SheetAutoFilterOverlapsTable {
                sheet_id,
                table_id: 1,
            },
        ),
        (
            draft(|sheet| filter_on(sheet, "E1", "F3")),
            widen_filter("E1", "G4"),
            ValidationError::ColumnWideningRangeInvalid,
        ),
        (
            draft(|sheet| filter_on(sheet, "E1", "F3")),
            widen_filter("E1", "E3"),
            ValidationError::ColumnWideningRangeInvalid,
        ),
        (
            draft(|sheet| {
                filter_on(sheet, "E1", "F3");
                sheet.set_merged_ranges(vec![range("G2", "H2")]);
            }),
            widen_filter("E1", "H3"),
            ValidationError::ColumnWideningOverlap {
                sheet_id,
                row: 2,
                column: 7,
            },
        ),
        (
            draft(|_| {}),
            widen_filter("E1", "F3"),
            ValidationError::SheetAutoFilterNotFound { sheet_id },
        ),
    ];

    for (mut draft, change, expected) in cases {
        let revision = draft.semantic_revision();
        let fingerprint = draft.workbook().fingerprint();
        assert_eq!(
            draft
                .apply_changes(EditBatch::new([change]))
                .expect_err("invalid widening"),
            expected
        );
        assert_eq!(draft.semantic_revision(), revision);
        assert_eq!(draft.workbook().fingerprint(), fingerprint);
    }
}

#[test]
fn widening_a_totals_table_requires_only_an_empty_totals_cell() {
    let mut draft = draft(|sheet| replace_table(sheet, table(1, "Sales", range("B1", "C3"), 1)));
    let receipt = draft
        .apply_changes(EditBatch::new([widen_table("D3")]))
        .expect("an empty totals cell admits the new column");
    assert_eq!(
        receipt.changed_table_ids(),
        [TableId::new(1).expect("table ID")]
    );
    let table = &draft.workbook().sheets()[0].tables()[0];
    assert_eq!(table.range(), range("B1", "D3"));
    let added = &table.columns()[2];
    assert_eq!((added.id(), added.name()), (3, "Margin"));
    assert!(added.calculated_column_formula().is_none());
    assert!(added.totals_row_function().is_none());
}

#[test]
fn widening_to_the_current_range_changes_nothing() {
    let mut draft = draft(|sheet| filter_on(sheet, "E1", "F3"));
    let revision = draft.semantic_revision();
    let receipt = draft
        .apply_changes(EditBatch::new([
            widen_table("C3"),
            widen_filter("E1", "F3"),
        ]))
        .expect("same ranges");
    assert_eq!(receipt.result_revision(), revision);
    assert!(receipt.changed_table_ids().is_empty());
}

#[test]
fn widening_a_sheet_auto_filter_moves_only_its_own_filter_database_name() {
    let mut draft = draft(|sheet| filter_on(sheet, "E1", "F3"));
    let sheet = SheetId::new(1).expect("sheet ID");
    let name = |scope, formula: &str| {
        DefinedName::new(
            "_xlnm._FilterDatabase",
            scope,
            FormulaText::from_xlsx(formula).expect("formula"),
            true,
        )
        .expect("defined name")
    };
    draft
        .set_defined_name(name(DefinedNameScope::Sheet(sheet), "Data!$E$1:$F$3"))
        .expect("sheet filter database");
    draft
        .set_defined_name(name(DefinedNameScope::Workbook, "Data!$E$1:$F$3"))
        .expect("workbook-scoped look-alike");

    let receipt = draft
        .apply_changes(EditBatch::new([widen_filter("E1", "H3")]))
        .expect("widen filter");

    assert!(receipt.topology_changed());
    let formulas = draft
        .workbook()
        .defined_names()
        .iter()
        .map(|name| (name.scope(), name.formula().as_str(), name.hidden()))
        .collect::<Vec<_>>();
    assert_eq!(
        formulas,
        [
            (DefinedNameScope::Sheet(sheet), "Data!$E$1:$H$3", true),
            (DefinedNameScope::Workbook, "Data!$E$1:$F$3", true),
        ]
    );
    let filter = draft.workbook().sheets()[0]
        .auto_filter()
        .expect("sheet auto-filter");
    assert_eq!(filter.range(), range("E1", "H3"));
    assert_eq!(filter.filter_column_ids(), [0]);
}
