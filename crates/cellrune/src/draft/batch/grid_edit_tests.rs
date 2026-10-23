use crate::workbook::{DataValidationRule, NonWorksheetTabs, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellRange, CellValue, Column, DataValidation, DataValidationErrorStyle,
    DataValidationOperator, DataValidationType, DateSystem, DefinedName, DefinedNameScope,
    EditBatch, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, FrozenPane, NumberFormat,
    Provenance, Row, SavedResult, SharedFormulaRole, Sheet, SheetId, SheetName, SheetVisibility,
    Table, TableColumn, TableId, TableName, ValidationError, WorkbookChange, WorkbookDraft,
    WorkbookSnapshot, WorkbookSource, calculate_workbook,
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

fn column(value: u32) -> Column {
    Column::new(value).expect("column")
}

fn formula(text: &str, metadata: FormulaMetadata) -> CellContent {
    CellContent::Formula(FormulaCell::new(
        FormulaDialect::ExcelA1,
        FormulaText::from_xlsx(text).expect("formula"),
        SavedResult::Missing,
        metadata,
    ))
}

fn sheet(sheet_id: u32, name: &str, cells: &[(&str, &str)]) -> Sheet {
    let mut sheet = Sheet::new(
        id(sheet_id),
        SheetName::new(name).expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (cell, input) in cells {
        let content = match input.strip_prefix('=') {
            Some(text) => formula(text, FormulaMetadata::Normal),
            None => match input.parse::<f64>() {
                Ok(number) => CellContent::Literal(CellValue::number(number).expect("number")),
                Err(_) => CellContent::Literal(CellValue::Text((*input).to_owned())),
            },
        };
        sheet.upsert_cell(address(cell), content, NumberFormat::default());
    }
    sheet
}

fn table(table_id: u32, display_name: &str, cells: &str, totals: u32, names: &[&str]) -> Table {
    Table::new(
        TableId::new(table_id).expect("table ID"),
        TableName::new(display_name).expect("table name"),
        TableName::new(display_name).expect("display name"),
        range(cells),
        1,
        totals,
        names
            .iter()
            .enumerate()
            .map(|(index, name)| TableColumn::new(index as u32 + 1, *name, None).expect("column"))
            .collect(),
    )
    .expect("table")
}

fn draft(sheets: Vec<Sheet>, defined_names: Vec<DefinedName>) -> WorkbookDraft {
    let workbook = WorkbookSnapshot::new_with_input(WorkbookSnapshotInput {
        sheets,
        non_worksheet_tabs: NonWorksheetTabs::default(),
        defined_names,
        diagnostics: Vec::new(),
        date_system: DateSystem::Excel1900,
        calculation_hints: CalculationHints::default(),
        source: WorkbookSource::default(),
        provenance: Provenance::new(crate::ProviderIdentity::writer(), None),
    })
    .expect("workbook");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn apply(draft: &mut WorkbookDraft, change: WorkbookChange) -> crate::EditReceipt {
    draft
        .apply_changes(EditBatch::new([change]))
        .expect("structural edit")
}

fn refused(draft: &mut WorkbookDraft, change: WorkbookChange) -> ValidationError {
    let before = draft.workbook().fingerprint();
    let error = draft
        .apply_changes(EditBatch::new([change]))
        .expect_err("refused structural edit");
    assert_eq!(draft.workbook().fingerprint(), before, "{error}");
    error
}

fn text(workbook: &WorkbookSnapshot, sheet: &str, cell: &str) -> String {
    match workbook
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell(address(cell))
        .map(crate::Cell::content)
    {
        Some(CellContent::Formula(formula)) => {
            format!("={}", formula.text().expect("formula text").as_str())
        }
        Some(CellContent::Literal(CellValue::Number(number))) => number.get().to_string(),
        Some(CellContent::Literal(CellValue::Text(text))) => text.clone(),
        Some(CellContent::Literal(other)) => format!("{other:?}"),
        None => String::new(),
    }
}

fn metadata(workbook: &WorkbookSnapshot, sheet: &str, cell: &str) -> FormulaMetadata {
    match workbook
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell(address(cell))
        .map(crate::Cell::content)
    {
        Some(CellContent::Formula(formula)) => formula.metadata().clone(),
        other => panic!("{sheet}!{cell} holds no formula: {other:?}"),
    }
}

fn value(draft: &WorkbookDraft, sheet: u32, cell: &str) -> CellValue {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    match calculation.cell(CalculationCellId::new(id(sheet), address(cell))) {
        Some(CalculationCellResult::Value(value)) => value.clone(),
        other => panic!("{cell}: {other:?}"),
    }
}

fn insert_rows(sheet: u32, at: u32, count: u32) -> WorkbookChange {
    WorkbookChange::insert_rows(id(sheet), row(at), count).expect("insertion")
}

fn delete_rows(sheet: u32, first: u32, last: u32) -> WorkbookChange {
    WorkbookChange::delete_rows(id(sheet), row(first), row(last)).expect("deletion")
}

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
            ("B1", "=SUM(A1:A3)"),
            ("B2", "=A3"),
            ("B3", "=SUM(A3:A4)"),
            ("B4", "=SUM(A1:A2)"),
            ("C1", "=$A$3+A$4"),
            ("D1", "=SUM(2:3)"),
            ("D2", "=SUM(A:A)"),
            ("D3", "=SUM('S1:S2'!A3)"),
            ("D4", "=INDIRECT(\"A3\")"),
            ("F1", "key"),
            ("G1", "val"),
            ("F2", "a"),
            ("G2", "1"),
            ("F3", "b"),
            ("G3", "2"),
            ("F4", "c"),
            ("G4", "3"),
        ],
    );
    s1.set_tables(vec![table(1, "Things", "F1:G4", 0, &["key", "val"])]);
    s1.set_merged_ranges(vec![range("H2:H4")]);
    let s2 = sheet(
        2,
        "S2",
        &[
            ("A1", "='S1'!A3"),
            ("A2", "=A3"),
            ("A3", "7"),
            ("B1", "=Window"),
        ],
    );
    let window = DefinedName::new(
        "Window",
        DefinedNameScope::Workbook,
        FormulaText::from_xlsx("SUM('S1'!$A$2:$A$4)").expect("name"),
        false,
    )
    .expect("defined name");
    draft(vec![s1, s2], vec![window])
}

