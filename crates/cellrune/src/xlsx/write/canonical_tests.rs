use std::collections::BTreeMap;

use super::{
    calculation_hints_match, draft_semantics_match, generated_worksheet_xml,
    optional_false_matches, workbook_xml,
};
use crate::calculation::{MaterializedCalculationCell, MaterializedResultOrigin};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationIssue,
    CalculationIssueCode, CalculationMode, CalculationOptions, CalculationSnapshot, CellAddress,
    CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope,
    DocumentPresentation, FiniteNumber, FormulaText, FrozenPane, NumberFormat, NumberFormatKind,
    PhoneticAlignment, PhoneticProperties, PhoneticRun, PhoneticTextRange, PhoneticType,
    PhoneticWriteOptions, Provenance, ProviderIdentity, RecalculationWriteOptions, SavedResult,
    Sheet, SheetId, SheetName, SheetVisibility, Table, TableAutoFilter, TableColorFilter,
    TableColumn, TableFilterColumn, TableFilterCriteria, TableFilterItem, TableFormula, TableId,
    TableName, TableSortCondition, TableSortState, TableStyleInfo, TableType, TableValueFilters,
    TotalsRowFunction, ValidationError, WorkbookDraft, WorkbookSnapshot, WorkbookSource,
    WriteOptions, XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes,
    write_recalculated_xlsx_bytes, write_xlsx_draft_bytes,
};

use super::super::materialization::MaterializationPlan;
use super::super::{RecalculationWritePolicy, WriteLimits};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid cell address")
}

fn sheet(
    id: u32,
    name: &str,
    visibility: SheetVisibility,
    cells: &[(CellAddress, CellContent, NumberFormat)],
) -> Sheet {
    let mut sheet = Sheet::new(
        SheetId::new(id).expect("valid sheet ID"),
        SheetName::new(name).expect("valid sheet name"),
        visibility,
    );
    for (address, content, format) in cells {
        sheet
            .insert_cell_with_number_format(*address, content.clone(), format.clone())
            .expect("unique test cell");
    }
    sheet
}

fn snapshot(
    sheets: Vec<Sheet>,
    defined_names: Vec<DefinedName>,
    date_system: DateSystem,
    hints: CalculationHints,
) -> WorkbookSnapshot {
    WorkbookSnapshot::new_with_metadata(
        sheets,
        defined_names,
        Vec::new(),
        date_system,
        hints,
        WorkbookSource::default(),
        Provenance::new(ProviderIdentity::writer(), None),
    )
    .expect("valid test workbook")
}

fn empty_plan(source: &WorkbookSnapshot) -> MaterializationPlan {
    let calculation = CalculationSnapshot::new(
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        source,
        CalculationOptions::default(),
    );
    MaterializationPlan::new(
        &calculation,
        RecalculationWritePolicy::RequireComplete,
        WriteLimits::default(),
    )
    .expect("empty materialization")
}

fn base_sheet() -> Sheet {
    sheet(
        1,
        "Sheet1",
        SheetVisibility::Visible,
        &[(
            address("A1"),
            CellContent::Literal(CellValue::Number(FiniteNumber::new(1.0).expect("finite"))),
            NumberFormat::default(),
        )],
    )
}

