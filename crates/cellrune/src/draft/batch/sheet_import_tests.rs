use super::ImportedSheet;
use crate::workbook::{NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationHints, CalculationOptions, CellAddress, CellContent, CellFormatEdit, CellRange,
    CellValue, Column, DateSystem, DefinedName, DefinedNameScope, EditBatch, FontEdit, FormulaCell,
    FormulaDialect, FormulaMetadata, FormulaText, NumberFormat, NumberFormatKind, Provenance,
    SavedResult, Sheet, SheetId, SheetName, SheetVisibility, Table, TableColumn, TableId,
    TableName, ValidationError, WorkbookChange, WorkbookDraft, WorkbookSnapshot, WorkbookSource,
    calculate_workbook,
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

fn table(table_id: u32, name: &str, range: &str) -> Table {
    let (start, end) = range.split_once(':').expect("range");
    Table::new(
        TableId::new(table_id).expect("table ID"),
        TableName::new(name).expect("name"),
        TableName::new(name).expect("display name"),
        CellRange::new(address(start), address(end)).expect("range"),
        1,
        0,
        vec![
            TableColumn::new(1, "h1", None).expect("h1"),
            TableColumn::new(2, "h2", None).expect("h2"),
        ],
    )
    .expect("table")
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

fn draft(
    sheets: Vec<Sheet>,
    defined_names: Vec<DefinedName>,
    date_system: DateSystem,
) -> WorkbookDraft {
    WorkbookDraft::from_snapshot_for_test(
        WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
            sheets,
            non_worksheet_tabs: NonWorksheetTabs::default(),
            defined_names,
            diagnostics: Vec::new(),
            date_system,
            calculation_hints: CalculationHints::default(),
            source: WorkbookSource::default(),
            provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
        })
        .expect("workbook"),
    )
}

