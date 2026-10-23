use crate::workbook::{NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationHints, CellAddress, CellContent, CellRange, CellValue, DateSystem, EditBatch,
    FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, NumberFormat, Provenance,
    SavedResult, Sheet, SheetId, SheetName, SheetVisibility, Table, TableColumn, TableId,
    TableName, ValidationError, WorkbookChange, WorkbookDraft, WorkbookSnapshot, WorkbookSource,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(address(start), address(end)).expect("range")
}

const TABLE: fn() -> TableId = || TableId::new(1).expect("table ID");

/// The Excel probe: `T` at A1:C4 with columns `a`, `b`, and `c`, read from column H.
fn draft() -> WorkbookDraft {
    let mut sheet = Sheet::new(
        SheetId::new(1).expect("sheet ID"),
        SheetName::new("S1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let mut put = |cell: &str, content: CellContent| {
        sheet.upsert_cell(address(cell), content, NumberFormat::default());
    };
    for (cell, text) in [("A1", "a"), ("B1", "b"), ("C1", "c"), ("D1", "d")] {
        put(cell, CellContent::Literal(CellValue::Text(text.to_owned())));
    }
    for row in 2..=4 {
        for column in ["A", "B", "C"] {
            put(
                &format!("{column}{row}"),
                CellContent::Literal(CellValue::number(f64::from(row)).expect("number")),
            );
        }
    }
    for (cell, formula) in [
        ("H1", "SUM(T[c])"),
        ("H2", "SUM(T[[a]:[c]])"),
        ("H3", "SUM(T[[a]:[b]])"),
        ("H4", "T[[#Headers],[c]]"),
        ("H5", "SUM(T[a])"),
    ] {
        put(
            cell,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(formula).expect("formula"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        );
    }
    sheet.set_tables(vec![
        Table::new(
            TABLE(),
            TableName::new("T").expect("name"),
            TableName::new("T").expect("display name"),
            range("A1:C4"),
            1,
            0,
            ["a", "b", "c"]
                .into_iter()
                .zip(1..)
                .map(|(name, id)| TableColumn::new(id, name, None).expect("column"))
                .collect(),
        )
        .expect("table"),
    ]);
    WorkbookDraft::from_snapshot_for_test(
        WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
            sheets: vec![sheet],
            non_worksheet_tabs: NonWorksheetTabs::default(),
            defined_names: Vec::new(),
            diagnostics: Vec::new(),
            date_system: DateSystem::Excel1900,
            calculation_hints: CalculationHints::default(),
            source: WorkbookSource::default(),
            provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
        })
        .expect("workbook"),
    )
}

fn formulas(draft: &WorkbookDraft) -> Vec<String> {
    let sheet = &draft.workbook().sheets()[0];
    ["H1", "H2", "H3", "H4", "H5"]
        .into_iter()
        .map(
            |cell| match sheet.cell(address(cell)).expect("cell").content() {
                CellContent::Formula(formula) => formula.text().expect("text").as_str().to_owned(),
                CellContent::Literal(_) => panic!("{cell} holds a formula"),
            },
        )
        .collect()
}

fn columns(draft: &WorkbookDraft) -> (CellRange, Vec<String>) {
    let table = draft.workbook().table("T").expect("T");
    (
        table.range(),
        table
            .columns()
            .iter()
            .map(|column| column.name().to_owned())
            .collect(),
    )
}

fn set(draft: &mut WorkbookDraft, target: &str) -> Result<crate::EditReceipt, ValidationError> {
    draft.apply_changes(EditBatch::new([WorkbookChange::set_table_range(
        TABLE(),
        range(target),
    )]))
}

#[test]
fn removing_a_table_column_turns_references_that_name_it_into_ref_errors() {
    // Excel 16.114: shrinking `T` past `c` turns `T[c]` and `T[[a]:[c]]`, which end on it, into
    // `#REF!`.
    let mut draft = draft();
    let receipt = set(&mut draft, "A1:B4").expect("shrink");
    assert!(receipt.topology_changed());
    assert_eq!(receipt.changed_table_ids(), [TABLE()]);
    assert_eq!(
        formulas(&draft),
        [
            "SUM(#REF!)",
            "SUM(#REF!)",
            "SUM(T[[a]:[b]])",
            "#REF!",
            "SUM(T[a])"
        ]
    );
    assert_eq!(
        columns(&draft),
        (range("A1:B4"), vec!["a".to_owned(), "b".to_owned()])
    );
    // The removed column's cells keep their content.
    assert!(draft.workbook().sheets()[0].cell(address("C2")).is_some());

    let mut draft = self::draft();
    set(&mut draft, "B1:C4").expect("shrink from the left");
    assert_eq!(formulas(&draft)[4], "SUM(#REF!)");
    assert_eq!(
        columns(&draft),
        (range("B1:C4"), vec!["b".to_owned(), "c".to_owned()])
    );
}

#[test]
fn a_table_grows_by_rows_and_columns_at_once() {
    let mut draft = draft();
    set(&mut draft, "A1:D6").expect("grow");
    assert_eq!(
        columns(&draft),
        (
            range("A1:D6"),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned()
            ]
        )
    );
    // Rows and a column change together: drop `a`, add `d`, and lose a data row.
    let mut draft = self::draft();
    set(&mut draft, "B1:D3").expect("shift");
    assert_eq!(
        columns(&draft),
        (
            range("B1:D3"),
            vec!["b".to_owned(), "c".to_owned(), "d".to_owned()]
        )
    );
}

#[test]
fn a_range_that_moves_the_header_or_leaves_the_table_fails() {
    let mut draft = draft();
    let revision = draft.semantic_revision();
    for target in ["A2:C4", "E1:F4", "A1:C1"] {
        assert_eq!(
            set(&mut draft, target).map(|_| ()),
            Err(ValidationError::TableRangeInvalid),
            "{target}"
        );
    }
    assert_eq!(draft.semantic_revision(), revision);
    let receipt = set(&mut draft, "A1:C4").expect("unchanged");
    assert_eq!(receipt.result_revision(), revision);
}