#[test]
fn canonical_writer_reopens_the_complete_table_model() {
    let table_range = CellRange::new(address("A1"), address("C4")).expect("table range");
    let filter_range = CellRange::new(address("A1"), address("C3")).expect("filter range");
    let sort_range = CellRange::new(address("A2"), address("C3")).expect("sort range");
    let nested_sort = TableSortState::from_xlsx(
        sort_range,
        true,
        false,
        None,
        vec![TableSortCondition::from_xlsx(
            CellRange::new(address("B2"), address("B3")).expect("sort condition"),
            true,
            None,
            None,
            None,
            None,
            None,
        )],
    );
    let auto_filter = TableAutoFilter::from_xlsx(
        filter_range,
        true,
        vec![TableFilterColumn::from_xlsx(
            1,
            false,
            true,
            Some(TableFilterCriteria::Values(TableValueFilters::from_xlsx(
                false,
                None,
                vec![TableFilterItem::Value(Some("East".into()))],
            ))),
        )],
        Some(nested_sort),
    );
    let table_sort = TableSortState::from_xlsx(
        sort_range,
        false,
        true,
        None,
        vec![TableSortCondition::from_xlsx(
            CellRange::new(address("A2"), address("C2")).expect("sort condition"),
            false,
            None,
            None,
            None,
            None,
            None,
        )],
    );
    let columns = vec![
        TableColumn::new(1, "Region", None)
            .expect("column")
            .with_metadata(Some("Total".to_owned()), None, None),
        TableColumn::new(5, "Amount", None)
            .expect("column")
            .with_metadata(
                None,
                Some(TableFormula::new(
                    FormulaText::from_xlsx("[@Amount]*2").expect("formula"),
                    true,
                )),
                None,
            ),
        TableColumn::new(3, "Note", Some(TotalsRowFunction::Custom))
            .expect("column")
            .with_metadata(
                None,
                None,
                Some(TableFormula::new(
                    FormulaText::from_xlsx("SUBTOTAL(109,[Amount])").expect("formula"),
                    false,
                )),
            ),
    ];
    let table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("Sales").expect("table name"),
        TableName::new("SalesDisplay").expect("display name"),
        table_range,
        1,
        1,
        columns,
    )
    .expect("table")
    .try_with_metadata(
        TableType::Worksheet,
        true,
        Some(auto_filter),
        Some(table_sort),
        Some(TableStyleInfo::new(
            Some("TableStyleMedium2".to_owned()),
            true,
            false,
            true,
            false,
        )),
        None,
    )
    .expect("valid table metadata");
    let mut sheet = sheet(
        1,
        "Sheet1",
        SheetVisibility::Visible,
        &[
            (
                address("A1"),
                CellContent::Literal(CellValue::Text("Region".to_owned())),
                NumberFormat::default(),
            ),
            (
                address("B1"),
                CellContent::Literal(CellValue::Text("Amount".to_owned())),
                NumberFormat::default(),
            ),
            (
                address("C1"),
                CellContent::Literal(CellValue::Text("Note".to_owned())),
                NumberFormat::default(),
            ),
        ],
    );
    sheet.set_tables(vec![table]);
    let workbook = snapshot(
        vec![sheet],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    let draft = WorkbookDraft::from_snapshot_for_test(workbook);
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("canonical table workbook");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default()).expect("reopen");

    assert_eq!(
        reopened
            .workbook()
            .sheet_by_id(SheetId::new(1).expect("sheet id"))
            .expect("reopened sheet")
            .tables()
            .len(),
        1
    );
    assert_eq!(
        reopened
            .workbook()
            .table("SalesDisplay")
            .expect("reopened table"),
        draft.workbook().table("SalesDisplay").expect("draft table")
    );
    assert_eq!(
        reopened
            .table_part(TableId::new(1).expect("table id"))
            .expect("table part")
            .as_str(),
        "xl/tables/table1.xml"
    );
}

#[test]
fn canonical_writer_fails_closed_for_opaque_table_metadata() {
    let table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("Opaque").expect("table name"),
        TableName::new("Opaque").expect("display name"),
        CellRange::new(address("A1"), address("A2")).expect("table range"),
        1,
        0,
        vec![TableColumn::new(1, "Value", None).expect("column")],
    )
    .expect("table")
    .try_with_metadata(
        TableType::Worksheet,
        true,
        None,
        None,
        None,
        Some(br#"<table published="1"/>"#.to_vec()),
    )
    .expect("valid table metadata");
    let mut sheet = sheet(
        1,
        "Sheet1",
        SheetVisibility::Visible,
        &[(
            address("A1"),
            CellContent::Literal(CellValue::Text("Value".to_owned())),
            NumberFormat::default(),
        )],
    );
    sheet.set_tables(vec![table]);
    let workbook = snapshot(
        vec![sheet],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    let draft = WorkbookDraft::from_snapshot_for_test(workbook);
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let error = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect_err("opaque canonical table must fail closed");

    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
}

#[test]
fn canonical_writer_rejects_publicly_constructible_names_outside_ooxml_rules() {
    let assert_unsupported = |table: Table, header: String| {
        let mut sheet = sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Text(header)),
                NumberFormat::default(),
            )],
        );
        sheet.set_tables(vec![table]);
        let workbook = snapshot(
            vec![sheet],
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
        );
        let draft = WorkbookDraft::from_snapshot_for_test(workbook);
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let error =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect_err("canonical XLSX must enforce its stricter scalar boundary");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    };

    let invalid_name_table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("1Sales").expect("compatible public constructor"),
        TableName::new("SalesDisplay").expect("display name"),
        CellRange::new(address("A1"), address("A2")).expect("table range"),
        1,
        0,
        vec![TableColumn::new(1, "Value", None).expect("column")],
    )
    .expect("core table");
    assert_unsupported(invalid_name_table, "Value".to_owned());

    let long_column_name = "x".repeat(256);
    let invalid_column_table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("Sales").expect("table name"),
        TableName::new("SalesDisplay").expect("display name"),
        CellRange::new(address("A1"), address("A2")).expect("table range"),
        1,
        0,
        vec![
            TableColumn::new(1, long_column_name.clone(), None)
                .expect("compatible public constructor"),
        ],
    )
    .expect("core table");
    assert_unsupported(invalid_column_table, long_column_name);
}

