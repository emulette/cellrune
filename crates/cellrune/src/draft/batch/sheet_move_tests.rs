use crate::workbook::{NonWorksheetTab, NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellValue, DateSystem, DefinedName, DefinedNameScope, EditBatch, FormulaCell,
    FormulaDialect, FormulaMetadata, FormulaText, NumberFormat, Provenance, SavedResult, Sheet,
    SheetId, SheetName, SheetVisibility, ValidationError, WorkbookChange, WorkbookDraft,
    WorkbookSnapshot, WorkbookSource, calculate_workbook,
};

fn id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("cell address")
}

const FORMULAS: [(&str, &str); 3] = [
    ("B1", "SUM('S2:S5'!A1)"),
    ("B2", "SUM('S3:S4'!A1)"),
    ("B3", "SUM('S2:S3'!A1)"),
];

/// Sheets `S1` through `S6` as in the Excel probe, each with its index in `A1`; `S1` holds the
/// 3-D sums, and names cover a 3-D span and a sheet scope.
fn probe(non_worksheet_tabs: NonWorksheetTabs) -> WorkbookDraft {
    let sheets = (1..=6)
        .map(|index| {
            let mut sheet = Sheet::new(
                id(index),
                SheetName::new(format!("S{index}")).expect("sheet name"),
                SheetVisibility::Visible,
            );
            sheet.upsert_cell(
                address("A1"),
                CellContent::Literal(CellValue::number(f64::from(index)).expect("number")),
                NumberFormat::default(),
            );
            if index == 1 {
                for (cell, formula) in FORMULAS {
                    sheet.upsert_cell(
                        address(cell),
                        CellContent::Formula(FormulaCell::new(
                            FormulaDialect::ExcelA1,
                            FormulaText::from_xlsx(formula).expect("formula"),
                            SavedResult::Missing,
                            FormulaMetadata::Normal,
                        )),
                        NumberFormat::default(),
                    );
                }
            }
            sheet
        })
        .collect();
    let name = |name: &str, scope, formula: &str| {
        DefinedName::new(
            name,
            scope,
            FormulaText::from_xlsx(formula).expect("name formula"),
            false,
        )
        .expect("defined name")
    };
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets,
        non_worksheet_tabs,
        defined_names: vec![
            name("Span", DefinedNameScope::Workbook, "'S3:S4'!$A$1"),
            name("Local", DefinedNameScope::Sheet(id(3)), "'S3'!$A$1"),
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

fn formulas(draft: &WorkbookDraft) -> Vec<String> {
    let sheet = draft.workbook().sheet_by_id(id(1)).expect("S1");
    FORMULAS
        .iter()
        .map(
            |(cell, _)| match sheet.cell(address(cell)).expect("cell").content() {
                CellContent::Formula(formula) => formula.text().expect("text").as_str().to_owned(),
                CellContent::Literal(_) => panic!("{cell} holds a formula"),
            },
        )
        .collect()
}

fn order(draft: &WorkbookDraft) -> Vec<String> {
    draft
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str().to_owned())
        .collect()
}

#[test]
fn moving_a_sheet_rewrites_only_the_3d_references_it_reverses_as_excel_saves_them() {
    // Excel 16.114, `S1` holding `SUM('S2:S5'!A1)`, `SUM('S3:S4'!A1)`, and `SUM('S2:S3'!A1)`.
    for (moved, position, expected) in [
        // S3 after S6: `'S2:S5'` loses S3 and keeps its text; `'S3:S4'` becomes `'S4'`.
        (3, 5, ["SUM('S2:S5'!A1)", "SUM('S4'!A1)", "SUM('S2:S3'!A1)"]),
        // S6 before S4 joins `'S2:S5'`; S5 after S6 makes `'S2:S5'` take S6 in.
        (
            6,
            3,
            ["SUM('S2:S5'!A1)", "SUM('S3:S4'!A1)", "SUM('S2:S3'!A1)"],
        ),
        (
            5,
            5,
            ["SUM('S2:S5'!A1)", "SUM('S3:S4'!A1)", "SUM('S2:S3'!A1)"],
        ),
        // S5 before S2 gives `'S2:S4'`.
        (
            5,
            1,
            ["SUM('S2:S4'!A1)", "SUM('S3:S4'!A1)", "SUM('S2:S3'!A1)"],
        ),
        // S2 after S5 gives `'S3:S5'`, and `'S2:S3'` becomes `'S3'`.
        (2, 4, ["SUM('S3:S5'!A1)", "SUM('S3:S4'!A1)", "SUM('S3'!A1)"]),
        // S2 after S3 keeps `'S2:S5'`, which loses S3, and reverses `'S2:S3'` into `'S3'`.
        (2, 2, ["SUM('S2:S5'!A1)", "SUM('S3:S4'!A1)", "SUM('S3'!A1)"]),
    ] {
        let mut draft = probe(NonWorksheetTabs::default());
        draft.move_sheet(id(moved), position).expect("move");
        assert_eq!(formulas(&draft), expected, "S{moved} to {position}");
        assert_eq!(
            draft.workbook().sheet_tab_position(id(moved)),
            Some(position),
            "S{moved}"
        );
    }
}

#[test]
fn a_move_keeps_ids_and_scoped_names_and_reorders_tabs() {
    let mut draft = probe(NonWorksheetTabs::default());
    let base = draft.semantic_revision();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::move_sheet(id(3), 5)]))
        .expect("move S3 last");
    assert!(receipt.topology_changed());
    assert_eq!(receipt.result_revision(), base + 1);
    assert_eq!(order(&draft), ["S1", "S2", "S4", "S5", "S6", "S3"]);
    let workbook = draft.workbook();
    let local = workbook
        .defined_names()
        .iter()
        .find(|name| name.name() == "Local")
        .expect("Local");
    assert_eq!(local.scope(), DefinedNameScope::Sheet(id(3)));
    assert_eq!(workbook.sheet_tab_position(id(3)), Some(5));
    let span = workbook
        .defined_names()
        .iter()
        .find(|name| name.name() == "Span")
        .expect("Span");
    assert_eq!(span.formula().as_str(), "'S4'!$A$1");

    // The 3-D sums follow tab order: `'S2:S5'` holds S2, S4, and S5 now.
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    assert_eq!(
        calculation.cell(CalculationCellId::new(id(1), address("B1"))),
        Some(&CalculationCellResult::Value(
            CellValue::number(11.0).expect("number")
        ))
    );

    // Moving a tab to its own position changes nothing.
    let revision = draft.semantic_revision();
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::move_sheet(id(3), 5)]))
        .expect("no-op move");
    assert_eq!(receipt.result_revision(), revision);
    assert!(!receipt.topology_changed());
}

