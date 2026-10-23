use crate::workbook::{NonWorksheetTab, NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope, EditBatch,
    ExcelError, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, FrozenPane,
    NumberFormat, Provenance, SavedResult, Sheet, SheetId, SheetName, SheetVisibility, Table,
    TableColumn, TableFormula, TableId, TableName, ValidationError, WorkbookChange, WorkbookDraft,
    WorkbookSnapshot, WorkbookSource, calculate_workbook,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
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

fn name(name: &str, scope: Option<u32>, formula: &str) -> DefinedName {
    DefinedName::new(
        name,
        scope.map_or(DefinedNameScope::Workbook, |sheet_id| {
            DefinedNameScope::Sheet(id(sheet_id))
        }),
        FormulaText::from_xlsx(formula).expect("name formula"),
        false,
    )
    .expect("defined name")
}

fn table(table_id: u32, display_name: &str, range: &str, columns: Vec<TableColumn>) -> Table {
    let (start, end) = range.split_once(':').expect("table range");
    Table::new(
        TableId::new(table_id).expect("table ID"),
        TableName::new(display_name).expect("table name"),
        TableName::new(display_name).expect("display name"),
        CellRange::new(address(start), address(end)).expect("range"),
        1,
        0,
        columns,
    )
    .expect("table")
}

/// Sheets `S1` to `S4` as in the Excel probe: `S2` holds the `Things` table, `S3` the `Calc`
/// table whose calculated column reads `S2`, and the names cover each scope.
fn probe_workbook(non_worksheet_tabs: NonWorksheetTabs) -> WorkbookDraft {
    let mut s1 = sheet(
        1,
        "S1",
        &[
            ("A1", "1"),
            ("B1", "='S2'!A1"),
            ("B2", "=SUM('S1:S3'!A1)"),
            ("B3", "=SUM('S2:S4'!A1)"),
            ("B4", "=SUM('S2:S3'!A1)"),
            ("B5", "=SUM(Wb_Used)"),
            ("B6", "=SUM(Things[val])"),
            ("B7", "='S3'!LocalS3"),
            ("B8", "='S2'!LocalS2"),
            ("B9", "=Wb_S3"),
            ("B10", "='S3'!A1"),
            ("B11", "=INDIRECT(\"'S2'!A1\")"),
            ("B12", "=SUM('S2'!A1:A3)"),
            ("B13", "='S2'!A1+'S3'!A1"),
            ("B14", "=LocalS1"),
        ],
    );
    s1.set_tables(Vec::new());
    let mut s2 = sheet(
        2,
        "S2",
        &[
            ("A1", "2"),
            ("A2", "20"),
            ("A3", "200"),
            ("C1", "key"),
            ("D1", "val"),
            ("C2", "a"),
            ("D2", "1"),
            ("C3", "b"),
            ("D3", "2"),
        ],
    );
    s2.set_tables(vec![table(
        1,
        "Things",
        "C1:D3",
        vec![
            TableColumn::new(1, "key", None).expect("key"),
            TableColumn::new(2, "val", None).expect("val"),
        ],
    )]);
    let mut s3 = sheet(
        3,
        "S3",
        &[
            ("A1", "3"),
            ("B1", "='S2'!A1"),
            ("D1", "x"),
            ("D2", "='S2'!$A$1+1"),
        ],
    );
    s3.set_tables(vec![table(
        2,
        "Calc",
        "D1:D2",
        vec![TableColumn::new(1, "x", None).expect("x").with_metadata(
            None,
            Some(TableFormula::new(
                FormulaText::from_xlsx("'S2'!$A$1+1").expect("calculated"),
                false,
            )),
            None,
        )],
    )]);
    let s4 = sheet(
        4,
        "S4",
        &[("A1", "4"), ("B1", "=SUM('S1:S3'!A1)"), ("B2", "='S1'!A1")],
    );
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets: vec![s1, s2, s3, s4],
        non_worksheet_tabs,
        defined_names: vec![
            name("LocalS1", Some(1), "'S2'!$A$1"),
            name("LocalS2", Some(2), "'S2'!$A$1"),
            name("LocalS3", Some(3), "'S3'!$A$1"),
            name("LocalS4", Some(4), "'S4'!$A$1"),
            name("Unused2", Some(2), "'S2'!$A$2"),
            name("Wb_S2", None, "'S2'!$A$1"),
            name("Wb_S2S4", None, "'S2:S4'!$A$1"),
            name("Wb_S3", None, "'S3'!$A$1"),
            name("Wb_Used", None, "'S2'!$A$1:$A$3"),
            name("Wb_Things", None, "Things[val]"),
        ],
        diagnostics: Vec::new(),
        date_system: DateSystem::Excel1900,
        calculation_hints: CalculationHints::default(),
        source: WorkbookSource::default(),
        provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
    })
    .expect("probe workbook");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn formula_text(workbook: &WorkbookSnapshot, sheet: &str, cell: &str) -> String {
    let CellContent::Formula(formula) = workbook
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell(address(cell))
        .expect("cell")
        .content()
    else {
        panic!("{sheet}!{cell} holds a formula");
    };
    formula.text().expect("formula text").as_str().to_owned()
}