#[test]
fn canonical_writer_fails_closed_for_external_table_dependencies() {
    for table_type in [TableType::QueryTable, TableType::Xml] {
        let table = Table::new(
            TableId::new(1).expect("table id"),
            TableName::new("External").expect("table name"),
            TableName::new("External").expect("display name"),
            CellRange::new(address("A1"), address("A2")).expect("table range"),
            1,
            0,
            vec![TableColumn::new(1, "Value", None).expect("column")],
        )
        .expect("table")
        .try_with_metadata(table_type, true, None, None, None, None)
        .expect("valid metadata");
        let mut sheet = sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Text("Value".to_owned())),
                NumberFormat::default(),
            )],
        );
        sheet.set_tables(vec![table]);
        let workbook = snapshot(
            vec![sheet],
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
        );
        let draft = WorkbookDraft::from_snapshot_for_test(workbook);
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let error =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect_err("external table dependencies cannot be synthesized");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    }
}

#[test]
fn canonical_writer_fails_closed_for_external_table_style_dependencies() {
    let assert_unsupported = |table: Table| {
        let mut sheet = sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Text("Value".to_owned())),
                NumberFormat::default(),
            )],
        );
        sheet.set_tables(vec![table]);
        let workbook = snapshot(
            vec![sheet],
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
        );
        let draft = WorkbookDraft::from_snapshot_for_test(workbook);
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let error =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect_err("unmodeled style dependencies cannot be synthesized");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    };

    let color_filter = TableAutoFilter::from_xlsx(
        CellRange::new(address("A1"), address("A2")).expect("filter range"),
        true,
        vec![TableFilterColumn::from_xlsx(
            0,
            false,
            true,
            Some(TableFilterCriteria::Color(TableColorFilter::from_xlsx(
                Some(4),
                true,
            ))),
        )],
        None,
    );
    let color_table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("Color").expect("table name"),
        TableName::new("Color").expect("display name"),
        CellRange::new(address("A1"), address("A2")).expect("table range"),
        1,
        0,
        vec![TableColumn::new(1, "Value", None).expect("column")],
    )
    .expect("table")
    .try_with_metadata(
        TableType::Worksheet,
        true,
        Some(color_filter),
        None,
        None,
        None,
    )
    .expect("valid metadata");
    assert_unsupported(color_table);

    for style_name in ["CustomTableStyle", "TableStyleLight01"] {
        let custom_style_table = Table::new(
            TableId::new(1).expect("table id"),
            TableName::new("Styled").expect("table name"),
            TableName::new("Styled").expect("display name"),
            CellRange::new(address("A1"), address("A2")).expect("table range"),
            1,
            0,
            vec![TableColumn::new(1, "Value", None).expect("column")],
        )
        .expect("table")
        .try_with_metadata(
            TableType::Worksheet,
            true,
            None,
            None,
            Some(TableStyleInfo::new(
                Some(style_name.to_owned()),
                false,
                false,
                false,
                false,
            )),
            None,
        )
        .expect("valid metadata");
        assert_unsupported(custom_style_table);
    }
}