#[test]
fn inserted_rows_move_cells_and_every_reference_by_the_measured_rules() {
    let mut draft = probe();
    let receipt = apply(&mut draft, insert_rows(1, 3, 2));
    assert!(receipt.topology_changed());
    let workbook = draft.workbook();
    for (cell, expected) in [
        ("A2", "2"),
        ("A3", ""),
        ("A5", "3"),
        ("A7", "5"),
        ("B1", "=SUM(A1:A5)"),
        ("B2", "=A5"),
        ("B5", "=SUM(A5:A6)"),
        ("B6", "=SUM(A1:A2)"),
        ("C1", "=$A$5+A$6"),
        ("D1", "=SUM(2:5)"),
        ("D2", "=SUM(A:A)"),
        ("D5", "=SUM('S1:S2'!A3)"),
        ("D6", "=INDIRECT(\"A3\")"),
        ("G6", "3"),
    ] {
        assert_eq!(text(workbook, "S1", cell), expected, "S1!{cell}");
    }
    assert_eq!(text(workbook, "S2", "A1"), "='S1'!A5");
    assert_eq!(text(workbook, "S2", "A2"), "=A3");
    assert_eq!(
        workbook.defined_names()[0].formula().as_str(),
        "SUM('S1'!$A$2:$A$6)"
    );
    let s1 = workbook.sheet_by_name("S1").expect("S1");
    assert_eq!(s1.tables()[0].range(), range("F1:G6"));
    assert_eq!(s1.merged_ranges(), &[range("H2:H6")]);
    assert_eq!(value(&draft, 1, "B1"), CellValue::number(6.0).expect("6"));
    assert_eq!(value(&draft, 2, "A1"), CellValue::number(3.0).expect("3"));
    assert_eq!(value(&draft, 2, "B1"), CellValue::number(9.0).expect("9"));
}

#[test]
fn deleted_rows_turn_references_to_them_into_ref_errors_and_shrink_ranges() {
    let mut draft = probe();
    apply(&mut draft, delete_rows(1, 3, 3));
    let workbook = draft.workbook();
    for (cell, expected) in [
        ("A3", "4"),
        ("B1", "=SUM(A1:A2)"),
        ("B2", "=#REF!"),
        ("B3", "=SUM(A1:A2)"),
        ("C1", "=#REF!+A$3"),
        ("D1", "=SUM(2:2)"),
        ("D2", "=SUM(A:A)"),
    ] {
        assert_eq!(text(workbook, "S1", cell), expected, "S1!{cell}");
    }
    assert_eq!(text(workbook, "S1", "B4"), "");
    assert_eq!(text(workbook, "S2", "A1"), "='S1'!#REF!");
    assert_eq!(
        workbook.defined_names()[0].formula().as_str(),
        "SUM('S1'!$A$2:$A$3)"
    );
    let s1 = workbook.sheet_by_name("S1").expect("S1");
    assert_eq!(s1.tables()[0].range(), range("F1:G3"));
    assert_eq!(s1.merged_ranges(), &[range("H2:H3")]);
    assert_eq!(
        value(&draft, 1, "B2"),
        CellValue::Error(crate::ExcelError::Reference)
    );

    let mut whole = probe();
    apply(&mut whole, delete_rows(1, 3, 4));
    assert_eq!(text(whole.workbook(), "S1", "B3"), "");
    assert_eq!(text(whole.workbook(), "S1", "B1"), "=SUM(A1:A2)");
    let s1 = whole.workbook().sheet_by_name("S1").expect("S1");
    assert_eq!(s1.tables()[0].range(), range("F1:G2"));
    assert!(s1.merged_ranges().is_empty());
}

#[test]
fn column_edits_move_columns_as_row_edits_move_rows() {
    let mut draft = probe();
    apply(
        &mut draft,
        WorkbookChange::insert_columns(id(1), column(2), 1).expect("insertion"),
    );
    let workbook = draft.workbook();
    assert_eq!(text(workbook, "S1", "C1"), "=SUM(A1:A3)");
    assert_eq!(text(workbook, "S1", "D1"), "=$A$3+A$4");
    assert_eq!(text(workbook, "S1", "E1"), "=SUM(2:3)");
    assert_eq!(
        workbook.sheet_by_name("S1").expect("S1").tables()[0].range(),
        range("G1:H4")
    );

    let mut deleted = probe();
    deleted
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
            id(2),
            address("C1"),
            FormulaText::from_xlsx("SUM('S1'!A1:C1)+'S1'!B1").expect("formula"),
        )]))
        .expect("formula");
    apply(
        &mut deleted,
        WorkbookChange::delete_columns(id(1), column(2), column(2)).expect("deletion"),
    );
    assert_eq!(
        text(deleted.workbook(), "S2", "C1"),
        "=SUM('S1'!A1:B1)+'S1'!#REF!"
    );
    assert_eq!(text(deleted.workbook(), "S1", "B1"), "=$A$3+A$4");
}

