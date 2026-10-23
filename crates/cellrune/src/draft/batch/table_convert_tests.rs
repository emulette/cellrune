use crate::workbook::{NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CalculationHints, CalculationOptions, CellAddress, CellContent, CellRange,
    CellValue, DateSystem, DefinedName, DefinedNameScope, EditBatch, FormulaCell, FormulaDialect,
    FormulaMetadata, FormulaText, NumberFormat, Provenance, SavedResult, Sheet, SheetId, SheetName,
    SheetVisibility, Table, TableColumn, TableFormula, TableId, TableName, ValidationError,
    WorkbookChange, WorkbookDraft, WorkbookSnapshot, WorkbookSource, calculate_workbook,
};

fn id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn sheet(sheet_id: u32, name: &str, cells: &[(&str, &str)]) -> Sheet {
    let mut sheet = Sheet::new(
        id(sheet_id),
        SheetName::new(name).expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (cell, input) in cells {
        let content = match input.strip_prefix('=') {
            Some(formula) => CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(formula).expect("formula"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
            None => match input.parse::<f64>() {
                Ok(number) => CellContent::Literal(CellValue::number(number).expect("number")),
                Err(_) => CellContent::Literal(CellValue::Text((*input).to_owned())),
            },
        };
        sheet.upsert_cell(address(cell), content, NumberFormat::default());
    }
    sheet
}

/// The Excel probe: `Things` at C1:E5 on `S2` with columns key, val, and dbl, whose calculated
/// `dbl` reads `[@val]`, and a totals row; `S1` and a name read it.
fn workbook() -> WorkbookDraft {
    let s1 = sheet(
        1,
        "S1",
        &[
            ("A1", "=SUM(Things[val])"),
            ("A2", "=SUM(Things[[#Data],[val]:[dbl]])"),
            ("A3", "=Things[[#Totals],[val]]"),
            ("A4", "=ROWS(Things[#All])"),
            ("A5", "=SUM(ThingsVal)"),
        ],
    );
    let mut s2 = sheet(
        2,
        "S2",
        &[
            ("C1", "key"),
            ("D1", "val"),
            ("E1", "dbl"),
            ("C2", "a"),
            ("D2", "1"),
            ("E2", "=[@val]*2"),
            ("C3", "b"),
            ("D3", "2"),
            ("E3", "=[@val]*2"),
            ("C4", "c"),
            ("D4", "3"),
            ("E4", "=[@val]*2"),
            ("D5", "=SUBTOTAL(109,Things[val])"),
        ],
    );
    let calculated = TableFormula::new(FormulaText::from_xlsx("[@val]*2").expect("dbl"), false);
    s2.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Things").expect("name"),
            TableName::new("Things").expect("display name"),
            CellRange::new(address("C1"), address("E5")).expect("range"),
            1,
            1,
            vec![
                TableColumn::new(1, "key", None).expect("key"),
                TableColumn::new(2, "val", None).expect("val"),
                TableColumn::new(3, "dbl", None)
                    .expect("dbl")
                    .with_metadata(None, Some(calculated), None),
            ],
        )
        .expect("table"),
    ]);
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets: vec![s1, s2],
        non_worksheet_tabs: NonWorksheetTabs::default(),
        defined_names: vec![
            DefinedName::new(
                "ThingsVal",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("Things[val]").expect("name"),
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
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn formula(draft: &WorkbookDraft, sheet_id: u32, cell: &str) -> String {
    match draft
        .workbook()
        .sheet_by_id(id(sheet_id))
        .expect("sheet")
        .cell(address(cell))
        .expect("cell")
        .content()
    {
        CellContent::Formula(formula) => formula.text().expect("text").as_str().to_owned(),
        CellContent::Literal(_) => panic!("{cell} holds a formula"),
    }
}

fn results(draft: &WorkbookDraft) -> Vec<(CalculationCellId, crate::CalculationCellResult)> {
    calculate_workbook(draft.workbook(), CalculationOptions::default())
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect()
}

#[test]
fn converting_a_table_writes_its_references_as_absolute_references_as_excel_saves_them() {
    let mut draft = workbook();
    let before = results(&draft);
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::convert_table_to_range(
            TableId::new(1).expect("table ID"),
        )]))
        .expect("convert Things");
    assert!(receipt.topology_changed());
    assert!(draft.workbook().table("Things").is_none());
    for (sheet_id, cell, expected) in [
        (1, "A1", "SUM('S2'!$D$2:$D$4)"),
        (1, "A2", "SUM('S2'!$D$2:$E$4)"),
        (1, "A3", "'S2'!$D$5"),
        (1, "A4", "ROWS('S2'!$C$1:$E$5)"),
        (2, "E2", "'S2'!$D2*2"),
        (2, "E4", "'S2'!$D4*2"),
        (2, "D5", "SUBTOTAL(109,'S2'!$D$2:$D$4)"),
    ] {
        assert_eq!(
            formula(&draft, sheet_id, cell),
            expected,
            "S{sheet_id}!{cell}"
        );
    }
    assert_eq!(
        draft.workbook().defined_names()[0].formula().as_str(),
        "'S2'!$D$2:$D$4"
    );
    // The cells keep their values, so every result is the same.
    assert_eq!(results(&draft), before);
}

#[test]
fn converting_an_unknown_table_fails() {
    let mut draft = workbook();
    assert_eq!(
        draft.apply_changes(EditBatch::new([WorkbookChange::convert_table_to_range(
            TableId::new(7).expect("table ID"),
        )])),
        Err(ValidationError::UnknownTableId { value: 7 })
    );
}