#[test]
fn canonical_writer_rejects_table_header_mismatch() {
    let table = Table::new(
        TableId::new(1).expect("table id"),
        TableName::new("Headers").expect("table name"),
        TableName::new("Headers").expect("display name"),
        CellRange::new(address("A1"), address("A2")).expect("table range"),
        1,
        0,
        vec![TableColumn::new(1, "Expected", None).expect("column")],
    )
    .expect("table");
    let mut sheet = sheet(
        1,
        "Sheet1",
        SheetVisibility::Visible,
        &[(
            address("A1"),
            CellContent::Literal(CellValue::Text("Different".to_owned())),
            NumberFormat::default(),
        )],
    );
    sheet.set_tables(vec![table]);
    let workbook = snapshot(
        vec![sheet],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    let draft = WorkbookDraft::from_snapshot_for_test(workbook);
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let error = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect_err("table headers must match their column metadata");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
}

#[test]
fn worksheet_dimension_uses_independent_row_and_column_bounds() {
    let sheet = sheet(
        1,
        "Sheet1",
        SheetVisibility::Visible,
        &[
            (
                address("XFD1"),
                CellContent::Literal(CellValue::number(1.0).expect("finite")),
                NumberFormat::default(),
            ),
            (
                address("A2"),
                CellContent::Literal(CellValue::number(2.0).expect("finite")),
                NumberFormat::default(),
            ),
        ],
    );

    let xml = generated_worksheet_xml(
        &sheet,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &crate::DocumentPresentation::default(),
    )
    .expect("worksheet XML");

    assert!(xml.contains(r#"<dimension ref="A1:XFD2"/>"#));
}

#[test]
fn canonical_writer_round_trips_phonetics_and_frozen_panes() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let address = address("A1");
    let runs = vec![
        PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "あした").expect("run"),
        PhoneticRun::new(PhoneticTextRange::new(3, 5).expect("range"), "がっこう").expect("run"),
    ];
    let options = PhoneticWriteOptions::show().with_properties(
        PhoneticProperties::new(0)
            .with_phonetic_type(PhoneticType::Hiragana)
            .with_alignment(PhoneticAlignment::Center),
    );
    draft
        .set_annotated_text(sheet_id, address, "明日は学校へ行く", runs, options)
        .expect("annotated text");
    draft
        .set_frozen_pane(sheet_id, FrozenPane::new(1, 3).expect("frozen pane"))
        .expect("set pane");
    assert_eq!(draft.semantic_revision(), 1);
    assert_eq!(draft.presentation_revision(), 2);
    assert!(matches!(
        draft.set_cell_value(sheet_id, address, CellValue::Text("replacement".to_owned())),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default()).expect("reopen");
    let phonetics = reopened
        .presentation()
        .cell_phonetics(sheet_id, address)
        .expect("phonetics");
    assert_eq!(phonetics.runs().len(), 2);
    assert_eq!(phonetics.runs()[0].text(), "あした");
    assert_eq!(phonetics.runs()[1].text(), "がっこう");
    assert_eq!(
        phonetics.properties().and_then(|value| value.alignment()),
        Some(PhoneticAlignment::Center)
    );
    assert!(phonetics.effective_visibility());
    assert_eq!(
        reopened.presentation().frozen_pane(sheet_id).expect("pane"),
        FrozenPane::new(1, 3).expect("pane")
    );
}

/// The consumption path end to end: author annotations, write, reopen, then reach the runs the
/// way an external consumer must — enumerate annotated cells, join the base text back out of the
/// snapshot, and translate the stored UTF-16 ranges into byte offsets.
///
/// The base text mixes an astral-plane character with a BMP one so the UTF-16 offsets, the byte
/// offsets, and the `char` count are three different numbers. A translation that confused any two
/// of them would still pass on ASCII.
#[test]
fn round_tripped_phonetics_resolve_to_byte_ranges_over_the_reopened_base_text() {
    const BASE: &str = "😀明日";

    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let annotated = address("B2");
    // UTF-16: the emoji occupies 0..2, then 明 at 2..3 and 日 at 3..4.
    let runs = vec![
        PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "にこ").expect("run"),
        PhoneticRun::new(PhoneticTextRange::new(2, 4).expect("range"), "あした").expect("run"),
    ];
    draft
        .set_annotated_text(
            sheet_id,
            annotated,
            BASE,
            runs,
            PhoneticWriteOptions::show(),
        )
        .expect("annotated text");
    // A plain text cell in between, to prove the iterator yields only annotated cells.
    draft
        .set_cell_value(
            sheet_id,
            address("A1"),
            CellValue::Text("no annotation".to_owned()),
        )
        .expect("set value");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default()).expect("reopen");

    let entries: Vec<_> = reopened
        .presentation()
        .phonetic_cell_entries(sheet_id)
        .collect();
    assert_eq!(entries.len(), 1, "only the annotated cell is yielded");
    let (address_out, phonetics) = entries[0];
    assert_eq!(address_out, annotated);

    let sheet = reopened
        .workbook()
        .sheet_by_id(sheet_id)
        .expect("sheet survives the round trip");
    let CellContent::Literal(CellValue::Text(base_text)) = sheet
        .cell(address_out)
        .expect("annotated cell survives")
        .content()
    else {
        panic!("annotated cells are text cells");
    };
    assert_eq!(base_text, BASE);

    let resolved = phonetics
        .resolved_runs(base_text)
        .expect("ranges resolve against the text they were read with");
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].base_bytes(), 0..4);
    assert_eq!(resolved[0].base_slice(), "😀");
    assert_eq!(resolved[0].text(), "にこ");
    assert_eq!(resolved[1].base_bytes(), 4..10);
    assert_eq!(resolved[1].base_slice(), "明日");
    assert_eq!(resolved[1].text(), "あした");

    // Passing the wrong cell's text is caught only when the ranges do not fit it.
    assert_eq!(
        phonetics.resolved_runs("ab"),
        Err(ValidationError::PhoneticRangeOutOfBounds {
            end: 4,
            base_utf16_len: 2,
        })
    );

    // A longer wrong string can still satisfy the stored UTF-16 ranges at resolution time, because
    // presentation state deliberately does not own workbook cell contents. Once resolved, however,
    // the run retains the exact string it was checked against and cannot be sliced with another.
    let mismatched = phonetics
        .resolved_runs("no annotation")
        .expect("ranges fit the supplied text");
    assert_eq!(mismatched[0].base_slice(), "no");
}