#[test]
fn tables_grow_inside_their_body_and_refuse_losing_their_header_or_data() {
    let mut draft = probe();
    for (change, error) in [
        (
            delete_rows(1, 1, 1),
            ValidationError::TableHeaderShift { table_id: 1 },
        ),
        (
            delete_rows(1, 2, 4),
            ValidationError::TableDataRowsDeleted { table_id: 1 },
        ),
    ] {
        assert_eq!(refused(&mut draft, change), error);
    }
    // Rows inserted right below a table leave it as it is.
    apply(&mut draft, insert_rows(1, 5, 1));
    assert_eq!(
        draft.workbook().sheet_by_name("S1").expect("S1").tables()[0].range(),
        range("F1:G4")
    );
    // A column inserted inside the table becomes a named table column.
    apply(
        &mut draft,
        WorkbookChange::insert_columns(id(1), column(7), 1).expect("insertion"),
    );
    let workbook = draft.workbook();
    let things = &workbook.sheet_by_name("S1").expect("S1").tables()[0];
    assert_eq!(things.range(), range("F1:H4"));
    assert_eq!(
        things
            .columns()
            .iter()
            .map(TableColumn::name)
            .collect::<Vec<_>>(),
        ["key", "Column1", "val"]
    );
    assert_eq!(text(workbook, "S1", "G1"), "Column1");
    assert_eq!(text(workbook, "S1", "H4"), "3");
}

#[test]
fn edits_that_split_a_legacy_array_or_push_cells_off_the_sheet_are_refused() {
    let mut s1 = sheet(1, "S1", &[("A1", "1"), ("A2", "2"), ("A3", "3")]);
    s1.upsert_cell(
        address("B1"),
        formula(
            "A1:A3*2",
            FormulaMetadata::Array {
                range: range("B1:B3"),
                always_calculate: false,
            },
        ),
        NumberFormat::default(),
    );
    let mut array = draft(vec![s1], Vec::new());
    for change in [insert_rows(1, 2, 1), delete_rows(1, 3, 4)] {
        assert!(matches!(
            refused(&mut array, change),
            ValidationError::ArrayFormulaPartEdit { .. }
        ));
    }
    apply(&mut array, insert_rows(1, 1, 1));
    assert_eq!(
        metadata(array.workbook(), "S1", "B2"),
        FormulaMetadata::Array {
            range: range("B2:B4"),
            always_calculate: false,
        }
    );
    assert_eq!(text(array.workbook(), "S1", "B2"), "=A2:A4*2");

    let mut spill = sheet(1, "S1", &[]);
    spill.upsert_cell(
        address("A1"),
        formula(
            "SEQUENCE(3)",
            FormulaMetadata::DynamicArray {
                range: Some(range("A1:A3")),
                always_calculate: false,
            },
        ),
        NumberFormat::default(),
    );
    let mut spilled = draft(vec![spill], Vec::new());
    for (source, destination) in [("A2:A3", "C2"), ("A1:A2", "E1")] {
        let change = WorkbookChange::move_range(id(1), range(source), address(destination))
            .expect("range move");
        assert!(matches!(
            refused(&mut spilled, change),
            ValidationError::ArrayFormulaPartEdit { .. }
        ));
    }
    apply(
        &mut spilled,
        WorkbookChange::move_range(id(1), range("A1:A3"), address("C1")).expect("range move"),
    );

    let mut full = probe();
    full.apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
        id(1),
        address("A1048576"),
        CellValue::number(1.0).expect("one"),
    )]))
    .expect("last row");
    assert_eq!(
        refused(&mut full, insert_rows(1, 2, 1)),
        ValidationError::ContentShiftedOffSheet { sheet_id: 1 }
    );
}

