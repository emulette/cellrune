use crate::workbook::WorkbookSnapshotInput;
use crate::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope, EditBatch,
    FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, NumberFormat, Provenance,
    SavedResult, Sheet, SheetId, SheetName, SheetVisibility, Table, TableColumn, TableFormula,
    TableId, TableName, ValidationError, WorkbookChange, WorkbookDraft, WorkbookSnapshot,
    WorkbookSource, calculate_workbook,
};

fn id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

fn sheet(sheet_id: u32, cells: &[(&str, &str)]) -> Sheet {
    let mut sheet = Sheet::new(
        id(sheet_id),
        SheetName::new(format!("S{sheet_id}")).expect("sheet name"),
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

/// A workbook `Rate` on `S1!A1`, an `S2` scoped `Rate` on `S2!A1`, a workbook `Twice` that reads
/// `Rate`, and the `Calc` table on `S3` whose calculated column reads `Rate`.
fn workbook() -> WorkbookDraft {
    let s1 = sheet(
        1,
        &[
            ("A1", "10"),
            ("B1", "=Rate*2"),
            ("B2", "='S2'!Rate"),
            ("B3", "=S3!Rate"),
            ("B4", "=INDIRECT(\"Rate\")"),
            ("B5", "=Twice"),
        ],
    );
    let s2 = sheet(2, &[("A1", "20"), ("B1", "=Rate"), ("B2", "=S1!Rate")]);
    let mut s3 = sheet(3, &[("D1", "x"), ("D2", "=Rate+1")]);
    s3.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Calc").expect("table name"),
            TableName::new("Calc").expect("display name"),
            CellRange::new(address("D1"), address("D2")).expect("range"),
            1,
            0,
            vec![TableColumn::new(1, "x", None).expect("x").with_metadata(
                None,
                Some(TableFormula::new(
                    FormulaText::from_xlsx("Rate+1").expect("calculated"),
                    false,
                )),
                None,
            )],
        )
        .expect("table"),
    ]);
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets: vec![s1, s2, s3],
        non_worksheet_tabs: crate::workbook::NonWorksheetTabs::default(),
        defined_names: vec![
            name("Rate", None, "'S1'!$A$1"),
            name("Rate", Some(2), "'S2'!$A$1"),
            name("Twice", None, "Rate*2"),
            name("Other", Some(3), "'S3'!$A$1"),
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

fn values(draft: &WorkbookDraft) -> Vec<Option<CalculationCellResult>> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    // Sheet-qualified names do not calculate, so only the unqualified uses are compared.
    [(1, "B1"), (1, "B5"), (2, "B1")]
        .into_iter()
        .map(|(sheet_id, cell)| {
            calculation
                .cell(CalculationCellId::new(id(sheet_id), address(cell)))
                .cloned()
        })
        .collect()
}

#[test]
fn renaming_a_workbook_name_rewrites_the_references_that_resolve_to_it() {
    let mut draft = workbook();
    let before = values(&draft);
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_defined_name(
            DefinedNameScope::Workbook,
            "rate",
            "Tax",
        )]))
        .expect("rename Rate");
    assert!(receipt.topology_changed());
    // `S2` has its own `Rate`, which `'S2'!Rate` and `Rate` on `S2` keep reading.
    for (sheet_id, cell, expected) in [
        (1, "B1", "Tax*2"),
        (1, "B2", "'S2'!Rate"),
        (1, "B3", "S3!Tax"),
        (1, "B4", "INDIRECT(\"Rate\")"),
        (2, "B1", "Rate"),
        (2, "B2", "S1!Tax"),
        (3, "D2", "Tax+1"),
    ] {
        assert_eq!(
            formula(&draft, sheet_id, cell),
            expected,
            "S{sheet_id}!{cell}"
        );
    }
    assert_eq!(
        names(&draft),
        [
            (
                "Tax".to_owned(),
                DefinedNameScope::Workbook,
                "'S1'!$A$1".to_owned()
            ),
            (
                "Rate".to_owned(),
                DefinedNameScope::Sheet(id(2)),
                "'S2'!$A$1".to_owned()
            ),
            (
                "Twice".to_owned(),
                DefinedNameScope::Workbook,
                "Tax*2".to_owned()
            ),
            (
                "Other".to_owned(),
                DefinedNameScope::Sheet(id(3)),
                "'S3'!$A$1".to_owned()
            ),
        ]
    );
    let calc = draft.workbook().table("Calc").expect("Calc");
    assert_eq!(
        calc.columns()[0]
            .calculated_column_formula()
            .expect("calculated")
            .text()
            .as_str(),
        "Tax+1"
    );
    assert_eq!(values(&draft), before);
}

#[test]
fn renaming_a_sheet_name_rewrites_its_sheet_and_qualified_uses() {
    let mut draft = workbook();
    let before = values(&draft);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_defined_name(
            DefinedNameScope::Sheet(id(2)),
            "Rate",
            "Local",
        )]))
        .expect("rename S2's Rate");
    for (sheet_id, cell, expected) in [
        (1, "B1", "Rate*2"),
        (1, "B2", "'S2'!Local"),
        (1, "B3", "S3!Rate"),
        (2, "B1", "Local"),
        (2, "B2", "S1!Rate"),
    ] {
        assert_eq!(
            formula(&draft, sheet_id, cell),
            expected,
            "S{sheet_id}!{cell}"
        );
    }
    assert_eq!(values(&draft), before);
}

#[test]
fn a_rename_that_collides_or_lets_another_scope_take_over_fails_atomically() {
    let mut draft = workbook();
    let revision = draft.semantic_revision();
    let rename = |scope, from: &str, to: &str| {
        EditBatch::new([WorkbookChange::rename_defined_name(scope, from, to)])
    };
    assert_eq!(
        draft.apply_changes(rename(DefinedNameScope::Workbook, "Missing", "X")),
        Err(ValidationError::UnknownDefinedName {
            name: "Missing".to_owned()
        })
    );
    assert_eq!(
        draft.apply_changes(rename(DefinedNameScope::Workbook, "Twice", "RATE")),
        Err(ValidationError::DuplicateDefinedName {
            name: "RATE".to_owned()
        })
    );
    // `S3` has its own `Other`, which uses of a workbook `Other` on `S3` would mean.
    assert_eq!(
        draft.apply_changes(rename(DefinedNameScope::Workbook, "Twice", "Other")),
        Err(ValidationError::DefinedNameRenameShadowed {
            name: "Other".to_owned()
        })
    );
    // A workbook `Twice` exists, which `Twice` on `S2` reads today.
    assert_eq!(
        draft.apply_changes(rename(DefinedNameScope::Sheet(id(2)), "Rate", "Twice")),
        Err(ValidationError::DefinedNameRenameShadowed {
            name: "Twice".to_owned()
        })
    );
    assert_eq!(draft.semantic_revision(), revision);

    // A rename to the same spelling is a no-op; one that only changes case is a rename.
    let receipt = draft
        .apply_changes(rename(DefinedNameScope::Workbook, "Twice", "Twice"))
        .expect("no-op");
    assert_eq!(receipt.result_revision(), revision);
    draft
        .apply_changes(rename(DefinedNameScope::Workbook, "Twice", "TWICE"))
        .expect("case-only rename");
    assert_eq!(formula(&draft, 1, "B5"), "TWICE");
}