#[test]
fn canonical_writer_round_trips_xml_normalization_sensitive_text() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let values = [
        ("A1", "\r"),
        ("A2", "\t"),
        ("A3", "\n"),
        ("A4", "before\tmiddle\nafter\rtail"),
        ("A5", "\r\n"),
    ];
    for (cell, value) in values {
        draft
            .set_cell_value(sheet_id, address(cell), CellValue::Text(value.to_owned()))
            .expect("set normalization-sensitive text");
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write normalization-sensitive text");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default()).expect("reopen");

    for (cell, expected) in values {
        let content = reopened
            .workbook()
            .sheet_by_id(sheet_id)
            .and_then(|sheet| sheet.cell(address(cell)))
            .map(|cell| cell.content());
        assert_eq!(
            content,
            Some(&CellContent::Literal(CellValue::Text(expected.to_owned()))),
            "{cell}"
        );
    }
}

#[test]
fn document_writer_round_trips_normalization_sensitive_formula_caches() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let formula_address = address("A1");
    draft
        .set_cell_formula(
            sheet_id,
            formula_address,
            FormulaText::from_xlsx("CHAR(13)").expect("formula"),
        )
        .expect("set formula");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let canonical =
        write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
            .expect("write canonical draft");
    let document = open_xlsx_document_bytes(canonical.bytes(), crate::OpenOptions::default())
        .expect("open canonical draft");

    let recalculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let rewritten = write_recalculated_xlsx_bytes(
        &document,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("rewrite document cache");
    let reopened =
        open_xlsx_document_bytes(rewritten.bytes(), crate::OpenOptions::default()).expect("reopen");
    let content = reopened
        .workbook()
        .sheet_by_id(sheet_id)
        .and_then(|sheet| sheet.cell(formula_address))
        .map(|cell| cell.content());
    let Some(CellContent::Formula(formula)) = content else {
        panic!("formula must remain present");
    };
    assert_eq!(
        formula.saved_result(),
        &SavedResult::Present(CellValue::Text("\r".to_owned()))
    );
}

#[test]
fn document_writer_round_trips_normalization_sensitive_number_formats() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let cell_address = address("A1");
    draft
        .set_cell_value(
            sheet_id,
            cell_address,
            CellValue::Number(FiniteNumber::new(1.0).expect("finite")),
        )
        .expect("set value");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let canonical =
        write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
            .expect("write canonical draft");
    let document = open_xlsx_document_bytes(canonical.bytes(), crate::OpenOptions::default())
        .expect("open canonical draft");

    let mut rewritten = WorkbookDraft::from_document(&document);
    let code = "0\"&<>\"\t0\n0\r0";
    rewritten
        .set_cell_number_format(
            sheet_id,
            cell_address,
            NumberFormat::custom(164, code, NumberFormatKind::Number).expect("custom format"),
        )
        .expect("set custom number format");
    let recalculation = calculate_workbook(rewritten.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        &rewritten,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("write document-backed draft");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), crate::OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_id(sheet_id)
            .and_then(|sheet| sheet.cell(cell_address))
            .and_then(|cell| cell.number_format().code()),
        Some(code)
    );
}

#[test]
fn canonical_writer_enforces_phonetic_run_limits_before_serialization() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let runs = vec![
        PhoneticRun::new(PhoneticTextRange::new(0, 1).expect("range"), "a").expect("run"),
        PhoneticRun::new(PhoneticTextRange::new(1, 2).expect("range"), "b").expect("run"),
    ];
    draft
        .set_annotated_text(
            sheet_id,
            address("A1"),
            "ab",
            runs,
            PhoneticWriteOptions::show(),
        )
        .expect("annotated text");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let limits = WriteLimits::default()
        .with_max_phonetic_runs_per_cell(1)
        .expect("non-zero limit");
    let options = RecalculationWriteOptions::new(WriteOptions::new(limits));

    let error = write_xlsx_draft_bytes(&draft, &calculation, options)
        .expect_err("phonetic run limit must be enforced");

    assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
    assert_eq!(error.detail(), Some("max_phonetic_runs_per_cell"));
}