#[test]
fn shared_formula_groups_stay_shared_only_while_they_read_back_the_same() {
    let shared = |text: &str, role: SharedFormulaRole, cells: Option<&str>| {
        formula(
            text,
            FormulaMetadata::Shared {
                group_index: 0,
                role,
                range: cells.map(range),
            },
        )
    };
    let follower = SharedFormulaRole::Follower {
        anchor: address("B1"),
    };
    let build = |anchor_text: &str, texts: [&str; 2]| {
        let mut s1 = sheet(
            1,
            "S1",
            &[("A1", "1"), ("A2", "2"), ("A3", "3"), ("A4", "4")],
        );
        s1.upsert_cell(
            address("B1"),
            shared(anchor_text, SharedFormulaRole::Anchor, Some("B1:B3")),
            NumberFormat::default(),
        );
        for (cell, text) in ["B2", "B3"].into_iter().zip(texts) {
            s1.upsert_cell(
                address(cell),
                shared(text, follower, None),
                NumberFormat::default(),
            );
        }
        draft(vec![s1], Vec::new())
    };

    let mut moved = build("A1", ["A2", "A3"]);
    apply(&mut moved, insert_rows(1, 1, 1));
    assert_eq!(
        metadata(moved.workbook(), "S1", "B2"),
        FormulaMetadata::Shared {
            group_index: 0,
            role: SharedFormulaRole::Anchor,
            range: Some(range("B2:B4")),
        }
    );
    assert_eq!(
        metadata(moved.workbook(), "S1", "B4"),
        FormulaMetadata::Shared {
            group_index: 0,
            role: SharedFormulaRole::Follower {
                anchor: address("B2")
            },
            range: None,
        }
    );

    // The anchor reads a row the insertion moves while the first follower stays: shifting the
    // anchor's new text no longer gives the followers' texts, so the group becomes ordinary
    // formulas with the same texts.
    let mut split = build("A2", ["A3", "A4"]);
    apply(&mut split, insert_rows(1, 2, 1));
    let workbook = split.workbook();
    for (cell, expected) in [("B1", "=A3"), ("B3", "=A4"), ("B4", "=A5")] {
        assert_eq!(text(workbook, "S1", cell), expected, "{cell}");
        assert_eq!(metadata(workbook, "S1", cell), FormulaMetadata::Normal);
    }
}

#[test]
fn data_validations_keep_their_cells_and_read_from_their_first_cell() {
    let rule = DataValidationRule {
        validation_type: DataValidationType::Custom,
        operator: DataValidationOperator::Between,
        error_style: DataValidationErrorStyle::Stop,
        allow_blank: true,
        in_cell_dropdown: true,
        show_input_message: false,
        show_error_message: false,
        error_title: None,
        error: None,
        prompt_title: None,
        prompt: None,
    };
    let validation = |ranges: &[&str], formula1: &str| {
        DataValidation::from_xlsx(
            ranges.iter().map(|text| range(text)).collect(),
            rule.clone(),
            Some(formula1.into()),
            None,
        )
    };
    let build = || {
        let mut s1 = sheet(1, "S1", &[("C1", "1")]);
        s1.set_data_validations(vec![
            validation(&["A1:A5"], "A1>0"),
            validation(&["B1:B2"], "$C$1>0"),
            validation(&["D3:D4"], "D3>0"),
        ]);
        draft(vec![s1], Vec::new())
    };
    let summary = |draft: &WorkbookDraft| {
        draft
            .workbook()
            .sheet_by_name("S1")
            .expect("S1")
            .data_validations()
            .iter()
            .map(|validation| {
                (
                    validation
                        .ranges()
                        .iter()
                        .map(|range| format!("{}:{}", range.start(), range.end()))
                        .collect::<Vec<_>>()
                        .join(" "),
                    validation.formula1().unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let owned = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(ranges, formula)| ((*ranges).to_owned(), (*formula).to_owned()))
            .collect::<Vec<_>>()
    };

    // Rows inserted right below a validated range join it, as they take the row above's format.
    let mut inserted = build();
    apply(&mut inserted, insert_rows(1, 3, 2));
    assert_eq!(
        summary(&inserted),
        owned(&[("A1:A7", "A1>0"), ("B1:B4", "$C$1>0"), ("D5:D6", "D5>0")])
    );

    // Deleting the first validated cell leaves the formula reading the cell that now comes
    // first, and a validation whose cells are all deleted goes.
    let mut deleted = build();
    apply(&mut deleted, delete_rows(1, 1, 2));
    assert_eq!(
        summary(&deleted),
        owned(&[("A1:A3", "A1>0"), ("D1:D2", "D1>0")])
    );

    // Cutting part of a validation away makes it a validation of its own that reads as the rule
    // did there, as Excel 16.114 splits it (excel-measure I4); the cells left keep the rule.
    let mut split = build();
    apply(
        &mut split,
        WorkbookChange::move_range(id(1), range("A4:A5"), address("F10")).expect("move"),
    );
    assert_eq!(
        summary(&split),
        owned(&[
            ("A1:A3", "A1>0"),
            ("F10:F11", "F10>0"),
            ("B1:B2", "$C$1>0"),
            ("D3:D4", "D3>0")
        ])
    );
    // Cutting the first cell away leaves the rest reading from the cell that now comes first.
    let mut first = build();
    apply(
        &mut first,
        WorkbookChange::move_range(id(1), range("A1:A2"), address("F10")).expect("move"),
    );
    assert_eq!(
        summary(&first),
        owned(&[
            ("A3:A5", "A3>0"),
            ("F10:F11", "F10>0"),
            ("B1:B2", "$C$1>0"),
            ("D3:D4", "D3>0")
        ])
    );
    let mut absolute = build();
    apply(
        &mut absolute,
        WorkbookChange::move_range(id(1), range("B2"), address("F10")).expect("move"),
    );
    assert_eq!(
        summary(&absolute),
        owned(&[
            ("A1:A5", "A1>0"),
            ("B1:B1", "$C$1>0"),
            ("F10:F10", "$C$1>0"),
            ("D3:D4", "D3>0")
        ])
    );
}

#[test]
fn inserted_rows_take_the_row_above_and_frozen_panes_keep_their_lines() {
    let mut draft = draft(
        vec![sheet(1, "S1", &[("A1", "1"), ("A6", "6")])],
        Vec::new(),
    );
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_row_height(id(1), row(2), Some(30.0)),
            WorkbookChange::set_row_height(id(1), row(4), Some(40.0)),
        ]))
        .expect("heights");
    draft
        .set_frozen_pane(id(1), FrozenPane::new(2, 0).expect("pane"))
        .expect("frozen pane");
    apply(&mut draft, insert_rows(1, 3, 2));
    let presentation = draft.presentation();
    for (line, height) in [
        (2, Some(30.0)),
        (3, Some(30.0)),
        (4, Some(30.0)),
        (6, Some(40.0)),
    ] {
        assert_eq!(presentation.row_height(id(1), row(line)), height, "{line}");
    }
    assert_eq!(presentation.row_height(id(1), row(5)), None);
    assert_eq!(
        presentation.frozen_pane(id(1)),
        Some(FrozenPane::new(2, 0).expect("pane"))
    );
    apply(&mut draft, insert_rows(1, 1, 1));
    assert_eq!(
        draft.presentation().frozen_pane(id(1)),
        Some(FrozenPane::new(3, 0).expect("pane"))
    );
    apply(&mut draft, delete_rows(1, 1, 2));
    assert_eq!(
        draft.presentation().frozen_pane(id(1)),
        Some(FrozenPane::new(1, 0).expect("pane"))
    );
}