fn defined_names(workbook: &WorkbookSnapshot) -> Vec<(String, Option<String>, String)> {
    workbook
        .defined_names()
        .iter()
        .map(|defined| {
            let scope = match defined.scope() {
                DefinedNameScope::Workbook => None,
                DefinedNameScope::Sheet(sheet_id) => Some(
                    workbook
                        .sheet_by_id(sheet_id)
                        .expect("scope sheet")
                        .name()
                        .as_str()
                        .to_owned(),
                ),
            };
            (
                defined.name().to_owned(),
                scope,
                defined.formula().as_str().to_owned(),
            )
        })
        .collect()
}

fn owned(names: &[(&str, Option<&str>, &str)]) -> Vec<(String, Option<String>, String)> {
    names
        .iter()
        .map(|(name, scope, formula)| {
            (
                (*name).to_owned(),
                scope.map(str::to_owned),
                (*formula).to_owned(),
            )
        })
        .collect()
}

#[test]
fn deleting_a_sheet_rewrites_formulas_names_and_table_formulas_as_excel_saves_them() {
    let mut draft = probe_workbook(NonWorksheetTabs::default());
    let base_revision = draft.semantic_revision();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::delete_sheet(id(2))]))
        .expect("delete S2");

    let workbook = draft.workbook();
    let names = workbook
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["S1", "S3", "S4"]);
    for (cell, expected) in [
        ("B1", "#REF!"),
        ("B2", "SUM('S1:S3'!A1)"),
        ("B3", "SUM('S3:S4'!A1)"),
        ("B4", "SUM('S3'!A1)"),
        ("B5", "SUM(Wb_Used)"),
        ("B6", "SUM(#REF!)"),
        ("B7", "'S3'!LocalS3"),
        ("B8", "#REF!"),
        ("B9", "Wb_S3"),
        ("B10", "'S3'!A1"),
        ("B11", "INDIRECT(\"'S2'!A1\")"),
        ("B12", "SUM(#REF!)"),
        ("B13", "#REF!+'S3'!A1"),
        ("B14", "LocalS1"),
    ] {
        assert_eq!(formula_text(workbook, "S1", cell), expected, "S1!{cell}");
    }
    assert_eq!(formula_text(workbook, "S3", "B1"), "#REF!");
    assert_eq!(formula_text(workbook, "S3", "D2"), "#REF!+1");
    let calc = workbook.table("Calc").expect("Calc table");
    assert_eq!(
        calc.columns()[0]
            .calculated_column_formula()
            .expect("calculated column")
            .text()
            .as_str(),
        "#REF!+1"
    );
    assert!(workbook.table("Things").is_none());
    assert_eq!(
        defined_names(workbook),
        owned(&[
            ("LocalS1", Some("S1"), "#REF!"),
            ("LocalS3", Some("S3"), "'S3'!$A$1"),
            ("LocalS4", Some("S4"), "'S4'!$A$1"),
            ("Wb_S2", None, "#REF!"),
            ("Wb_S2S4", None, "'S3:S4'!$A$1"),
            ("Wb_S3", None, "'S3'!$A$1"),
            ("Wb_Used", None, "#REF!"),
            ("Wb_Things", None, "#REF!"),
        ])
    );

    assert_eq!(receipt.base_revision(), base_revision);
    assert_eq!(receipt.result_revision(), base_revision + 1);
    assert!(receipt.topology_changed());
    assert_eq!(
        receipt.changed_table_ids(),
        [TableId::new(2).expect("Calc")]
    );
    assert!(
        receipt
            .changed_cells()
            .iter()
            .all(|cell| cell.sheet_id() != id(2))
    );
    assert!(
        receipt
            .calculation_changed_cells()
            .contains(&CalculationCellId::new(id(1), address("B13")))
    );

    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    let value = |sheet_id: u32, cell: &str| match calculation
        .cell(CalculationCellId::new(id(sheet_id), address(cell)))
    {
        Some(CalculationCellResult::Value(value)) => value.clone(),
        other => panic!("{cell}: {other:?}"),
    };
    let number = |value: f64| CellValue::number(value).expect("number");
    assert_eq!(value(1, "B2"), number(4.0));
    assert_eq!(value(1, "B3"), number(7.0));
    assert_eq!(value(1, "B13"), CellValue::Error(ExcelError::Reference));
    assert_eq!(value(1, "B14"), CellValue::Error(ExcelError::Reference));
    assert_eq!(value(4, "B1"), number(4.0));
}