#[test]
fn canonical_frozen_panes_omit_unused_split_axes() {
    let mut rows_only = crate::DocumentPresentation::default();
    let sheet_id = SheetId::new(1).expect("sheet ID");
    rows_only.source_frozen_pane(sheet_id, FrozenPane::new(2, 0).expect("rows-only pane"));
    let rows_xml = generated_worksheet_xml(
        &base_sheet(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &rows_only,
    )
    .expect("rows-only worksheet");
    assert!(
        rows_xml.contains(
            r#"<pane ySplit="2" topLeftCell="A3" activePane="bottomLeft" state="frozen"/>"#
        )
    );
    assert!(!rows_xml.contains("xSplit="));

    let mut columns_only = crate::DocumentPresentation::default();
    columns_only.source_frozen_pane(sheet_id, FrozenPane::new(0, 2).expect("columns-only pane"));
    let columns_xml = generated_worksheet_xml(
        &base_sheet(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &columns_only,
    )
    .expect("columns-only worksheet");
    assert!(
        columns_xml.contains(
            r#"<pane xSplit="2" topLeftCell="C1" activePane="topRight" state="frozen"/>"#
        )
    );
    assert!(!columns_xml.contains("ySplit="));
}

fn base_snapshot() -> WorkbookSnapshot {
    snapshot(
        vec![base_sheet()],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    )
}

#[test]
fn semantic_verification_rejects_each_independent_workbook_difference() {
    let expected = base_snapshot();
    let plan = empty_plan(&expected);
    assert!(draft_semantics_match(&expected, &expected, &plan));

    let changed_date = snapshot(
        vec![base_sheet()],
        Vec::new(),
        DateSystem::Excel1904,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_date, &plan));

    let changed_hints = snapshot(
        vec![base_sheet()],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::new(Some(CalculationMode::Manual), None, None, None),
    );
    assert!(!draft_semantics_match(&expected, &changed_hints, &plan));

    let changed_names = snapshot(
        vec![base_sheet()],
        vec![
            DefinedName::new(
                "Input",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("Sheet1!$A$1").expect("formula"),
                false,
            )
            .expect("defined name"),
        ],
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_names, &plan));

    let changed_count = snapshot(
        vec![
            base_sheet(),
            sheet(2, "Second", SheetVisibility::Visible, &[]),
        ],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_count, &plan));

    let changed_id = snapshot(
        vec![sheet(
            2,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Number(FiniteNumber::new(1.0).expect("finite"))),
                NumberFormat::default(),
            )],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_id, &plan));

    let changed_name = snapshot(
        vec![sheet(
            1,
            "Renamed",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Number(FiniteNumber::new(1.0).expect("finite"))),
                NumberFormat::default(),
            )],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_name, &plan));

    let changed_visibility = snapshot(
        vec![sheet(
            1,
            "Sheet1",
            SheetVisibility::Hidden,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Number(FiniteNumber::new(1.0).expect("finite"))),
                NumberFormat::default(),
            )],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(
        &expected,
        &changed_visibility,
        &plan
    ));

    let missing_cell = snapshot(
        vec![sheet(1, "Sheet1", SheetVisibility::Visible, &[])],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &missing_cell, &plan));

    let changed_format = snapshot(
        vec![sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Number(FiniteNumber::new(1.0).expect("finite"))),
                NumberFormat::custom(164, "0.00", NumberFormatKind::Number).expect("custom format"),
            )],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_format, &plan));

    let changed_content = snapshot(
        vec![sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[(
                address("A1"),
                CellContent::Literal(CellValue::Text("different".to_owned())),
                NumberFormat::default(),
            )],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &changed_content, &plan));

    let extra_cell = snapshot(
        vec![sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[
                (
                    address("A1"),
                    CellContent::Literal(CellValue::Number(
                        FiniteNumber::new(1.0).expect("finite"),
                    )),
                    NumberFormat::default(),
                ),
                (
                    address("B1"),
                    CellContent::Literal(CellValue::Number(
                        FiniteNumber::new(2.0).expect("finite"),
                    )),
                    NumberFormat::default(),
                ),
            ],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    assert!(!draft_semantics_match(&expected, &extra_cell, &plan));
}

#[test]
fn semantic_verification_only_allows_declared_legacy_array_followers() {
    let expected = base_snapshot();
    let actual = snapshot(
        vec![sheet(
            1,
            "Sheet1",
            SheetVisibility::Visible,
            &[
                (
                    address("A1"),
                    CellContent::Literal(CellValue::Number(
                        FiniteNumber::new(1.0).expect("finite"),
                    )),
                    NumberFormat::default(),
                ),
                (
                    address("B1"),
                    CellContent::Literal(CellValue::Number(
                        FiniteNumber::new(2.0).expect("finite"),
                    )),
                    NumberFormat::default(),
                ),
            ],
        )],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
    );
    let anchor = CalculationCellId::new(SheetId::new(1).expect("sheet ID"), address("A1"));
    let follower = CalculationCellId::new(SheetId::new(1).expect("sheet ID"), address("B1"));
    let range = CellRange::new(address("A1"), address("B1")).expect("array range");

    let mut legacy_materialized = BTreeMap::new();
    legacy_materialized.insert(
        follower,
        MaterializedCalculationCell::new(
            MaterializedResultOrigin::LegacyArray { anchor, range },
            CalculationCellResult::Value(CellValue::Number(
                FiniteNumber::new(2.0).expect("finite"),
            )),
        ),
    );
    let legacy_calculation = CalculationSnapshot::new(
        BTreeMap::new(),
        legacy_materialized,
        BTreeMap::new(),
        &expected,
        CalculationOptions::default(),
    );
    let legacy_plan = MaterializationPlan::new(
        &legacy_calculation,
        RecalculationWritePolicy::RequireComplete,
        WriteLimits::default(),
    )
    .expect("legacy plan");
    assert!(draft_semantics_match(&expected, &actual, &legacy_plan));

    let mut direct_cells = BTreeMap::new();
    direct_cells.insert(
        follower,
        CalculationCellResult::Value(CellValue::Number(FiniteNumber::new(2.0).expect("finite"))),
    );
    let mut direct_materialized = BTreeMap::new();
    direct_materialized.insert(
        follower,
        MaterializedCalculationCell::new(
            MaterializedResultOrigin::DirectFormula,
            CalculationCellResult::Value(CellValue::Number(
                FiniteNumber::new(2.0).expect("finite"),
            )),
        ),
    );
    let direct_calculation = CalculationSnapshot::new(
        direct_cells,
        direct_materialized,
        BTreeMap::new(),
        &expected,
        CalculationOptions::default(),
    );
    let direct_plan = MaterializationPlan::new(
        &direct_calculation,
        RecalculationWritePolicy::RequireComplete,
        WriteLimits::default(),
    )
    .expect("direct plan");
    assert!(!draft_semantics_match(&expected, &actual, &direct_plan));
}

#[test]
fn incomplete_materialization_alone_requires_host_recalculation_flags() {
    let expected = base_snapshot();
    let actual = snapshot(
        vec![base_sheet()],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::new(None, None, Some(true), Some(true)),
    );
    let cell = CalculationCellId::new(SheetId::new(1).expect("sheet ID"), address("A1"));
    let unavailable = CalculationCellResult::Unavailable(CalculationIssue::new(
        CalculationIssueCode::UnsupportedFunction,
        None,
    ));
    let mut cells = BTreeMap::new();
    cells.insert(cell, unavailable.clone());
    let mut materialized = BTreeMap::new();
    materialized.insert(
        cell,
        MaterializedCalculationCell::new(MaterializedResultOrigin::DirectFormula, unavailable),
    );
    let calculation = CalculationSnapshot::new(
        cells,
        materialized,
        BTreeMap::new(),
        &expected,
        CalculationOptions::default(),
    );
    let plan = MaterializationPlan::new(
        &calculation,
        RecalculationWritePolicy::InvalidateUnavailable,
        WriteLimits::default(),
    )
    .expect("incomplete materialization plan");

    assert!(draft_semantics_match(&expected, &actual, &plan));
}

#[test]
fn calculation_hint_comparison_covers_every_flag_branch() {
    let baseline = CalculationHints::new(
        Some(CalculationMode::Automatic),
        Some(42),
        Some(true),
        Some(false),
    );
    assert!(calculation_hints_match(baseline, baseline, false));
    assert!(!calculation_hints_match(
        baseline,
        CalculationHints::new(
            Some(CalculationMode::Manual),
            Some(42),
            Some(true),
            Some(false)
        ),
        false
    ));
    assert!(!calculation_hints_match(
        baseline,
        baseline.with_iterative_calculation(Some(true)),
        false
    ));
    assert!(!calculation_hints_match(
        baseline,
        CalculationHints::new(
            Some(CalculationMode::Automatic),
            Some(43),
            Some(true),
            Some(false)
        ),
        false
    ));
    assert!(!calculation_hints_match(
        baseline,
        CalculationHints::new(
            Some(CalculationMode::Automatic),
            Some(42),
            Some(false),
            Some(false)
        ),
        false
    ));
    assert!(!calculation_hints_match(
        baseline,
        CalculationHints::new(
            Some(CalculationMode::Automatic),
            Some(42),
            Some(true),
            Some(true)
        ),
        false
    ));

    let host_expected = CalculationHints::new(Some(CalculationMode::Manual), Some(9), None, None);
    assert!(calculation_hints_match(
        host_expected,
        CalculationHints::new(
            Some(CalculationMode::Manual),
            Some(9),
            Some(true),
            Some(true)
        ),
        true
    ));
    assert!(!calculation_hints_match(
        host_expected,
        CalculationHints::new(
            Some(CalculationMode::Manual),
            Some(9),
            Some(false),
            Some(true)
        ),
        true
    ));
    assert!(!calculation_hints_match(
        host_expected,
        CalculationHints::new(
            Some(CalculationMode::Manual),
            Some(9),
            Some(true),
            Some(false)
        ),
        true
    ));
}

#[test]
fn canonical_workbook_writes_iterative_calculation_hint() {
    let workbook = snapshot(
        vec![base_sheet()],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default().with_iterative_calculation(Some(true)),
    );
    let xml = workbook_xml(&workbook, &DocumentPresentation::default(), false)
        .expect("canonical workbook XML");
    assert!(xml.contains(r#"iterate="1""#));
}

#[test]
fn document_backed_writer_preserves_xml_sensitive_existing_sheet_names() {
    let mut draft = WorkbookDraft::new();
    let formula_sheet = draft.workbook().sheets()[0].id();
    let referenced_sheet = draft
        .add_sheet(SheetName::new("Data Input").expect("sheet name"))
        .expect("add sheet");
    draft
        .set_cell_value(
            referenced_sheet,
            address("A1"),
            CellValue::Number(FiniteNumber::new(2.0).expect("finite")),
        )
        .expect("set referenced value");
    draft
        .set_cell_formula(
            formula_sheet,
            address("C1"),
            FormulaText::from_xlsx("'Data Input'!A1+1").expect("formula"),
        )
        .expect("set formula");
    draft
        .rename_sheet(
            referenced_sheet,
            SheetName::new("O'Brien").expect("sheet name"),
        )
        .expect("rename sheet");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let first = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write canonical draft");
    let document = open_xlsx_document_bytes(first.bytes(), crate::OpenOptions::default())
        .expect("reopen canonical draft");
    let mut rewritten = WorkbookDraft::from_document(&document);
    rewritten
        .set_cell_value(
            formula_sheet,
            address("A1"),
            CellValue::Number(FiniteNumber::new(0.0).expect("finite")),
        )
        .expect("insert cell");
    let recalculation = calculate_workbook(rewritten.workbook(), CalculationOptions::default());

    let second = write_xlsx_draft_bytes(
        &rewritten,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("write document-backed draft");
    let final_document = open_xlsx_document_bytes(second.bytes(), crate::OpenOptions::default())
        .expect("reopen document-backed draft");
    assert_eq!(
        final_document
            .workbook()
            .sheet_by_id(referenced_sheet)
            .expect("referenced sheet")
            .name()
            .as_str(),
        "O'Brien"
    );
}

#[test]
fn document_backed_writer_escapes_new_workbook_attributes_once() {
    let draft = WorkbookDraft::new();
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let first = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write canonical draft");
    let document = open_xlsx_document_bytes(first.bytes(), crate::OpenOptions::default())
        .expect("reopen canonical draft");
    let mut rewritten = WorkbookDraft::from_document(&document);
    let added_sheet_name = "A&B <\"New\">";
    let added_sheet = rewritten
        .add_sheet(SheetName::new(added_sheet_name).expect("sheet name"))
        .expect("add sheet");
    let defined_name = "Input&<\"Name\">";
    rewritten
        .set_defined_name(
            DefinedName::new(
                defined_name,
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("Sheet1!$A$1").expect("formula"),
                false,
            )
            .expect("defined name"),
        )
        .expect("set defined name");
    let recalculation = calculate_workbook(rewritten.workbook(), CalculationOptions::default());

    let second = write_xlsx_draft_bytes(
        &rewritten,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("write document-backed draft");
    let final_document = open_xlsx_document_bytes(second.bytes(), crate::OpenOptions::default())
        .expect("reopen document-backed draft");

    assert_eq!(
        final_document
            .workbook()
            .sheet_by_id(added_sheet)
            .expect("added sheet")
            .name()
            .as_str(),
        added_sheet_name
    );
    assert_eq!(
        final_document.workbook().defined_names()[0].name(),
        defined_name
    );
}

#[test]
fn optional_false_comparison_has_an_explicit_truth_table() {
    assert!(optional_false_matches(None, None));
    assert!(optional_false_matches(None, Some(false)));
    assert!(!optional_false_matches(None, Some(true)));
    assert!(optional_false_matches(Some(true), Some(true)));
    assert!(!optional_false_matches(Some(true), Some(false)));
    assert!(optional_false_matches(Some(false), Some(false)));
    assert!(!optional_false_matches(Some(false), Some(true)));
    assert!(!optional_false_matches(Some(false), None));
}