#[test]
fn constructors_reject_empty_and_off_sheet_edits() {
    assert_eq!(
        WorkbookChange::insert_rows(id(1), row(1), 0),
        Err(ValidationError::StructuralEditEmpty)
    );
    assert_eq!(
        WorkbookChange::insert_rows(id(1), row(crate::EXCEL_MAX_ROWS), 2),
        Err(ValidationError::RowOutOfRange {
            value: crate::EXCEL_MAX_ROWS + 1
        })
    );
    assert_eq!(
        WorkbookChange::delete_columns(id(1), column(3), column(2)),
        Err(ValidationError::RangeStartAfterEnd)
    );
    let mut draft = probe();
    assert_eq!(
        refused(
            &mut draft,
            WorkbookChange::InsertRows {
                sheet_id: id(1),
                row: row(1),
                count: 0,
            }
        ),
        ValidationError::StructuralEditEmpty
    );
    assert_eq!(
        refused(&mut draft, insert_rows(9, 1, 1)),
        ValidationError::UnknownSheetId { value: 9 }
    );
}

#[test]
fn a_new_workbook_saves_structural_edits_and_recalculates_the_same_when_reopened() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    let mut changes = (1..=6)
        .map(|line| {
            WorkbookChange::set_cell_value(
                sheet,
                CellAddress::new(row(line), column(1)),
                CellValue::number(f64::from(line)).expect("number"),
            )
        })
        .collect::<Vec<_>>();
    for (cell, text) in [
        ("B1", "SUM(A1:A6)"),
        ("B2", "A4*2"),
        ("B3", "COUNT(A:A)"),
        ("B4", "SUM(A5:A6)"),
        ("C1", "ROW(A6)"),
    ] {
        changes.push(WorkbookChange::set_cell_formula(
            sheet,
            address(cell),
            FormulaText::from_xlsx(text).expect("formula"),
        ));
    }
    changes.push(WorkbookChange::set_row_height(sheet, row(3), Some(25.0)));
    draft.apply_changes(EditBatch::new(changes)).expect("cells");
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::insert_rows(sheet, row(2), 2).expect("insertion"),
            WorkbookChange::delete_rows(sheet, row(6), row(6)).expect("deletion"),
            WorkbookChange::insert_columns(sheet, column(1), 1).expect("insertion"),
        ]))
        .expect("structural edits");
    let workbook = draft.workbook();
    assert_eq!(text(workbook, "Sheet1", "C1"), "=SUM(B1:B7)");
    assert_eq!(text(workbook, "Sheet1", "C4"), "=#REF!*2");
    assert_eq!(text(workbook, "Sheet1", "D1"), "=ROW(B7)");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = crate::write_xlsx_draft_bytes(
        &draft,
        &calculation,
        crate::RecalculationWriteOptions::default(),
    )
    .expect("canonical write");
    let reopened = crate::open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default())
        .expect("reopen");
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    for cell in draft.workbook().sheets()[0].cells() {
        let spelled = cell.address().to_string();
        assert_eq!(
            text(reopened.workbook(), "Sheet1", &spelled),
            text(draft.workbook(), "Sheet1", &spelled),
            "{spelled}"
        );
        let id = CalculationCellId::new(sheet, cell.address());
        assert_eq!(fresh.cell(id), calculation.cell(id), "{spelled}");
    }
    assert_eq!(
        reopened.presentation().row_height(sheet, row(5)),
        Some(25.0)
    );
}