/// The section O workbook: `Data` with the `Things` table and formulas reading names, the
/// table, and itself, and `Other`; `Rate` and `Local` read `Data`, and `Far` reads `Other`.
fn source() -> WorkbookDraft {
    let mut data = sheet(
        1,
        "Data",
        &[
            ("A1", "h1"),
            ("B1", "h2"),
            ("A2", "1"),
            ("B2", "2"),
            ("A3", "3"),
            ("B3", "4"),
            ("D1", "=Rate*2"),
            ("D2", "=Local+1"),
            ("D3", "=SUM(Things[h2])"),
            ("D5", "=Data!A2"),
        ],
    );
    data.set_tables(vec![table(1, "Things", "A1:B3")]);
    let other = sheet(2, "Other", &[("A1", "10")]);
    draft(
        vec![data, other],
        vec![
            name("Rate", None, "Data!$A$2"),
            name("Far", None, "Other!$A$1"),
            name("Local", Some(1), "Data!$B$2"),
        ],
        DateSystem::Excel1900,
    )
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

fn names(draft: &WorkbookDraft) -> Vec<(String, DefinedNameScope, String)> {
    draft
        .workbook()
        .defined_names()
        .iter()
        .map(|name| {
            (
                name.name().to_owned(),
                name.scope(),
                name.formula().as_str().to_owned(),
            )
        })
        .collect()
}

fn import(target: &mut WorkbookDraft, position: usize) -> SheetId {
    let imported = ImportedSheet::from_draft(&source(), id(1)).expect("readable sheet");
    let receipt = target
        .apply_changes(EditBatch::new([WorkbookChange::import_sheet(
            imported, position,
        )]))
        .expect("import");
    assert!(receipt.topology_changed());
    let sheet_id = receipt.created_sheet_ids()[0];
    // The inverse removes the names the import added and then the sheet.
    assert_eq!(
        receipt
            .inverse()
            .and_then(|inverse| inverse.changes().last()),
        Some(&WorkbookChange::delete_sheet(sheet_id))
    );
    sheet_id
}

#[test]
fn an_imported_sheet_keeps_free_names_and_brings_the_names_that_read_it() {
    let mut target = draft(
        vec![sheet(1, "Sheet1", &[])],
        Vec::new(),
        DateSystem::Excel1900,
    );
    let sheet_id = import(&mut target, 1);
    assert_eq!(sheet_id, id(2));
    let imported = target.workbook().sheet_by_id(sheet_id).expect("imported");
    assert_eq!(imported.name().as_str(), "Data");
    assert_eq!(imported.tables()[0].display_name().as_str(), "Things");
    assert_eq!(formula(&target, "Data", "D5"), "Data!A2");
    // Excel 16.114 (excel-measure RESULTS.md, section O): Local stays scoped to the sheet, Rate
    // becomes a workbook name, and Far, which reads another sheet, stays behind.
    assert_eq!(
        names(&target),
        [
            (
                "Local".to_owned(),
                DefinedNameScope::Sheet(sheet_id),
                "Data!$B$2".to_owned()
            ),
            (
                "Rate".to_owned(),
                DefinedNameScope::Workbook,
                "Data!$A$2".to_owned()
            ),
        ]
    );
    let calculation = calculate_workbook(target.workbook(), CalculationOptions::default());
    for (cell, expected) in [("D1", 2.0), ("D2", 3.0), ("D3", 6.0), ("D5", 1.0)] {
        assert_eq!(
            calculation.cell(crate::CalculationCellId::new(sheet_id, address(cell))),
            Some(&crate::CalculationCellResult::Value(
                CellValue::number(expected).expect("number")
            )),
            "{cell}"
        );
    }
}

#[test]
fn an_imported_sheet_takes_new_names_where_the_target_has_them() {
    let mut existing = sheet(1, "Data", &[("A1", "x"), ("B1", "y"), ("A2", "5")]);
    existing.set_tables(vec![table(1, "Things", "A1:B2")]);
    let mut target = draft(
        vec![existing],
        vec![name("Rate", None, "Data!$A$2")],
        DateSystem::Excel1900,
    );
    let sheet_id = import(&mut target, 1);
    let imported = target.workbook().sheet_by_id(sheet_id).expect("imported");
    assert_eq!(imported.name().as_str(), "Data (2)");
    // One table in the target: `Things` becomes `Things3`, and the formulas follow.
    assert_eq!(imported.tables()[0].display_name().as_str(), "Things3");
    assert_eq!(
        imported.tables()[0].id(),
        TableId::new(2).expect("table ID")
    );
    assert_eq!(formula(&target, "Data (2)", "D3"), "SUM(Things3[h2])");
    assert_eq!(formula(&target, "Data (2)", "D5"), "'Data (2)'!A2");
    // The target keeps its workbook `Rate`; the sheet's own comes scoped to it.
    assert_eq!(
        names(&target),
        [
            (
                "Rate".to_owned(),
                DefinedNameScope::Workbook,
                "Data!$A$2".to_owned()
            ),
            (
                "Local".to_owned(),
                DefinedNameScope::Sheet(sheet_id),
                "'Data (2)'!$B$2".to_owned()
            ),
            (
                "Rate".to_owned(),
                DefinedNameScope::Sheet(sheet_id),
                "'Data (2)'!$A$2".to_owned()
            ),
        ]
    );
}

#[test]
fn an_imported_sheet_carries_its_formats_widths_and_number_formats() {
    let mut source = source();
    let percent = NumberFormat::built_in(10, NumberFormatKind::Number).expect("built-in");
    source
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_number_format(id(1), address("A2"), percent.clone()),
            WorkbookChange::set_cell_format(
                id(1),
                address("B2"),
                CellFormatEdit::new().with_font(FontEdit::new().with_bold(true)),
            ),
            WorkbookChange::set_column_width(
                id(1),
                Column::new(2).expect("column"),
                Column::new(3).expect("column"),
                Some(20.0),
            ),
        ]))
        .expect("format the source");
    let imported = ImportedSheet::from_draft(&source, id(1)).expect("readable sheet");
    let mut target = draft(
        vec![sheet(1, "Sheet1", &[])],
        Vec::new(),
        DateSystem::Excel1900,
    );
    target
        .apply_changes(EditBatch::new([WorkbookChange::import_sheet(imported, 0)]))
        .expect("import");
    let sheet_id = id(2);
    assert_eq!(target.workbook().sheets()[0].id(), sheet_id);
    let cell = target
        .workbook()
        .sheet_by_id(sheet_id)
        .and_then(|sheet| sheet.cell(address("A2")))
        .expect("A2");
    assert_eq!(cell.number_format(), &percent);
    assert!(
        target
            .cell_format(sheet_id, address("B2"))
            .expect("format")
            .font()
            .bold()
    );
    assert!(
        !target
            .cell_format(sheet_id, address("A2"))
            .expect("format")
            .font()
            .bold()
    );
    assert_eq!(
        target
            .presentation()
            .column_width(sheet_id, Column::new(3).expect("column")),
        Some(20.0)
    );
}

#[test]
fn reading_another_sheet_or_workbook_or_a_different_date_system_refuses_the_import() {
    let unsupported = |content: &str| ValidationError::SheetImportUnsupported {
        content: content.to_owned(),
    };
    for (formula, content) in [
        ("=Other!A1", "a reference to sheet 'Other'"),
        ("=SUM(Data:Other!A1)", "a 3-D reference"),
        ("=[1]Book!A1", "a reference to another workbook"),
        ("=Far*2", "the name Far that reads another sheet"),
    ] {
        let mut source = source();
        source
            .set_cell_formula(
                id(1),
                address("E1"),
                FormulaText::from_user_input(formula).expect("formula"),
            )
            .expect("formula edit");
        assert_eq!(
            ImportedSheet::from_draft(&source, id(1)),
            Err(unsupported(content)),
            "{formula}"
        );
    }
    let imported = ImportedSheet::from_draft(&source(), id(1)).expect("readable sheet");
    let mut target = draft(
        vec![sheet(1, "Sheet1", &[])],
        Vec::new(),
        DateSystem::Excel1904,
    );
    let Err(error) =
        target.apply_changes(EditBatch::new([WorkbookChange::import_sheet(imported, 0)]))
    else {
        panic!("the date systems differ");
    };
    assert_eq!(error, unsupported("a different date system"));
}