#[test]
fn deleting_each_end_of_a_3d_reference_moves_it_toward_the_other_end() {
    for (deleted, s1_b2, s1_b3, s4_b1) in [
        (1, None, None, Some("SUM('S2:S3'!A1)")),
        (
            3,
            Some("SUM('S1:S2'!A1)"),
            Some("SUM('S2:S4'!A1)"),
            Some("SUM('S1:S2'!A1)"),
        ),
        (4, Some("SUM('S1:S3'!A1)"), Some("SUM('S2:S3'!A1)"), None),
    ] {
        let mut draft = probe_workbook(NonWorksheetTabs::default());
        draft.delete_sheet(id(deleted)).expect("delete");
        let workbook = draft.workbook();
        for (sheet, cell, expected) in [
            ("S1", "B2", s1_b2),
            ("S1", "B3", s1_b3),
            ("S4", "B1", s4_b1),
        ] {
            if let Some(expected) = expected {
                assert_eq!(
                    formula_text(workbook, sheet, cell),
                    expected,
                    "S{deleted}: {sheet}!{cell}"
                );
            }
        }
    }
}

#[test]
fn deleting_a_worksheet_moves_later_non_worksheet_tabs_and_3d_ends_cross_them() {
    // Tab order: S1, S2, Chart, S3, S4.
    let chart = NonWorksheetTab::new(2, id(9), SheetName::new("Chart").expect("chart name"));
    let mut draft = probe_workbook(NonWorksheetTabs::new(vec![chart]));
    draft.delete_sheet(id(1)).expect("delete S1");
    let workbook = draft.workbook();
    assert_eq!(workbook.sheet_tab_position(id(9)), Some(1));
    assert_eq!(workbook.sheet_tab_position(id(3)), Some(2));
    assert_eq!(formula_text(workbook, "S4", "B1"), "SUM('S2:S3'!A1)");

    let mut draft = probe_workbook(NonWorksheetTabs::new(vec![NonWorksheetTab::new(
        2,
        id(9),
        SheetName::new("Chart").expect("chart name"),
    )]));
    draft.delete_sheet(id(2)).expect("delete S2");
    // `'S2:S3'!A1` keeps the chartsheet between them as its first tab, as tab order dictates.
    assert_eq!(
        formula_text(draft.workbook(), "S1", "B4"),
        "SUM('Chart:S3'!A1)"
    );
    // A deleted ID is never reused, even when it was the highest.
    let added = draft
        .add_sheet(SheetName::new("Added").expect("sheet name"))
        .expect("add sheet");
    assert_eq!(added, id(10));
}

#[test]
fn deleting_the_last_visible_or_an_unknown_sheet_fails_atomically() {
    let mut draft = probe_workbook(NonWorksheetTabs::default());
    for sheet_id in [2, 3, 4] {
        draft
            .set_sheet_visibility(id(sheet_id), SheetVisibility::Hidden)
            .expect("hide");
    }
    let revision = draft.semantic_revision();
    assert_eq!(
        draft.delete_sheet(id(1)),
        Err(ValidationError::LastVisibleSheet)
    );
    assert_eq!(
        draft.delete_sheet(id(7)),
        Err(ValidationError::UnknownSheetId { value: 7 })
    );
    // A later failure undoes an earlier deletion in the same batch.
    assert_eq!(
        draft.apply_changes(EditBatch::new([
            WorkbookChange::delete_sheet(id(2)),
            WorkbookChange::set_cell_value(id(2), address("A1"), CellValue::Blank),
        ])),
        Err(ValidationError::UnknownSheetId { value: 2 })
    );
    assert_eq!(draft.semantic_revision(), revision);
    assert_eq!(draft.workbook().sheets().len(), 4);

    draft
        .delete_sheet(id(2))
        .expect("a hidden sheet can be deleted");
    assert_eq!(draft.workbook().sheets().len(), 3);
}

#[test]
fn deleting_a_sheet_drops_what_the_draft_recorded_for_it() {
    let mut draft = probe_workbook(NonWorksheetTabs::default());
    draft
        .set_frozen_pane(id(2), FrozenPane::new(1, 0).expect("pane"))
        .expect("freeze");
    let receipt = draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_value(
                id(2),
                address("E5"),
                CellValue::Text("gone".to_owned()),
            ),
            WorkbookChange::add_sheet(SheetName::new("Added").expect("sheet name")),
            WorkbookChange::delete_sheet(id(2)),
            WorkbookChange::delete_sheet(id(5)),
        ]))
        .expect("delete edited and added sheets");

    assert_eq!(receipt.created_sheet_ids(), [id(5)]);
    assert!(
        receipt
            .changed_cells()
            .iter()
            .chain(receipt.calculation_changed_cells())
            .all(|cell| cell.sheet_id() != id(2) && cell.sheet_id() != id(5))
    );
    assert!(draft.presentation().frozen_pane(id(2)).is_none());
    assert!(draft.cell_mutations().sheet_keys(id(2)).next().is_none());
    assert!(draft.presentation_sheet_mutations().is_empty());
    assert!(draft.added_sheets().is_empty());
    assert!(
        draft
            .changed_table_ids()
            .iter()
            .all(|table_id| *table_id != TableId::new(1).expect("Things"))
    );
}
