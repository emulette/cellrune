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

/// `S1`, `S2` with the `Things` table (calculated `dbl`), and `S3`; `S2`'s formulas read
/// itself, `S1`, a 3-D span, and its table.
fn workbook() -> WorkbookDraft {
    let s1 = sheet(1, "S1", &[("A1", "1")]);
    let mut s2 = sheet(
        2,
        "S2",
        &[
            ("A1", "2"),
            ("A2", "='S2'!A1*2"),
            ("A3", "=S1!A1+'S2'!A1"),
            ("A4", "=SUM('S1:S3'!A1)"),
            ("A5", "=SUM(Things[val])"),
            ("A6", "=Local*1"),
            ("C1", "val"),
            ("D1", "dbl"),
            ("C2", "5"),
            ("D2", "=[@val]*2"),
        ],
    );
    s2.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Things").expect("name"),
            TableName::new("Things").expect("display name"),
            CellRange::new(address("C1"), address("D2")).expect("range"),
            1,
            0,
            vec![
                TableColumn::new(1, "val", None).expect("val"),
                TableColumn::new(2, "dbl", None)
                    .expect("dbl")
                    .with_metadata(
                        None,
                        Some(TableFormula::new(
                            FormulaText::from_xlsx("[@val]*2").expect("calculated"),
                            false,
                        )),
                        None,
                    ),
            ],
        )
        .expect("table"),
    ]);
    let s3 = sheet(3, "S3", &[("A1", "3")]);
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets: vec![s1, s2, s3],
        non_worksheet_tabs: NonWorksheetTabs::default(),
        defined_names: vec![
            name("Local", Some(2), "'S2'!$A$1"),
            name("Here", None, "'S2'!$A$1:$A$2"),
            name("Elsewhere", None, "'S1'!$A$1"),
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

fn formula(draft: &WorkbookDraft, sheet: &str, cell: &str) -> String {
    match draft
        .workbook()
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell(address(cell))
        .expect("cell")
        .content()
    {
        CellContent::Formula(formula) => formula.text().expect("text").as_str().to_owned(),
        CellContent::Literal(_) => panic!("{cell} holds a formula"),
    }
}

#[test]
fn a_copied_sheet_retargets_its_own_references_tables_and_names_as_excel_saves_them() {
    let mut draft = workbook();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::copy_sheet(id(2), 2)]))
        .expect("copy S2");
    assert_eq!(receipt.created_sheet_ids(), [id(4)]);
    assert!(receipt.topology_changed());
    let order = draft
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(order, ["S1", "S2", "S2 (2)", "S3"]);
    // Excel 16.114: self-references name the copy; other sheets and 3-D spans keep their text.
    for (cell, expected) in [
        ("A2", "'S2 (2)'!A1*2"),
        ("A3", "S1!A1+'S2 (2)'!A1"),
        ("A4", "SUM('S1:S3'!A1)"),
        ("A5", "SUM(Things3[val])"),
        ("A6", "Local*1"),
        ("D2", "[@val]*2"),
    ] {
        assert_eq!(formula(&draft, "S2 (2)", cell), expected, "{cell}");
    }
    assert_eq!(formula(&draft, "S2", "A2"), "'S2'!A1*2");
    // One table before the copy: `Things` becomes `Things3` with a new ID.
    let copied = draft.workbook().table("Things3").expect("Things3");
    assert_eq!(copied.id(), TableId::new(2).expect("table ID"));
    assert_eq!(
        receipt.changed_table_ids(),
        [TableId::new(2).expect("table ID")]
    );
    // The scoped name is copied, and the workbook name reading S2 becomes a name of the copy.
    let names = draft
        .workbook()
        .defined_names()
        .iter()
        .filter(|defined| defined.scope() == DefinedNameScope::Sheet(id(4)))
        .map(|defined| {
            (
                defined.name().to_owned(),
                defined.formula().as_str().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            ("Local".to_owned(), "'S2 (2)'!$A$1".to_owned()),
            ("Here".to_owned(), "'S2 (2)'!$A$1:$A$2".to_owned()),
        ]
    );
    // The 3-D span now holds the copy too.
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(
        calculation.cell(CalculationCellId::new(id(4), address("A4"))),
        Some(&crate::CalculationCellResult::Value(
            CellValue::number(8.0).expect("number")
        ))
    );

    // A second copy takes the next free number, and its table counts two tables before it.
    draft.copy_sheet(id(2), 0).expect("copy S2 again");
    assert_eq!(draft.workbook().sheets()[0].name().as_str(), "S2 (3)");
    assert!(draft.workbook().table("Things4").is_some());
}

#[test]
fn copy_names_count_up_and_fit_the_length_limit() {
    let taken = ["Data (2)", "data (3)"];
    let name = SheetName::new("Data").expect("name");
    assert_eq!(
        name.copy_name(|candidate| taken
            .iter()
            .any(|taken| taken.eq_ignore_ascii_case(candidate)))
            .as_str(),
        "Data (4)"
    );
    let long = SheetName::new("A".repeat(31)).expect("name");
    assert_eq!(
        long.copy_name(|_| false).as_str(),
        format!("{} (2)", "A".repeat(27))
    );
}

#[test]
fn copy_names_count_on_from_a_number_in_parentheses_as_excel_does() {
    // Excel 16.114 (excel-measure G and G2): the copy of each source name, then of the same
    // source once the first copy exists.
    for (source, first, second) in [
        ("Data (2)", "Data (3)", "Data (4)"),
        ("Data (9)", "Data (10)", "Data (11)"),
        ("Data(2)", "Data(3)", "Data(4)"),
        ("Data (x)", "Data (x) (2)", "Data (x) (3)"),
        ("Data (2) (2)", "Data (2) (3)", "Data (2) (4)"),
        ("Data (01)", "Data (2)", "Data (3)"),
        ("Data (0)", "Data (1)", "Data (2)"),
        ("(2)", "(3)", "(4)"),
        ("Data ( 2)", "Data (3)", "Data (4)"),
        (
            "Data (2147483648)",
            "Data (2147483648) (2)",
            "Data (2147483648) (3)",
        ),
        (
            "ABCDEFGHIJKLMNOPQRSTUVWXYZA (9)",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ (10)",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ (11)",
        ),
    ] {
        let name = SheetName::new(source).expect("name");
        let copy = name.copy_name(|candidate| candidate == source);
        assert_eq!(copy.as_str(), first, "{source}");
        assert_eq!(
            name.copy_name(|candidate| candidate == source || candidate == first)
                .as_str(),
            second,
            "{source}"
        );
    }
}

#[test]
fn a_copy_past_the_tab_order_or_of_an_unknown_sheet_fails() {
    let mut draft = workbook();
    assert_eq!(
        draft.copy_sheet(id(2), 4),
        Err(ValidationError::SheetPositionOutOfRange {
            position: 4,
            tab_count: 4
        })
    );
    assert_eq!(
        draft.copy_sheet(id(9), 0),
        Err(ValidationError::UnknownSheetId { value: 9 })
    );
    draft.copy_sheet(id(1), 3).expect("copy S1 last");
    assert_eq!(draft.workbook().sheets()[3].name().as_str(), "S1 (2)");
}