#[test]
fn moved_rows_carry_their_references_and_each_range_end() {
    let mut draft = draft(
        vec![sheet(
            1,
            "S1",
            &[
                ("A1", "1"),
                ("A2", "2"),
                ("A3", "3"),
                ("A4", "4"),
                ("A8", "8"),
                ("C1", "=SUM(A1:A3)"),
                ("C2", "=SUM(A3:A4)"),
                ("C3", "=A3"),
                ("C4", "=A4"),
            ],
        )],
        Vec::new(),
    );
    let receipt = apply(
        &mut draft,
        WorkbookChange::move_rows(id(1), row(3), row(3), row(8)).expect("move"),
    );
    assert!(receipt.topology_changed());
    let workbook = draft.workbook();
    // Row 3 lands on row 8, so rows 4 through 8 move up one.
    for (cell, expected) in [
        ("A3", "4"),
        ("A7", "8"),
        ("A8", "3"),
        ("C1", "=SUM(A1:A8)"),
        ("C2", "=SUM(A3:A3)"),
        ("C3", "=A3"),
        ("C8", "=A8"),
    ] {
        assert_eq!(text(workbook, "S1", cell), expected, "{cell}");
    }
    assert_eq!(value(&draft, 1, "C1"), CellValue::number(18.0).expect("18"));

    apply(
        &mut draft,
        WorkbookChange::move_columns(id(1), column(3), column(3), column(1)).expect("move"),
    );
    assert_eq!(text(draft.workbook(), "S1", "A1"), "=SUM(B1:B8)");
    assert_eq!(text(draft.workbook(), "S1", "B8"), "3");
}

#[test]
fn moves_that_split_an_array_a_merge_or_a_table_are_refused() {
    let mut s1 = sheet(1, "S1", &[("A1", "1"), ("A2", "2"), ("A3", "3")]);
    s1.upsert_cell(
        address("B1"),
        formula(
            "A1:A3*2",
            FormulaMetadata::Array {
                range: range("B1:B3"),
                always_calculate: false,
            },
        ),
        NumberFormat::default(),
    );
    s1.set_merged_ranges(vec![range("D5:D6")]);
    s1.set_tables(vec![table(1, "Things", "F10:G12", 0, &["key", "val"])]);
    let mut draft = draft(vec![s1], Vec::new());
    let moved = |first, last, destination| {
        WorkbookChange::move_rows(id(1), row(first), row(last), row(destination)).expect("move")
    };
    assert!(matches!(
        refused(&mut draft, moved(2, 2, 20)),
        ValidationError::ArrayFormulaPartEdit { .. }
    ));
    assert!(matches!(
        refused(&mut draft, moved(6, 6, 20)),
        ValidationError::MergedRangeSplit { .. }
    ));
    assert_eq!(
        refused(&mut draft, moved(11, 11, 20)),
        ValidationError::TableSplit { table_id: 1 }
    );
    assert_eq!(
        refused(&mut draft, moved(30, 30, 11)),
        ValidationError::TableSplit { table_id: 1 }
    );
    // Moving a whole table, merge, and array moves them with their rows.
    apply(&mut draft, moved(1, 12, 21));
    let sheet = draft.workbook().sheet_by_name("S1").expect("S1");
    assert_eq!(sheet.tables()[0].range(), range("F30:G32"));
    assert_eq!(sheet.merged_ranges(), &[range("D25:D26")]);
    assert_eq!(
        metadata(draft.workbook(), "S1", "B21"),
        FormulaMetadata::Array {
            range: range("B21:B23"),
            always_calculate: false,
        }
    );
}

#[test]
fn a_range_move_carries_references_wholly_inside_it_and_keeps_moved_formulas() {
    let build = || {
        draft(
            vec![
                sheet(
                    1,
                    "S1",
                    &[
                        ("A1", "1"),
                        ("A2", "2"),
                        ("A3", "=B1"),
                        ("A4", "=A2"),
                        ("A5", "5"),
                        ("B1", "10"),
                        ("C1", "=A5"),
                        ("C2", "=SUM(A2:A5)"),
                        ("C3", "=SUM(A1:A5)"),
                        ("M2", "99"),
                    ],
                ),
                sheet(2, "S2", &[("A1", "='S1'!A2")]),
            ],
            Vec::new(),
        )
    };
    let cut = WorkbookChange::move_range(id(1), range("A2:A5"), address("M2")).expect("move");
    let mut moved = build();
    apply(&mut moved, cut.clone());
    let workbook = moved.workbook();
    for (cell, expected) in [
        ("A2", ""),
        ("M2", "2"),
        ("M3", "=B1"),
        ("M4", "=M2"),
        ("C1", "=M5"),
        ("C2", "=SUM(M2:M5)"),
        ("C3", "=SUM(A1:A5)"),
    ] {
        assert_eq!(text(workbook, "S1", cell), expected, "{cell}");
    }
    assert_eq!(text(workbook, "S2", "A1"), "='S1'!M2");
    assert_eq!(value(&moved, 1, "C2"), CellValue::number(19.0).expect("19"));

    // A reference to a cell the paste overwrites is gone, as in Excel.
    let mut reading = build();
    reading
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
            id(2),
            address("B1"),
            FormulaText::from_xlsx("'S1'!M3").expect("formula"),
        )]))
        .expect("reader");
    apply(&mut reading, cut);
    assert_eq!(text(reading.workbook(), "S2", "B1"), "='S1'!#REF!");

    // A formula the paste overwrites does not hold the move back, whatever it reads.
    let mut overwritten = draft(
        vec![sheet(
            1,
            "S1",
            &[("A1", "1"), ("A2", "2"), ("M1", "5"), ("M2", "=M1+1")],
        )],
        Vec::new(),
    );
    apply(
        &mut overwritten,
        WorkbookChange::move_range(id(1), range("A1:A2"), address("M1")).expect("move"),
    );
    assert_eq!(
        overwritten
            .workbook()
            .sheet_by_name("S1")
            .and_then(|sheet| sheet.cell(address("M2")))
            .map(crate::Cell::content),
        Some(&CellContent::Literal(CellValue::number(2.0).expect("two")))
    );

    let mut s1 = sheet(1, "S1", &[("A1", "1")]);
    s1.set_merged_ranges(vec![range("D1:D2")]);
    s1.set_tables(vec![table(1, "Things", "F1:G3", 0, &["key", "val"])]);
    let mut structures = draft(vec![s1], Vec::new());
    for (cut, error) in [
        (
            WorkbookChange::move_range(id(1), range("D2:D3"), address("J1")),
            ValidationError::MergedRangeSplit {
                sheet_id: 1,
                row: 1,
                column: 4,
            },
        ),
        (
            WorkbookChange::move_range(id(1), range("G1:G3"), address("J1")),
            ValidationError::TableSplit { table_id: 1 },
        ),
        (
            WorkbookChange::move_range(id(1), range("A1"), address("F2")),
            ValidationError::TableSplit { table_id: 1 },
        ),
    ] {
        assert_eq!(refused(&mut structures, cut.expect("move")), error);
    }
    apply(
        &mut structures,
        WorkbookChange::move_range(id(1), range("D1:G3"), address("D11")).expect("move"),
    );
    let sheet = structures.workbook().sheet_by_name("S1").expect("S1");
    assert_eq!(sheet.tables()[0].range(), range("F11:G13"));
    assert_eq!(sheet.merged_ranges(), &[range("D11:D12")]);
}