#[test]
fn chartsheets_move_and_3d_neighbours_pass_over_them() {
    // Tab order: S1, S2, Chart, S3, S4, S5, S6.
    let chart = || {
        NonWorksheetTabs::new(vec![NonWorksheetTab::new(
            2,
            id(9),
            SheetName::new("Chart").expect("chart name"),
            SheetVisibility::Visible,
        )])
    };
    let mut draft = probe(chart());
    // S2 after S3: the neighbour of S2 inside `'S2:S3'` passes over the chartsheet to S3.
    draft.move_sheet(id(2), 3).expect("move S2");
    assert_eq!(formulas(&draft)[2], "SUM('S3'!A1)");
    assert_eq!(order(&draft), ["S1", "S3", "S2", "S4", "S5", "S6"]);
    assert_eq!(draft.workbook().sheet_tab_position(id(9)), Some(1));

    let mut draft = probe(chart());
    draft.move_sheet(id(9), 0).expect("move the chartsheet");
    assert_eq!(draft.workbook().sheet_tab_position(id(9)), Some(0));
    assert_eq!(draft.workbook().sheet_tab_position(id(1)), Some(1));
    assert_eq!(formulas(&draft), FORMULAS.map(|(_, formula)| formula));
}

#[test]
fn a_move_past_the_tab_order_or_of_an_unknown_tab_fails_atomically() {
    let mut draft = probe(NonWorksheetTabs::default());
    let revision = draft.semantic_revision();
    assert_eq!(
        draft.move_sheet(id(2), 6),
        Err(ValidationError::SheetPositionOutOfRange {
            position: 6,
            tab_count: 6
        })
    );
    assert_eq!(
        draft.move_sheet(id(8), 0),
        Err(ValidationError::UnknownSheetId { value: 8 })
    );
    assert_eq!(
        draft.apply_changes(EditBatch::new([
            WorkbookChange::move_sheet(id(2), 0),
            WorkbookChange::move_sheet(id(2), 9),
        ])),
        Err(ValidationError::SheetPositionOutOfRange {
            position: 9,
            tab_count: 6
        })
    );
    assert_eq!(draft.semantic_revision(), revision);
    assert_eq!(order(&draft), ["S1", "S2", "S3", "S4", "S5", "S6"]);
}