fn copy(source: &str, destination_sheet: u32, destination: &str) -> WorkbookChange {
    WorkbookChange::copy_range(
        id(1),
        range(source),
        id(destination_sheet),
        range(destination),
    )
    .expect("copy")
}

#[test]
fn copies_move_relative_references_by_the_copy_distance() {
    let build = || {
        draft(
            vec![
                sheet(
                    1,
                    "S1",
                    &[
                        ("A1", "1"),
                        ("A2", "2"),
                        ("B1", "=A1*2"),
                        ("B2", "=SUM(A2:A5)"),
                        ("C1", "=$A$1+A$1+$A1"),
                        ("D1", "=SUM(T[c])+Rate"),
                        ("E1", "=A1+'S2'!A1"),
                        ("F2", "=A1"),
                        ("G1", "7"),
                        ("G2", "8"),
                        ("H1", "=G1"),
                    ],
                ),
                sheet(2, "S2", &[("A1", "5"), ("B2", "9")]),
            ],
            Vec::new(),
        )
    };
    let mut draft = build();
    draft
        .apply_changes(EditBatch::new([
            copy("B2", 1, "C6"),
            copy("B1", 1, "B3:B5"),
            copy("C1", 1, "D4"),
            copy("D1", 1, "D9"),
            copy("F2", 1, "F1"),
            copy("G1:H1", 1, "J1:M2"),
            copy("E1", 2, "E1"),
            copy("Z1", 1, "A2"),
        ]))
        .expect("copies");
    let workbook = draft.workbook();
    for (cell, expected) in [
        ("C6", "=SUM(B6:B9)"),
        ("B3", "=A3*2"),
        ("B5", "=A5*2"),
        ("D4", "=$A$1+B$1+$A4"),
        ("D9", "=SUM(T[c])+Rate"),
        ("F1", "=#REF!"),
        ("J1", "7"),
        ("K1", "=J1"),
        ("L2", "7"),
        ("M2", "=L2"),
        ("A2", ""),
    ] {
        assert_eq!(text(workbook, "S1", cell), expected, "{cell}");
    }
    // Unqualified references of a formula copied to another sheet read that sheet.
    assert_eq!(text(workbook, "S2", "E1"), "=A1+'S2'!A1");
    assert_eq!(value(&draft, 2, "E1"), CellValue::number(10.0).expect("10"));

    let mut refused_draft = build();
    refused_draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
            id(1),
            address("P1"),
            FormulaText::from_xlsx("A1#").expect("spill reference"),
        )]))
        .expect("spill reference");
    assert_eq!(
        refused(&mut refused_draft, copy("P1", 1, "P2")),
        ValidationError::CopyUnsupportedFormula {
            sheet_id: 1,
            row: 1,
            column: 16,
        }
    );
    assert_eq!(
        WorkbookChange::copy_range(id(1), range("A1:A2"), id(1), range("B1:B3")),
        Err(ValidationError::CopyDestinationShape)
    );
}

#[test]
fn copies_carry_number_formats_and_cell_formats() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    let bold = crate::CellFormatEdit::new().with_font(
        crate::Font::new()
            .with_name("Calibri")
            .and_then(|font| font.with_size(11.0))
            .expect("font")
            .with_bold(true),
    );
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_value(
                sheet,
                address("A1"),
                CellValue::number(1.5).expect("1.5"),
            ),
            WorkbookChange::set_cell_number_format(
                sheet,
                address("A1"),
                NumberFormat::custom(164, "0.000", crate::NumberFormatKind::Number)
                    .expect("format"),
            ),
            WorkbookChange::set_cell_format(sheet, address("A1"), bold),
            WorkbookChange::copy_range(sheet, range("A1"), sheet, range("C3")).expect("copy"),
        ]))
        .expect("copy formats");
    let copied = draft.workbook().sheets()[0]
        .cell(address("C3"))
        .expect("copied cell");
    assert_eq!(copied.number_format().code(), Some("0.000"));
    assert_eq!(
        draft.cell_format(sheet, address("C3")).expect("format"),
        draft.cell_format(sheet, address("A1")).expect("format")
    );
}

/// `T` holds the table `T` over A1:C5 with columns `a`, `b`, and `c`, and formulas in H1:H8 that
/// read it.
fn table_probe() -> WorkbookDraft {
    let mut cells = vec![
        ("H1", "=SUM(T[b])"),
        ("H2", "=SUM(T[c])"),
        ("H3", "=SUM(T[[a]:[b]])"),
        ("H4", "=SUM(T[[b]:[c]])"),
        ("H5", "=ROWS(T[#All])"),
        ("H6", "=SUM(B2:B5)"),
        ("H7", "=SUM(A3:C3)"),
        ("H8", "=B3"),
    ];
    let values = (2..=5)
        .flat_map(|row| {
            [
                (format!("A{row}"), row.to_string()),
                (format!("B{row}"), (row * 10).to_string()),
                (format!("C{row}"), (row * 100).to_string()),
            ]
        })
        .collect::<Vec<_>>();
    cells.extend(
        values
            .iter()
            .map(|(cell, value)| (cell.as_str(), value.as_str())),
    );
    let mut t = sheet(1, "T", &cells);
    for (cell, header) in [("A1", "a"), ("B1", "b"), ("C1", "c")] {
        t.upsert_cell(
            address(cell),
            CellContent::Literal(CellValue::Text(header.to_owned())),
            crate::NumberFormat::default(),
        );
    }
    t.set_tables(vec![table(1, "T", "A1:C5", 0, &["a", "b", "c"])]);
    draft(vec![t], Vec::new())
}

#[test]
fn column_deletions_remove_table_columns_and_tables_as_excel_does() {
    // Excel 16.114 (excel-measure K): each deletion, the table range and columns left, and the
    // formulas that read the table, moved left with their cells.
    for (first, last, table_range, columns, formulas) in [
        (
            2,
            2,
            Some("A1:B5"),
            vec!["a", "c"],
            [
                "=SUM(#REF!)",
                "=SUM(T[c])",
                "=SUM(#REF!)",
                "=SUM(#REF!)",
                "=ROWS(T[#All])",
                "=SUM(#REF!)",
                "=SUM(A3:B3)",
                "=#REF!",
            ],
        ),
        (
            3,
            4,
            Some("A1:B5"),
            vec!["a", "b"],
            [
                "=SUM(T[b])",
                "=SUM(#REF!)",
                "=SUM(T[[a]:[b]])",
                "=SUM(#REF!)",
                "=ROWS(T[#All])",
                "=SUM(B2:B5)",
                "=SUM(A3:B3)",
                "=B3",
            ],
        ),
        (
            1,
            3,
            None,
            Vec::new(),
            [
                "=SUM(#REF!)",
                "=SUM(#REF!)",
                "=SUM(#REF!)",
                "=SUM(#REF!)",
                "=ROWS(#REF!)",
                "=SUM(#REF!)",
                "=SUM(#REF!)",
                "=#REF!",
            ],
        ),
    ] {
        let mut draft = table_probe();
        apply(
            &mut draft,
            WorkbookChange::delete_columns(id(1), column(first), column(last)).expect("deletion"),
        );
        let workbook = draft.workbook();
        let sheet = workbook.sheet_by_name("T").expect("T");
        assert_eq!(
            sheet.tables().first().map(Table::range),
            table_range.map(range),
            "{first}:{last}"
        );
        if let Some(table) = sheet.tables().first() {
            assert_eq!(
                table
                    .columns()
                    .iter()
                    .map(TableColumn::name)
                    .collect::<Vec<_>>(),
                columns
            );
        }
        let formula_column = 8 - (last - first + 1);
        for (row, expected) in (1..).zip(formulas) {
            let cell = CellAddress::new(crate::Row::new(row).expect("row"), column(formula_column));
            assert_eq!(
                text(workbook, "T", &cell.to_string()),
                expected,
                "{first}:{last} {cell}"
            );
        }
    }
}

#[test]
fn rows_move_within_a_table_s_data_and_keep_its_range() {
    // Excel 16.114 (excel-measure K): row 3 cut and inserted before row 6 lands on row 5.
    let mut draft = table_probe();
    apply(
        &mut draft,
        WorkbookChange::move_rows(id(1), row(3), row(3), row(5)).expect("move"),
    );
    let workbook = draft.workbook();
    assert_eq!(
        workbook.sheet_by_name("T").expect("T").tables()[0].range(),
        range("A1:C5")
    );
    for (cell, expected) in [
        ("A5", "3"),
        ("A3", "4"),
        ("H1", "=SUM(T[b])"),
        ("H7", "=SUM(A5:C5)"),
        ("H8", "=B5"),
    ] {
        assert_eq!(text(workbook, "T", cell), expected, "{cell}");
    }
    // A move out of the data rows still splits the table.
    assert_eq!(
        refused(
            &mut table_probe(),
            WorkbookChange::move_rows(id(1), row(3), row(3), row(8)).expect("move"),
        ),
        ValidationError::TableSplit { table_id: 1 }
    );
}
