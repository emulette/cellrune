use std::io::{Cursor, Read, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{CompressionMethod, ZipArchive};

use super::{read_xlsx, read_xlsx_bytes};
use crate::{
    CalculationMode, CellAddress, CellContent, CellValue, DateSystem, NumberFormatKind,
    OpenOptions, PhoneticAlignment, PhoneticRun, PhoneticTextRange, PhoneticType,
    PhoneticWriteOptions, ReadLimits, ReadOptions, RecalculationWriteOptions, SavedResult, SheetId,
    SheetVisibility, WorkbookDraft, WorkbookSourceKind, XlsxErrorCode, XlsxWriteErrorCode,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:rel="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <workbookPr date1904="1"/>
  <sheets>
    <sheet name="Second" sheetId="2" state="hidden" rel:id="rId2"/>
    <sheet name="First" sheetId="1" rel:id="rId1"/>
  </sheets>
  <calcPr calcId="7" calcMode="manual" fullCalcOnLoad="1" forceFullCalc="0" iterate="1"/>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts>
  <cellXfs count="3">
    <xf numFmtId="0"/>
    <xf numFmtId="164" applyNumberFormat="1"/>
    <xf numFmtId="46" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"#;

const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="1">
  <si><r><t>Hello</t></r><r><t xml:space="preserve">, </t></r><r><t>世界</t></r></si>
</sst>"#;

const SHEET_ONE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1">
    <c r="A1" t="s"><v>0</v></c>
    <c r="B1" t="inlineStr"><is><r><t>inline</t></r><r><t xml:space="preserve"> value</t></r></is></c>
    <c r="C1" t="b"><v>1</v></c>
    <c r="D1" t="e"><v>#DIV/0!</v></c>
    <c r="E1" s="1"><v>45292</v></c>
    <c r="F1" s="2"><v>1.5</v></c>
    <c r="G1"><v>42.5</v></c>
    <c r="H1"><f>1+1</f><v>2</v></c>
    <c r="I1"/>
  </row></sheetData>
</worksheet>"#;

const SHEET_TWO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="2"><c r="A2" t="s"><v>0</v></c></row></sheetData>
</worksheet>"#;

#[test]
fn reads_workbook_metadata_and_supported_literal_types() {
    let archive = build_archive(SHEET_ONE, SHARED_STRINGS);
    let snapshot =
        read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("generated workbook");

    assert_eq!(snapshot.sheets()[0].name().as_str(), "Second");
    assert_eq!(snapshot.sheets()[1].name().as_str(), "First");
    assert_eq!(snapshot.sheets()[0].visibility(), SheetVisibility::Hidden);
    assert_eq!(snapshot.date_system(), DateSystem::Excel1904);
    assert_eq!(
        snapshot.calculation_hints().mode(),
        Some(CalculationMode::Manual)
    );
    assert_eq!(snapshot.calculation_hints().calculation_id(), Some(7));
    assert_eq!(
        snapshot.calculation_hints().full_calculation_on_load(),
        Some(true)
    );
    assert_eq!(
        snapshot.calculation_hints().force_full_calculation(),
        Some(false)
    );
    assert_eq!(
        snapshot.calculation_hints().iterative_calculation(),
        Some(true)
    );
    assert_eq!(snapshot.source().kind(), WorkbookSourceKind::Reader);
    assert!(snapshot.source().byte_length().is_some());

    let first = snapshot
        .sheet_by_name("first")
        .expect("case-insensitive sheet");
    assert_eq!(first.len(), 8);
    assert_eq!(literal(first, "A1"), &CellValue::Text("Hello, 世界".into()));
    assert_eq!(
        literal(first, "B1"),
        &CellValue::Text("inline value".into())
    );
    assert_eq!(literal(first, "C1"), &CellValue::Logical(true));
    assert_eq!(
        literal(first, "D1"),
        &CellValue::Error(crate::ExcelError::DivisionByZero)
    );
    let CellValue::Number(date) = literal(first, "E1") else {
        panic!("date serial must remain numeric");
    };
    assert_eq!(date.get(), 45_292.0);
    assert_eq!(cell(first, "E1").number_format().id(), 164);
    assert_eq!(
        cell(first, "E1").number_format().kind(),
        NumberFormatKind::Date
    );
    assert_eq!(
        cell(first, "F1").number_format().kind(),
        NumberFormatKind::Duration
    );
    assert_eq!(number(literal(first, "G1")), 42.5);
    let CellContent::Formula(formula) = cell(first, "H1").content() else {
        panic!("H1 formula");
    };
    assert_eq!(formula.text().expect("formula text").as_str(), "1+1");
    let SavedResult::Present(value) = formula.saved_result() else {
        panic!("H1 saved result");
    };
    assert_eq!(number(value), 2.0);
    assert!(
        first.cell(address("I1")).is_none(),
        "empty styled cell stays sparse"
    );
}

#[test]
fn bytes_adapter_records_the_input_kind() {
    let archive = build_archive(SHEET_ONE, SHARED_STRINGS);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("byte workbook");
    assert_eq!(snapshot.source().kind(), WorkbookSourceKind::Bytes);
}

#[test]
fn semantic_limits_and_invalid_shared_string_indexes_are_rejected() {
    let archive = build_archive(SHEET_ONE, SHARED_STRINGS);
    let limits = ReadLimits::default()
        .with_max_sheets(1)
        .expect("nonzero sheet limit");
    let error =
        read_xlsx(Cursor::new(&archive), ReadOptions::new(limits)).expect_err("sheet limit");
    assert_eq!(error.code(), XlsxErrorCode::TooManySheets);

    let limits = ReadLimits::default()
        .with_max_cells_per_sheet(1)
        .expect("nonzero cell limit");
    let error = read_xlsx(Cursor::new(&archive), ReadOptions::new(limits)).expect_err("cell limit");
    assert_eq!(error.code(), XlsxErrorCode::TooManyCellsInSheet);

    let limits = ReadLimits::default()
        .with_max_total_cells(1)
        .expect("nonzero total cell limit");
    let error =
        read_xlsx(Cursor::new(&archive), ReadOptions::new(limits)).expect_err("total cell limit");
    assert_eq!(error.code(), XlsxErrorCode::TooManyCells);

    let invalid_sheet = SHEET_ONE.replacen("<v>0</v>", "<v>99</v>", 1);
    let error = read_xlsx(
        Cursor::new(build_archive(&invalid_sheet, SHARED_STRINGS)),
        ReadOptions::default(),
    )
    .expect_err("shared string index");
    assert_eq!(error.code(), XlsxErrorCode::InvalidCellValue);
}

#[test]
fn shared_string_budgets_apply_to_decoded_rich_text() {
    let archive = build_archive(SHEET_ONE, SHARED_STRINGS);
    let limits = ReadLimits::default()
        .with_max_shared_string_bytes(5)
        .expect("nonzero string limit");
    let error =
        read_xlsx(Cursor::new(archive), ReadOptions::new(limits)).expect_err("rich string limit");
    assert_eq!(error.code(), XlsxErrorCode::SharedStringTooLarge);

    let archive = build_archive(SHEET_ONE, SHARED_STRINGS);
    let limits = ReadLimits::default()
        .with_max_total_shared_string_bytes(5)
        .expect("nonzero total string limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(limits))
        .expect_err("total rich string limit");
    assert_eq!(error.code(), XlsxErrorCode::TotalSharedStringsTooLarge);

    let archive = build_archive(
        SHEET_ONE,
        &SHARED_STRINGS.replace("uniqueCount=\"1\"", "uniqueCount=\"2\""),
    );
    let limits = ReadLimits::default()
        .with_max_shared_strings(1)
        .expect("nonzero shared string count limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(limits))
        .expect_err("declared shared string count limit");
    assert_eq!(error.code(), XlsxErrorCode::TooManySharedStrings);
}

#[test]
fn duplicate_sheet_data_is_rejected() {
    let duplicate = SHEET_ONE.replace(
        "</sheetData>",
        "</sheetData><sheetData><row r=\"2\"><c r=\"A2\"><v>1</v></c></row></sheetData>",
    );
    let error = read_xlsx(
        Cursor::new(build_archive(&duplicate, SHARED_STRINGS)),
        ReadOptions::default(),
    )
    .expect_err("duplicate sheet data");
    assert_eq!(error.code(), XlsxErrorCode::InvalidWorksheet);
}

#[test]
fn empty_sheet_with_self_closing_sheet_data_is_read() {
    let empty = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <dimension ref="A1"/>
  <sheetData/>
</worksheet>"#;
    let snapshot = read_xlsx(
        Cursor::new(build_archive(empty, SHARED_STRINGS)),
        ReadOptions::default(),
    )
    .expect("a workbook with an empty sheet must be readable");
    assert_eq!(
        snapshot.sheet_by_name("First").expect("empty sheet").len(),
        0
    );
    assert_eq!(
        snapshot.sheet_by_name("Second").expect("data sheet").len(),
        1
    );
}

#[test]
fn duplicate_self_closing_sheet_data_is_rejected() {
    for worksheet in [
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData/>
  <sheetData/>
</worksheet>"#,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData></sheetData>
  <sheetData/>
</worksheet>"#,
    ] {
        let error = read_xlsx(
            Cursor::new(build_archive(worksheet, SHARED_STRINGS)),
            ReadOptions::default(),
        )
        .expect_err("duplicate sheet data");
        assert_eq!(error.code(), XlsxErrorCode::InvalidWorksheet);
    }
}

#[test]
fn document_mode_captures_frozen_panes_while_snapshot_mode_ignores_them() {
    let sheet = SHEET_ONE.replace(
        "<sheetData>",
        r#"<sheetViews><sheetView workbookViewId="0"><pane xSplit="3" ySplit="1" topLeftCell="D2" activePane="bottomRight" state="frozen"/></sheetView></sheetViews><sheetData>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("snapshot");
    assert_eq!(snapshot.sheet_by_name("First").expect("sheet").len(), 8);

    let document =
        open_xlsx_document_bytes(&archive, OpenOptions::default()).expect("document mode");
    let pane = document
        .presentation()
        .frozen_pane(SheetId::new(1).expect("sheet"))
        .expect("frozen pane");
    assert_eq!(pane.frozen_rows(), 1);
    assert_eq!(pane.frozen_columns(), 3);

    let scrolled = sheet.replace("topLeftCell=\"D2\"", "topLeftCell=\"D23\"");
    let scrolled_archive = build_archive(&scrolled, SHARED_STRINGS);
    let scrolled_document =
        open_xlsx_document_bytes(&scrolled_archive, OpenOptions::default()).expect("scrolled pane");
    assert_eq!(
        scrolled_document
            .presentation()
            .frozen_pane(SheetId::new(1).expect("sheet")),
        Some(pane)
    );

    let omitted = sheet.replace(r#" topLeftCell="D2""#, "");
    open_xlsx_document_bytes(
        &build_archive(&omitted, SHARED_STRINGS),
        OpenOptions::default(),
    )
    .expect("optional top-left cell");

    let malformed = sheet.replace("topLeftCell=\"D2\"", "topLeftCell=\"C2\"");
    let malformed_archive = build_archive(&malformed, SHARED_STRINGS);
    read_xlsx_bytes(&malformed_archive, ReadOptions::default())
        .expect("snapshot ignores presentation-only inconsistency");
    let error = open_xlsx_document_bytes(&malformed_archive, OpenOptions::default())
        .expect_err("document validates panes");
    assert_eq!(error.code(), XlsxErrorCode::InvalidFrozenPane);
}

#[test]
fn document_writer_clears_frozen_pane_without_dropping_view_siblings() {
    let sheet = SHEET_ONE.replace(
        "<sheetData>",
        r#"<sheetViews><sheetView workbookViewId="0" showGridLines="0"><pane xSplit="3" ySplit="1" topLeftCell="D2" activePane="bottomRight" state="frozen"/><selection pane="bottomRight" activeCell="D2" sqref="D2"/></sheetView></sheetViews><sheetData>"#,
    );
    let source = build_archive(&sheet, SHARED_STRINGS);
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let sheet_id = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    let semantic_revision = draft.semantic_revision();
    draft.clear_frozen_pane(sheet_id).expect("clear pane");
    assert_eq!(draft.semantic_revision(), semantic_revision);
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert_eq!(reopened.presentation().frozen_pane(sheet_id), None);

    let worksheet = archive_text(output.bytes(), "xl/worksheets/sheet1.xml");
    assert!(worksheet.contains(r#"showGridLines="0""#), "{worksheet}");
    assert!(worksheet.contains(r#"activeCell="D2""#), "{worksheet}");
    assert!(!worksheet.contains("<pane"), "{worksheet}");
    assert!(!worksheet.contains(r#"selection pane="#), "{worksheet}");
}

#[test]
fn document_mode_reads_shared_and_inline_phonetics_without_changing_base_text() {
    let shared_strings = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
  <si><t>明日😀</t><rPh sb="0" eb="2"><t>あした</t></rPh><phoneticPr fontId="4" type="Hiragana" alignment="center"/></si>
</sst>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <cols><col min="1" max="2" phonetic="1"/></cols>
  <phoneticPr fontId="0" type="noConversion" alignment="left"/>
  <sheetData><row r="1" ph="0">
    <c r="A1" t="s" ph="1"><v>0</v></c>
    <c r="B1" t="inlineStr"><is><t>学校</t><rPh sb="0" eb="2"><t>がっこう</t></rPh><phoneticPr fontId="0"/></is></c>
  </row></sheetData>
</worksheet>"#;
    let archive = build_archive(sheet, shared_strings);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("snapshot");
    let first = snapshot.sheet_by_name("First").expect("first");
    assert_eq!(literal(first, "A1"), &CellValue::Text("明日😀".into()));
    assert_eq!(literal(first, "B1"), &CellValue::Text("学校".into()));

    let document =
        open_xlsx_document_bytes(&archive, OpenOptions::default()).expect("document mode");
    let sheet_id = SheetId::new(1).expect("sheet");
    let shared = document
        .presentation()
        .cell_phonetics(sheet_id, address("A1"))
        .expect("shared phonetics");
    assert_eq!(shared.runs().len(), 1);
    assert_eq!(shared.runs()[0].text(), "あした");
    assert_eq!(shared.runs()[0].base_range().start_utf16(), 0);
    assert_eq!(shared.runs()[0].base_range().end_utf16(), 2);
    let properties = shared.properties().expect("properties");
    assert_eq!(properties.font_id(), 4);
    assert_eq!(properties.effective_font_id(), 0);
    assert_eq!(properties.phonetic_type(), Some(PhoneticType::Hiragana));
    assert_eq!(properties.alignment(), Some(PhoneticAlignment::Center));
    assert_eq!(shared.explicit_cell_visibility(), Some(true));
    assert_eq!(shared.explicit_row_visibility(), Some(false));
    assert_eq!(shared.explicit_column_visibility(), Some(true));
    assert!(shared.effective_visibility());

    let inline = document
        .presentation()
        .cell_phonetics(sheet_id, address("B1"))
        .expect("inline phonetics");
    assert_eq!(inline.runs()[0].text(), "がっこう");
    assert!(!inline.effective_visibility());
    assert_eq!(
        document
            .presentation()
            .worksheet_phonetic_properties(sheet_id)
            .expect("worksheet properties")
            .phonetic_type(),
        Some(PhoneticType::NoConversion)
    );
}

#[test]
fn phonetic_validation_and_reference_limits_are_document_only() {
    let invalid = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
  <si><t>A</t><rPh sb="0" eb="2"><t>えー</t></rPh></si>
</sst>"#;
    let archive = build_archive(SHEET_ONE, invalid);
    read_xlsx_bytes(&archive, ReadOptions::default()).expect("snapshot ignores invalid range");
    let error = open_xlsx_document_bytes(&archive, OpenOptions::default())
        .expect_err("document validates range");
    assert_eq!(error.code(), XlsxErrorCode::InvalidPhoneticMetadata);

    let annotated = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="1">
  <si><t>A</t><rPh sb="0" eb="1"><t>えー</t></rPh></si>
</sst>"#;
    let limits = ReadLimits::default()
        .with_max_annotated_cells(1)
        .expect("limit");
    let error = open_xlsx_document_bytes(
        &build_archive(SHEET_ONE, annotated),
        OpenOptions::new(ReadOptions::new(limits)),
    )
    .expect_err("two sheets reference the same annotated shared item");
    assert_eq!(error.code(), XlsxErrorCode::TooManyAnnotatedCells);

    let shared_once_limits = ReadLimits::default()
        .with_max_total_phonetic_runs(1)
        .expect("limit");
    open_xlsx_document_bytes(
        &build_archive(SHEET_ONE, annotated),
        OpenOptions::new(ReadOptions::new(shared_once_limits)),
    )
    .expect("shared annotation storage is charged once");
}

#[test]
fn overlapping_source_runs_are_preserved_with_a_diagnostic() {
    let shared = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="1">
  <si><t>ABC</t><rPh sb="0" eb="2"><t>first</t></rPh><rPh sb="1" eb="3"><t>second</t></rPh></si>
</sst>"#;
    let document =
        open_xlsx_document_bytes(&build_archive(SHEET_ONE, shared), OpenOptions::default())
            .expect("overlap is a compatibility diagnostic");
    let phonetics = document
        .presentation()
        .cell_phonetics(SheetId::new(1).expect("sheet"), address("A1"))
        .expect("phonetics");
    assert_eq!(phonetics.runs()[0].text(), "first");
    assert_eq!(phonetics.runs()[1].text(), "second");
    assert!(
        document
            .presentation()
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "xlsx.phonetic.overlap" })
    );
}

#[test]
fn document_writer_de_shares_one_edited_phonetic_cell() {
    let shared = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="1">
  <si><t>明日</t><rPh sb="0" eb="2"><t>あした</t></rPh><phoneticPr fontId="0"/></si>
</sst>"#;
    let source = build_archive(SHEET_ONE, shared);
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let first_sheet = SheetId::new(1).expect("sheet");
    let second_sheet = SheetId::new(2).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_phonetics(
            first_sheet,
            address("A1"),
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 1).expect("range"), "あ").expect("run"),
                PhoneticRun::new(PhoneticTextRange::new(1, 2).expect("range"), "す").expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("edit one cell");
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");

    let edited = reopened
        .presentation()
        .cell_phonetics(first_sheet, address("A1"))
        .expect("edited");
    assert_eq!(
        edited
            .runs()
            .iter()
            .map(|run| run.text())
            .collect::<Vec<_>>(),
        vec!["あ", "す"]
    );
    let untouched = reopened
        .presentation()
        .cell_phonetics(second_sheet, address("A2"))
        .expect("untouched alias");
    assert_eq!(untouched.runs()[0].text(), "あした");
}

#[test]
fn document_writer_rejects_phonetic_edits_that_would_flatten_rich_text() {
    let source = build_archive(SHEET_ONE, SHARED_STRINGS);
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let sheet_id = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_phonetics(
            sheet_id,
            address("A1"),
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 5).expect("range"), "hello")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("typed edit");
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let error = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect_err("rich text must not be flattened");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
}

#[test]
fn reads_merged_ranges_sorted_by_top_left_address() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells count="3"><mergeCell ref="D5:E6"/><mergeCell ref="A10:C12"/><mergeCell ref="A2:B3"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let snapshot =
        read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("merged workbook");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    let ranges: Vec<String> = first
        .merged_ranges()
        .iter()
        .map(|range| format!("{}:{}", range.start(), range.end()))
        .collect();
    assert_eq!(ranges, vec!["A2:B3", "D5:E6", "A10:C12"]);
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.code().as_str().starts_with("xlsx.merged_range")),
        "valid merges must not produce diagnostics"
    );
    let second = snapshot.sheet_by_name("Second").expect("sheet");
    assert!(second.merged_ranges().is_empty());
}

#[test]
fn merged_range_problems_become_diagnostics_and_are_dropped() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="B2:A1"/><mergeCell ref="NOPE"/><mergeCell ref="C3:C3"/><mergeCell ref="D4"/><mergeCell ref="A1:B2"/><mergeCell ref="B2:C3"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let snapshot =
        read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read must succeed");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    let ranges: Vec<String> = first
        .merged_ranges()
        .iter()
        .map(|range| format!("{}:{}", range.start(), range.end()))
        .collect();
    assert_eq!(ranges, vec!["A1:B2"]);
    let codes: Vec<&str> = snapshot
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str().starts_with("xlsx.merged_range"))
        .map(|diagnostic| diagnostic.code().as_str())
        .collect();
    assert_eq!(
        codes,
        vec![
            "xlsx.merged_range.invalid",
            "xlsx.merged_range.invalid",
            "xlsx.merged_range.single_cell",
            "xlsx.merged_range.single_cell",
            "xlsx.merged_range.overlap",
        ]
    );
}

#[test]
fn merged_range_sweep_keeps_row_disjoint_and_column_disjoint_ranges() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="A1:A5"/><mergeCell ref="B1:B5"/><mergeCell ref="A3:B3"/><mergeCell ref="A6:B6"/><mergeCell ref="C1:D2"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    let ranges: Vec<String> = first
        .merged_ranges()
        .iter()
        .map(|range| format!("{}:{}", range.start(), range.end()))
        .collect();
    assert_eq!(ranges, vec!["A1:A5", "B1:B5", "C1:D2", "A6:B6"]);
    let overlap_count = snapshot
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.merged_range.overlap")
        .count();
    assert_eq!(overlap_count, 1, "only A3:B3 overlaps a kept range");
}

#[test]
fn merged_range_budget_fails_the_read_with_a_dedicated_code() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="A1:B2"/><mergeCell ref="NOPE"/><mergeCell ref="D1:E2"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let limits = ReadLimits::default()
        .with_max_merged_ranges(2)
        .expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(limits))
        .expect_err("third declaration must exceed the budget");
    assert_eq!(error.code(), XlsxErrorCode::TooManyMergedRanges);
}

#[test]
fn merged_range_budget_accumulates_across_worksheets_and_admits_the_exact_limit() {
    let sheet_one = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="A1:B2"/><mergeCell ref="D1:E2"/></mergeCells></worksheet>"#,
    );
    let sheet_two = SHEET_TWO.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="A5:B6"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive_with_sheets(&sheet_one, &sheet_two, SHARED_STRINGS);
    let at_limit = ReadLimits::default()
        .with_max_merged_ranges(3)
        .expect("limit");
    let snapshot = read_xlsx(Cursor::new(archive.clone()), ReadOptions::new(at_limit))
        .expect("three declarations fit a limit of three");
    assert_eq!(
        snapshot
            .sheet_by_name("Second")
            .expect("sheet")
            .merged_ranges()
            .len(),
        1
    );
    let below = ReadLimits::default()
        .with_max_merged_ranges(2)
        .expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(below))
        .expect_err("the second sheet's declaration must exceed the workbook budget");
    assert_eq!(error.code(), XlsxErrorCode::TooManyMergedRanges);
}

#[test]
fn duplicate_merge_cells_elements_fail_the_read() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells><mergeCell ref="A1:B2"/></mergeCells><mergeCells><mergeCell ref="D1:E2"/></mergeCells></worksheet>"#,
    );
    let archive = build_archive(&sheet, SHARED_STRINGS);
    let error = read_xlsx(Cursor::new(archive), ReadOptions::default())
        .expect_err("duplicate mergeCells elements are structurally invalid");
    assert_eq!(error.code(), XlsxErrorCode::InvalidWorksheet);
}

const SHEET_WITH_TABLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c><c r="C1"><v>3</v></c></row></sheetData>
  <tableParts count="1"><tablePart r:id="rId7"/></tableParts>
</worksheet>"#;

const SHEET_ONE_TABLE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;

const TABLE_ONE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Sales" displayName="SalesDisplay" ref="A1:C4" totalsRowCount="1">
  <tableColumns count="3">
    <tableColumn id="1" name="Region"/>
    <tableColumn id="5" name="Amount" totalsRowFunction="sum"/>
    <tableColumn id="3" name="Note" totalsRowFunction="none"/>
  </tableColumns>
</table>"#;

const TABLE_WITH_METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Sales" displayName="SalesDisplay" ref="A1:C4" tableType="queryTable" headerRowCount="1" totalsRowCount="1" totalsRowShown="1">
  <autoFilter ref="A1:C3">
    <filterColumn colId="1"><filters><filter val="East"/></filters></filterColumn>
    <sortState ref="A2:C3" caseSensitive="1" sortMethod="stroke"><sortCondition ref="B2:B3" descending="1" sortBy="cellColor" dxfId="4"/></sortState>
  </autoFilter>
  <sortState ref="A2:C3" columnSort="1" sortMethod="pinYin"><sortCondition ref="A2:C2" sortBy="icon" iconSet="3Arrows" iconId="2"/></sortState>
  <tableColumns count="3">
    <tableColumn id="1" name="Region" totalsRowLabel="Total"/>
    <tableColumn id="5" name="Amount"><calculatedColumnFormula array="1">[@Amount]*2</calculatedColumnFormula></tableColumn>
    <tableColumn id="3" name="Note" totalsRowFunction="custom"><totalsRowFormula>SUBTOTAL(109,[Amount])</totalsRowFormula></tableColumn>
  </tableColumns>
  <tableStyleInfo name="TableStyleMedium2" showFirstColumn="1" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/>
</table>"#;

fn table_content_types() -> String {
    CONTENT_TYPES.replace(
        "</Types>",
        r#"<Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/tables/table2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/tables/table3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/></Types>"#,
    )
}

fn build_table_archive(sheet_one: &str, extra_parts: &[(&str, &str)]) -> Vec<u8> {
    build_table_archive_with_workbook(WORKBOOK, sheet_one, extra_parts)
}

fn build_table_archive_with_workbook(
    workbook: &str,
    sheet_one: &str,
    extra_parts: &[(&str, &str)],
) -> Vec<u8> {
    let content_types = table_content_types();
    let mut entries: Vec<(&str, &str)> = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/styles.xml", STYLES),
        ("xl/sharedStrings.xml", SHARED_STRINGS),
        ("xl/worksheets/sheet1.xml", sheet_one),
        ("xl/worksheets/sheet2.xml", SHEET_TWO),
    ];
    entries.extend_from_slice(extra_parts);
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, contents) in entries {
            writer
                .start_file(name, options)
                .expect("start fixture part");
            writer
                .write_all(contents.as_bytes())
                .expect("write fixture part");
        }
        writer.finish().expect("finish fixture archive");
    }
    output.into_inner()
}

fn read_two_table_archive(second_table: &str) -> crate::WorkbookSnapshot {
    let sheet = SHEET_WITH_TABLE.replace(
        r#"<tableParts count="1"><tablePart r:id="rId7"/></tableParts>"#,
        r#"<tableParts count="2"><tablePart r:id="rId7"/><tablePart r:id="rId8"/></tableParts>"#,
    );
    let relationships = SHEET_ONE_TABLE_RELS.replace(
        "</Relationships>",
        r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table2.xml"/></Relationships>"#,
    );
    let archive = build_table_archive(
        &sheet,
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                relationships.as_str(),
            ),
            ("xl/tables/table1.xml", TABLE_ONE),
            ("xl/tables/table2.xml", second_table),
        ],
    );
    read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read table archive")
}

fn read_three_table_archive(second_table: &str, third_table: &str) -> crate::WorkbookSnapshot {
    let sheet = SHEET_WITH_TABLE.replace(
        r#"<tableParts count="1"><tablePart r:id="rId7"/></tableParts>"#,
        r#"<tableParts count="3"><tablePart r:id="rId7"/><tablePart r:id="rId8"/><tablePart r:id="rId9"/></tableParts>"#,
    );
    let relationships = SHEET_ONE_TABLE_RELS.replace(
        "</Relationships>",
        r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table2.xml"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table3.xml"/></Relationships>"#,
    );
    let archive = build_table_archive(
        &sheet,
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                relationships.as_str(),
            ),
            ("xl/tables/table1.xml", TABLE_ONE),
            ("xl/tables/table2.xml", second_table),
            ("xl/tables/table3.xml", third_table),
        ],
    );
    read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read table archive")
}

#[test]
fn reads_table_metadata_through_worksheet_relationships() {
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_ONE),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("table workbook");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    let table = &first.tables()[0];
    assert_eq!(table.id().get(), 1);
    assert_eq!(table.name().as_str(), "Sales");
    assert_eq!(table.display_name().as_str(), "SalesDisplay");
    assert_eq!(table.range().start().to_string(), "A1");
    assert_eq!(table.range().end().to_string(), "C4");
    assert_eq!(table.header_row_count(), 1);
    assert_eq!(table.totals_row_count(), 1);
    let columns: Vec<(u32, &str, Option<crate::TotalsRowFunction>)> = table
        .columns()
        .iter()
        .map(|column| (column.id(), column.name(), column.totals_row_function()))
        .collect();
    assert_eq!(
        columns,
        vec![
            (1, "Region", None),
            (5, "Amount", Some(crate::TotalsRowFunction::Sum)),
            (3, "Note", None),
        ]
    );
    assert_eq!(
        snapshot
            .table("salesdisplay")
            .expect("global lookup")
            .display_name()
            .as_str(),
        "SalesDisplay"
    );
    assert!(
        snapshot.table("Sales").is_none(),
        "programmatic name is not a workbook lookup key"
    );
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.code().as_str().starts_with("xlsx.table")),
        "a valid table must not produce diagnostics"
    );
}

#[test]
fn reads_complete_table_metadata_and_records_the_source_part() {
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_WITH_METADATA),
        ],
    );
    let document =
        open_xlsx_document_bytes(&archive, OpenOptions::default()).expect("table document");
    let table_id = crate::TableId::new(1).expect("table id");
    assert_eq!(
        document
            .table_part(table_id)
            .expect("accepted table source part")
            .as_str(),
        "xl/tables/table1.xml"
    );
    let table = document
        .workbook()
        .table("SalesDisplay")
        .expect("complete table");
    assert_eq!(table.table_type(), crate::TableType::QueryTable);
    assert!(table.totals_row_shown());
    assert!(!table.has_opaque_metadata());

    let filter = table.auto_filter().expect("auto filter");
    assert_eq!(filter.range().start().to_string(), "A1");
    assert_eq!(filter.range().end().to_string(), "C3");
    assert_eq!(filter.filter_columns().len(), 1);
    assert_eq!(filter.filter_columns()[0].column_id(), 1);
    match filter.filter_columns()[0]
        .criteria()
        .expect("filter criteria")
    {
        crate::TableFilterCriteria::Values(filters) => {
            assert_eq!(
                filters.items(),
                &[crate::TableFilterItem::Value(Some("East".into()))]
            );
        }
        other => panic!("unexpected filter criteria: {other:?}"),
    }
    let filter_sort = filter.sort_state().expect("nested sort");
    assert!(filter_sort.case_sensitive());
    assert!(!filter_sort.column_sort());
    assert_eq!(
        filter_sort.sort_method(),
        Some(crate::TableSortMethod::Stroke)
    );
    assert_eq!(
        filter_sort
            .conditions()
            .iter()
            .map(crate::TableSortCondition::range)
            .collect::<Vec<_>>(),
        &[crate::CellRange::new(address("B2"), address("B3")).expect("condition range")]
    );
    assert!(filter_sort.conditions()[0].descending());
    assert_eq!(
        filter_sort.conditions()[0].sort_by(),
        Some(crate::TableSortBy::CellColor)
    );
    assert_eq!(
        filter_sort.conditions()[0].differential_format_id(),
        Some(4)
    );

    let table_sort = table.sort_state().expect("table sort");
    assert!(!table_sort.case_sensitive());
    assert!(table_sort.column_sort());
    assert_eq!(
        table_sort.sort_method(),
        Some(crate::TableSortMethod::PinYin)
    );
    assert_eq!(
        table_sort
            .conditions()
            .iter()
            .map(crate::TableSortCondition::range)
            .collect::<Vec<_>>(),
        &[crate::CellRange::new(address("A2"), address("C2")).expect("condition range")]
    );
    assert_eq!(
        table_sort.conditions()[0].sort_by(),
        Some(crate::TableSortBy::Icon)
    );
    assert_eq!(
        table_sort.conditions()[0].icon_set(),
        Some(crate::TableIconSet::ThreeArrows)
    );
    assert_eq!(table_sort.conditions()[0].icon_id(), Some(2));

    let columns = table.columns();
    assert_eq!(columns[0].totals_row_label(), Some("Total"));
    let calculated = columns[1]
        .calculated_column_formula()
        .expect("calculated column formula");
    assert_eq!(calculated.text().as_str(), "[@Amount]*2");
    assert!(calculated.is_array());
    assert_eq!(
        columns[2]
            .totals_row_formula()
            .expect("totals formula")
            .text()
            .as_str(),
        "SUBTOTAL(109,[Amount])"
    );
    assert_eq!(
        columns[2].totals_row_function(),
        Some(crate::TotalsRowFunction::Custom)
    );

    let style = table.style_info().expect("table style");
    assert_eq!(style.name(), Some("TableStyleMedium2"));
    assert!(style.show_first_column());
    assert!(!style.show_last_column());
    assert!(style.show_row_stripes());
    assert!(!style.show_column_stripes());
}

#[test]
fn reads_each_typed_table_filter_criteria_variant() {
    let read_criteria = |replacement: &str| {
        let table =
            TABLE_WITH_METADATA.replace(r#"<filters><filter val="East"/></filters>"#, replacement);
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("table workbook");
        snapshot
            .table("SalesDisplay")
            .expect("table")
            .auto_filter()
            .expect("auto filter")
            .filter_columns()[0]
            .criteria()
            .expect("criteria")
            .clone()
    };

    match read_criteria(
        r#"<filters blank="1" calendarType="gregorian"><filter val="East"/><dateGroupItem year="2026" month="7" day="31" dateTimeGrouping="day"/></filters>"#,
    ) {
        crate::TableFilterCriteria::Values(filters) => {
            assert!(filters.blank());
            assert_eq!(
                filters.calendar_type(),
                Some(crate::TableCalendarType::Gregorian)
            );
            assert_eq!(filters.items().len(), 2);
            let crate::TableFilterItem::DateGroup(date) = &filters.items()[1] else {
                panic!("expected grouped date");
            };
            assert_eq!(date.year(), 2026);
            assert_eq!(date.month(), Some(7));
            assert_eq!(date.day(), Some(31));
            assert_eq!(date.grouping(), crate::TableDateTimeGrouping::Day);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<filters><filter/></filters>"#) {
        crate::TableFilterCriteria::Values(filters) => {
            assert_eq!(filters.items(), &[crate::TableFilterItem::Value(None)]);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(
        r#"<customFilters and="1"><customFilter operator="greaterThan" val="10"/></customFilters>"#,
    ) {
        crate::TableFilterCriteria::Custom(filters) => {
            assert!(filters.and());
            assert_eq!(
                filters.filters()[0].operator(),
                Some(crate::TableCustomFilterOperator::GreaterThan)
            );
            assert_eq!(filters.filters()[0].value(), Some("10"));
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<customFilters><customFilter/></customFilters>"#) {
        crate::TableFilterCriteria::Custom(filters) => {
            assert_eq!(filters.filters()[0].operator(), None);
            assert_eq!(filters.filters()[0].value(), None);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<dynamicFilter type="thisMonth" val="1" maxVal="2"/>"#) {
        crate::TableFilterCriteria::Dynamic(filter) => {
            assert_eq!(filter.kind(), crate::TableDynamicFilterType::ThisMonth);
            assert_eq!(
                filter.value().map(crate::TableNumericValue::as_str),
                Some("1")
            );
            assert_eq!(
                filter.max_value().map(crate::TableNumericValue::as_str),
                Some("2")
            );
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<dynamicFilter type="today"/>"#) {
        crate::TableFilterCriteria::Dynamic(filter) => {
            assert_eq!(filter.value(), None);
            assert_eq!(filter.max_value(), None);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(
        r#"<dynamicFilter type="today" valIso="2026-07-31T00:00:00Z" maxValIso="2026-08-01T00:00:00Z"/>"#,
    ) {
        crate::TableFilterCriteria::Dynamic(filter) => {
            assert_eq!(
                filter.iso_value().map(crate::TableDateTimeValue::as_str),
                Some("2026-07-31T00:00:00Z")
            );
            assert_eq!(
                filter
                    .max_iso_value()
                    .map(crate::TableDateTimeValue::as_str),
                Some("2026-08-01T00:00:00Z")
            );
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<dynamicFilter type="null"/>"#) {
        crate::TableFilterCriteria::Dynamic(filter) => {
            assert_eq!(filter.kind(), crate::TableDynamicFilterType::Null);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<colorFilter dxfId="4" cellColor="0"/>"#) {
        crate::TableFilterCriteria::Color(filter) => {
            assert_eq!(filter.differential_format_id(), Some(4));
            assert!(!filter.cell_color());
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<colorFilter/>"#) {
        crate::TableFilterCriteria::Color(filter) => {
            assert_eq!(filter.differential_format_id(), None);
            assert!(filter.cell_color());
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<iconFilter iconSet="3Arrows" iconId="2"/>"#) {
        crate::TableFilterCriteria::Icon(filter) => {
            assert_eq!(filter.icon_set(), crate::TableIconSet::ThreeArrows);
            assert_eq!(filter.icon_id(), Some(2));
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<iconFilter iconSet="3Arrows"/>"#) {
        crate::TableFilterCriteria::Icon(filter) => {
            assert_eq!(filter.icon_id(), None);
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
    match read_criteria(r#"<top10 top="0" percent="1" val="10" filterVal="42"/>"#) {
        crate::TableFilterCriteria::Top(filter) => {
            assert!(!filter.top());
            assert!(filter.percent());
            assert_eq!(filter.value().as_str(), "10");
            assert_eq!(
                filter.filter_value().map(crate::TableNumericValue::as_str),
                Some("42")
            );
        }
        other => panic!("unexpected criteria: {other:?}"),
    }
}

#[test]
fn invalid_table_filter_and_sort_scalars_fail_closed() {
    let criteria = r#"<filters><filter val="East"/></filters>"#;
    let invalid_criteria = [
        r#"<filters calendarType="future"><filter val="East"/></filters>"#,
        r#"<filters><dateGroupItem year="65536" dateTimeGrouping="year"/></filters>"#,
        r#"<filters><dateGroupItem year="2026" month="13" dateTimeGrouping="month"/></filters>"#,
        r#"<filters><dateGroupItem year="2026" dateTimeGrouping="year"/><filter val="East"/></filters>"#,
        r#"<customFilters/>"#,
        r#"<customFilters><customFilter/><customFilter/><customFilter/></customFilters>"#,
        r#"<customFilters><customFilter operator="future"/></customFilters>"#,
        r#"<dynamicFilter type="future"/>"#,
        r#"<dynamicFilter type="today" val="inf"/>"#,
        r#"<dynamicFilter type="today" valIso="2026-02-30T00:00:00Z"/>"#,
        r#"<iconFilter iconSet="3Arrows" iconId="3"/>"#,
        r#"<top10 val="."/>"#,
    ];
    let mut invalid_tables: Vec<String> = invalid_criteria
        .into_iter()
        .map(|replacement| TABLE_WITH_METADATA.replace(criteria, replacement))
        .collect();
    invalid_tables.extend([
        TABLE_WITH_METADATA.replace(r#"sortMethod="stroke""#, r#"sortMethod="future""#),
        TABLE_WITH_METADATA.replace(r#"sortBy="cellColor""#, r#"sortBy="future""#),
        TABLE_WITH_METADATA.replace(
            r#"sortBy="cellColor" dxfId="4""#,
            r#"sortBy="cellColor" dxfId="4" iconSet="3Arrows""#,
        ),
        TABLE_WITH_METADATA.replace(r#"iconId="2""#, r#"iconId="3""#),
    ]);

    for invalid_table in invalid_tables {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("bounded read");
        assert!(snapshot.table("SalesDisplay").is_none(), "{invalid_table}");
        assert_eq!(
            snapshot
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
                .count(),
            1,
            "{invalid_table}"
        );
    }
}

#[test]
fn canonical_writer_reopens_each_self_contained_typed_filter_criteria_variant() {
    for criteria in [
        r#"<filters blank="1" calendarType="gregorian"><filter val="East"/><dateGroupItem year="2026" month="7" day="31" dateTimeGrouping="day"/></filters>"#,
        r#"<filters><filter/></filters>"#,
        r#"<customFilters and="1"><customFilter operator="greaterThan" val="10"/></customFilters>"#,
        r#"<customFilters><customFilter/></customFilters>"#,
        r#"<dynamicFilter type="thisMonth" val="1" maxVal="2"/>"#,
        r#"<dynamicFilter type="today"/>"#,
        r#"<dynamicFilter type="today" valIso="2026-07-31T00:00:00Z" maxValIso="2026-08-01T00:00:00Z"/>"#,
        r#"<dynamicFilter type="null"/>"#,
        r#"<colorFilter/>"#,
        r#"<iconFilter iconSet="3Arrows" iconId="2"/>"#,
        r#"<iconFilter iconSet="3Arrows"/>"#,
        r#"<top10 top="0" percent="1" val="10" filterVal="42"/>"#,
    ] {
        let table = TABLE_WITH_METADATA
            .replace(r#"tableType="queryTable""#, r#"tableType="worksheet""#)
            .replace(
                r#"<sortCondition ref="B2:B3" descending="1" sortBy="cellColor" dxfId="4"/>"#,
                r#"<sortCondition ref="B2:B3" descending="1" sortBy="value"/>"#,
            )
            .replace(r#"<filters><filter val="East"/></filters>"#, criteria);
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("table workbook");
        let expected = snapshot.table("SalesDisplay").expect("table").clone();
        let mut draft = WorkbookDraft::from_snapshot_for_test(snapshot);
        for (cell, header) in [("A1", "Region"), ("B1", "Amount"), ("C1", "Note")] {
            draft
                .set_cell_value(
                    SheetId::new(1).expect("sheet"),
                    address(cell),
                    CellValue::Text(header.to_owned()),
                )
                .expect("header");
        }
        let calculation =
            calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect("canonical write");
        let reopened =
            open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
        assert_eq!(
            reopened
                .workbook()
                .table("SalesDisplay")
                .expect("reopened table"),
            &expected
        );
    }
}

#[test]
fn strict_dynamic_filters_accept_iso_bounds_and_reject_transitional_max_val() {
    const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    let criteria = r#"<filters><filter val="East"/></filters>"#;
    let strict_table = TABLE_WITH_METADATA.replace(TRANSITIONAL, STRICT);
    let valid = strict_table.replace(
        criteria,
        r#"<dynamicFilter type="today" valIso="2026-07-31T00:00:00Z" maxValIso="2026-08-01T00:00:00Z"/>"#,
    );
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &valid),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("strict table");
    let table = snapshot.table("SalesDisplay").expect("strict table");
    assert!(!table.has_opaque_metadata());
    let crate::TableFilterCriteria::Dynamic(filter) =
        table.auto_filter().expect("auto filter").filter_columns()[0]
            .criteria()
            .expect("criteria")
    else {
        panic!("expected dynamic filter");
    };
    assert_eq!(
        filter.iso_value().map(crate::TableDateTimeValue::as_str),
        Some("2026-07-31T00:00:00Z")
    );
    assert_eq!(
        filter
            .max_iso_value()
            .map(crate::TableDateTimeValue::as_str),
        Some("2026-08-01T00:00:00Z")
    );

    let invalid = strict_table.replace(
        criteria,
        r#"<dynamicFilter type="today" valIso="2026-07-31T00:00:00Z" maxVal="46235"/>"#,
    );
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &invalid),
        ],
    );
    let snapshot =
        read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("bounded strict read");
    assert!(snapshot.table("SalesDisplay").is_none());
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
    );
}

#[test]
fn ambiguous_dynamic_filter_values_require_source_linked_preservation() {
    for replacement in [
        r#"<dynamicFilter type="aboveAverage"/>"#,
        r#"<dynamicFilter type="aboveAverage" val="10" valIso="2026-07-31T00:00:00Z"/>"#,
        r#"<dynamicFilter type="Q1" val="1"/>"#,
    ] {
        let table =
            TABLE_WITH_METADATA.replace(r#"<filters><filter val="East"/></filters>"#, replacement);
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("tolerant read");
        assert!(
            snapshot
                .table("SalesDisplay")
                .expect("source-preserved table")
                .has_opaque_metadata(),
            "{replacement}"
        );
    }
}

#[test]
fn prefixed_transitional_and_strict_table_fragments_remain_canonical() {
    const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";

    let mut prefixed = TABLE_WITH_METADATA
        .replace(
            &format!(r#"<table xmlns="{TRANSITIONAL}""#),
            &format!(r#"<x:table xmlns:x="{TRANSITIONAL}" xmlns="{TRANSITIONAL}""#),
        )
        .replace("</table>", "</x:table>")
        .replace(r#"tableType="queryTable""#, r#"tableType="worksheet""#)
        .replace(
            r#"<sortCondition ref="B2:B3" descending="1" sortBy="cellColor" dxfId="4"/>"#,
            r#"<sortCondition ref="B2:B3" descending="1" sortBy="value"/>"#,
        );
    for element in [
        "autoFilter",
        "filterColumn",
        "filters",
        "filter",
        "sortState",
        "sortCondition",
    ] {
        prefixed = prefixed
            .replace(&format!("<{element}"), &format!("<x:{element}"))
            .replace(&format!("</{element}>"), &format!("</x:{element}>"));
    }

    for table_xml in [prefixed.clone(), prefixed.replace(TRANSITIONAL, STRICT)] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table_xml),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("prefixed table");
        let expected = snapshot.table("SalesDisplay").expect("table").clone();
        assert!(
            !expected.has_opaque_metadata(),
            "recognized namespace prefixes are semantic, not opaque"
        );

        let mut draft = WorkbookDraft::from_snapshot_for_test(snapshot);
        for (cell, header) in [("A1", "Region"), ("B1", "Amount"), ("C1", "Note")] {
            draft
                .set_cell_value(
                    SheetId::new(1).expect("sheet"),
                    address(cell),
                    CellValue::Text(header.to_owned()),
                )
                .expect("header");
        }
        let calculation =
            calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect("canonical write");
        let reopened =
            open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
        assert_eq!(
            reopened
                .workbook()
                .table("SalesDisplay")
                .expect("reopened table"),
            &expected
        );
    }
}

#[test]
fn auto_filter_without_ref_inherits_the_table_range_without_totals_rows() {
    let table_xml = TABLE_WITH_METADATA
        .replace(r#"tableType="queryTable""#, r#"tableType="worksheet""#)
        .replace(r#"<autoFilter ref="A1:C3">"#, "<autoFilter>")
        .replace(r#"sortBy="cellColor" dxfId="4""#, r#"sortBy="value""#);
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &table_xml),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("table");
    let table = snapshot.table("SalesDisplay").expect("table");
    let filter = table.auto_filter().expect("auto filter");
    assert_eq!(filter.range().start().to_string(), "A1");
    assert_eq!(filter.range().end().to_string(), "C3");
    assert_eq!(filter.declared_range(), None);
    assert!(!table.has_opaque_metadata());

    let mut draft = WorkbookDraft::from_snapshot_for_test(snapshot);
    for (cell, header) in [("A1", "Region"), ("B1", "Amount"), ("C1", "Note")] {
        draft
            .set_cell_value(
                SheetId::new(1).expect("sheet"),
                address(cell),
                CellValue::Text(header.to_owned()),
            )
            .expect("header");
    }
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("canonical write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened
            .workbook()
            .table("SalesDisplay")
            .expect("reopened table")
            .auto_filter()
            .expect("auto filter")
            .declared_range(),
        None
    );
}

#[test]
fn unmodeled_table_metadata_is_marked_for_source_linked_preservation() {
    for table in [
        TABLE_WITH_METADATA.replace(r#"ref="A1:C4""#, r#"ref="A1:C4" published="1""#),
        TABLE_WITH_METADATA.replace(
            r#"<filter val="East"/>"#,
            r#"<filter val="East" future="1"/><futureFilter/>"#,
        ),
        TABLE_WITH_METADATA.replace("[@Amount]*2", "[@Amount]<future>IGNORED</future>*2"),
        TABLE_WITH_METADATA.replace(
            r#"<tableColumns count="3">"#,
            r#"<tableColumns count="3" future="1"><futureColumn/>"#,
        ),
        TABLE_WITH_METADATA.replace(
            r#"<autoFilter ref="A1:C3">"#,
            r#"<autoFilter ref="A1:C3">PAYLOAD<filter val="WrongLevel"/>"#,
        ),
        TABLE_WITH_METADATA.replace(
            r#"<tableColumns count="3">"#,
            "<![CDATA[PAYLOAD]]><tableColumns count=\"3\">",
        ),
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("table workbook");
        let table = snapshot.table("SalesDisplay").expect("table");
        assert!(table.has_opaque_metadata());
        assert_eq!(
            table.columns()[1]
                .calculated_column_formula()
                .expect("formula")
                .text()
                .as_str(),
            "[@Amount]*2",
            "nested metadata text must not become formula text"
        );
    }
}

#[test]
fn table_fragments_reject_undeclared_entities_and_document_level_text() {
    for invalid_table in [
        TABLE_WITH_METADATA.replace(
            r#"<filter val="East"/>"#,
            r#"<filter val="East">&bogus;</filter>"#,
        ),
        TABLE_WITH_METADATA.replace(r#"<table xmlns="#, r#"PAYLOAD<table xmlns="#),
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let error = read_xlsx(Cursor::new(archive), ReadOptions::default())
            .expect_err("invalid table XML must fail the read");
        assert_eq!(error.code(), XlsxErrorCode::InvalidXml);
    }
}

#[test]
fn table_xml_declarations_comments_and_whitespace_fail_closed() {
    let declaration = r#"<?xml version="1.0" encoding="UTF-8"?>"#;
    let body = TABLE_ONE
        .strip_prefix(declaration)
        .expect("fixture declaration");
    let invalid_tables = [
        format!("{declaration}\n{TABLE_ONE}"),
        format!(" \n{TABLE_ONE}"),
        format!("<!--before-->{TABLE_ONE}"),
        format!("<?probe value?>{TABLE_ONE}"),
        format!("<?XML value?>{TABLE_ONE}"),
        format!("<?1probe value?>{TABLE_ONE}"),
        TABLE_ONE.replace(r#"version="1.0""#, r#"version="1.1""#),
        TABLE_ONE.replace(declaration, r#"<?xml encoding="UTF-8"?>"#),
        TABLE_ONE.replace(declaration, r#"<?xml version="1.0" version="1.0"?>"#),
        TABLE_ONE.replace(declaration, r#"<?xml version="1.0" standalone="maybe"?>"#),
        TABLE_ONE.replace(
            declaration,
            r#"<?xml version="1.0" standalone="yes" encoding="UTF-8"?>"#,
        ),
        TABLE_ONE.replace(declaration, r#"<?xml version="1.0" encoding="UTF 8"?>"#),
        TABLE_ONE.replace(
            declaration,
            r#"<?xml version="1.0" extension="unsupported"?>"#,
        ),
        TABLE_ONE.replace("  <tableColumns", r#"  <?xml version="1.0"?><tableColumns"#),
        TABLE_ONE.replace("  <tableColumns", "  <!--bad--comment--><tableColumns"),
        format!("\u{a0}{body}"),
        format!("{body}\u{a0}"),
    ];
    for invalid_table in invalid_tables {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let error = read_xlsx(Cursor::new(archive), ReadOptions::default())
            .expect_err("malformed table XML must fail the read");
        assert_eq!(error.code(), XlsxErrorCode::InvalidXml, "{invalid_table}");
    }

    for valid_table in [
        format!(" \t\r\n{body}"),
        TABLE_ONE.replace(
            declaration,
            &format!("{declaration}\n<!--prolog--><?probe value?>"),
        ),
        TABLE_ONE.replace(
            declaration,
            &format!(r#"{declaration}<?xml-stylesheet href="table.xsl"?>"#),
        ),
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &valid_table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("valid XML prolog");
        assert!(snapshot.table("SalesDisplay").is_some());
    }
}

#[test]
fn table_child_sequences_and_filter_column_choice_are_enforced() {
    let invalid_tables = [
        TABLE_ONE.replace(
            "</tableColumns>",
            r#"</tableColumns><autoFilter ref="A1:C3"/>"#,
        ),
        TABLE_ONE.replace(
            "  <tableColumns",
            r#"  <autoFilter ref="A1:C3"><sortState ref="A2:C3"/><filterColumn colId="0"/></autoFilter>
  <tableColumns"#,
        ),
        TABLE_ONE.replace(
            r#"<tableColumn id="1" name="Region"/>"#,
            r#"<tableColumn id="1" name="Region"><totalsRowFormula>1</totalsRowFormula><calculatedColumnFormula>2</calculatedColumnFormula></tableColumn>"#,
        ),
        TABLE_ONE.replace(
            "  <tableColumns",
            r#"  <autoFilter ref="A1:C3"><filterColumn colId="0"><filters/><extLst/></filterColumn></autoFilter>
  <tableColumns"#,
        ),
    ];
    for invalid_table in invalid_tables {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("bounded read");
        assert!(snapshot.table("SalesDisplay").is_none(), "{invalid_table}");
        assert!(
            snapshot
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
        );
    }
}

#[test]
fn table_filter_and_sort_ranges_must_stay_inside_the_table() {
    let filter_column_outside_filter_range = TABLE_ONE.replace(
        "  <tableColumns",
        r#"  <autoFilter ref="B1:C3"><filterColumn colId="2"/></autoFilter>
  <tableColumns"#,
    );
    let duplicate_filter_column = TABLE_ONE.replace(
        "  <tableColumns",
        r#"  <autoFilter ref="A1:C3"><filterColumn colId="1"/><filterColumn colId="1"/></autoFilter>
  <tableColumns"#,
    );
    for invalid_table in [
        TABLE_WITH_METADATA.replacen(r#"autoFilter ref="A1:C3""#, r#"autoFilter ref="A1:D3""#, 1),
        TABLE_WITH_METADATA.replacen(
            r#"<sortState ref="A2:C3" columnSort="1" sortMethod="pinYin">"#,
            r#"<sortState ref="A2:D3" columnSort="1" sortMethod="pinYin">"#,
            1,
        ),
        TABLE_WITH_METADATA
            .replacen(
                r#"<sortState ref="A2:C3" columnSort="1" sortMethod="pinYin">"#,
                r#"<sortState ref="A2:B3" columnSort="1" sortMethod="pinYin">"#,
                1,
            )
            .replacen(
                r#"<sortCondition ref="A2:C2""#,
                r#"<sortCondition ref="C2:C3""#,
                1,
            ),
        filter_column_outside_filter_range,
        duplicate_filter_column,
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("bounded read");
        assert!(snapshot.table("SalesDisplay").is_none());
        assert_eq!(
            snapshot
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
                .count(),
            1
        );
    }
}

#[test]
fn table_sort_state_rejects_more_than_sixty_four_conditions() {
    let conditions = r#"<sortCondition ref="A2:A3"/>"#.repeat(65);
    let table = TABLE_ONE.replace(
        "  <tableColumns",
        &format!(
            r#"  <sortState ref="A2:C3">{conditions}</sortState>
  <tableColumns"#
        ),
    );
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &table),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("bounded read");
    assert!(snapshot.table("SalesDisplay").is_none());
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
            .count(),
        1
    );
}

#[test]
fn invalid_table_definitions_are_dropped_with_a_diagnostic() {
    let mismatched = TABLE_ONE.replace(
        r#"<tableColumn id="3" name="Note" totalsRowFunction="none"/>"#,
        "",
    );
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &mismatched),
        ],
    );
    let snapshot =
        read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read must succeed");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert!(first.tables().is_empty());
    assert!(snapshot.table("Sales").is_none());
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
            .count(),
        1
    );
}

#[test]
fn duplicate_table_display_names_drop_the_later_table() {
    let second_table = TABLE_ONE
        .replace(r#"id="1" name="Sales""#, r#"id="2" name="Other""#)
        .replace(
            r#"displayName="SalesDisplay""#,
            r#"displayName="SALESDISPLAY""#,
        );
    let snapshot = read_two_table_archive(&second_table);
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(first.tables()[0].display_name().as_str(), "SalesDisplay");
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code().as_str() == "xlsx.table.duplicate_display_name"
            })
            .count(),
        1
    );
}

#[test]
fn duplicate_table_ids_and_programmatic_names_drop_the_later_table() {
    let duplicate_id = TABLE_ONE
        .replace(r#"name="Sales""#, r#"name="Other""#)
        .replace(
            r#"displayName="SalesDisplay""#,
            r#"displayName="OtherDisplay""#,
        );
    let snapshot = read_two_table_archive(&duplicate_id);
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.duplicate_id")
            .count(),
        1
    );

    let duplicate_programmatic_name = TABLE_ONE
        .replace(r#"id="1" name="Sales""#, r#"id="2" name="SALES""#)
        .replace(
            r#"displayName="SalesDisplay""#,
            r#"displayName="OtherDisplay""#,
        );
    let snapshot = read_two_table_archive(&duplicate_programmatic_name);
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code().as_str() == "xlsx.table.duplicate_programmatic_name"
            })
            .count(),
        1
    );
}

#[test]
fn overlapping_table_ranges_drop_one_table_with_a_diagnostic() {
    let overlapping = TABLE_ONE
        .replace(r#"id="1" name="Sales""#, r#"id="2" name="Other""#)
        .replace(
            r#"displayName="SalesDisplay""#,
            r#"displayName="OtherDisplay""#,
        );
    let snapshot = read_two_table_archive(&overlapping);
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.overlap")
            .count(),
        1
    );
}

#[test]
fn overlapping_table_does_not_reserve_identity_from_a_later_valid_table() {
    let overlapping = TABLE_ONE
        .replace(r#"id="1" name="Sales""#, r#"id="2" name="Other""#)
        .replace(
            r#"displayName="SalesDisplay""#,
            r#"displayName="OtherDisplay""#,
        );
    let later_valid = overlapping.replace(r#"ref="A1:C4""#, r#"ref="E1:G4""#);
    let snapshot = read_three_table_archive(&overlapping, &later_valid);
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 2);
    assert_eq!(first.tables()[0].id().get(), 1);
    assert_eq!(first.tables()[1].id().get(), 2);
    assert_eq!(first.tables()[1].range().start().to_string(), "E1");
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.overlap")
            .count(),
        1
    );
    assert!(
        snapshot.diagnostics().iter().all(|diagnostic| {
            !matches!(
                diagnostic.code().as_str(),
                "xlsx.table.duplicate_id"
                    | "xlsx.table.duplicate_display_name"
                    | "xlsx.table.duplicate_programmatic_name"
            )
        }),
        "the discarded table must not reserve its identities"
    );
}

#[test]
fn table_display_names_conflicting_with_defined_names_are_dropped() {
    let workbook = WORKBOOK.replace(
        "  <calcPr",
        "  <definedNames><definedName name=\"SALESDISPLAY\">1</definedName></definedNames>\n  <calcPr",
    );
    let archive = build_table_archive_with_workbook(
        &workbook,
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_ONE),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read");
    assert!(
        snapshot
            .sheet_by_name("First")
            .expect("sheet")
            .tables()
            .is_empty()
    );
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code().as_str() == "xlsx.table.display_name_conflict"
            })
            .count(),
        1
    );
}

#[test]
fn required_table_and_column_ids_are_validated() {
    let invalid_tables = [
        TABLE_ONE.replace(r#" id="1""#, ""),
        TABLE_ONE.replace(r#"id="1""#, r#"id="0""#),
        TABLE_ONE.replace(r#"<tableColumn id="1""#, r#"<tableColumn id="0""#),
        TABLE_ONE.replace(r#"tableColumns count="3""#, r#"tableColumns count="bad""#),
    ];
    for invalid_table in invalid_tables {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read must succeed");
        assert!(
            snapshot
                .sheet_by_name("First")
                .expect("sheet")
                .tables()
                .is_empty()
        );
        assert_eq!(
            snapshot
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
                .count(),
            1
        );
    }
}

#[test]
fn optional_and_mismatched_table_column_counts_use_the_declared_children() {
    for (table_xml, expected_normalization_count) in [
        (
            TABLE_ONE.replace(r#"tableColumns count="3""#, "tableColumns"),
            0,
        ),
        (
            TABLE_ONE.replace(r#"tableColumns count="3""#, r#"tableColumns count="2""#),
            1,
        ),
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &table_xml),
            ],
        );
        let snapshot =
            read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read must succeed");
        let table = snapshot.table("SalesDisplay").expect("valid table");
        assert_eq!(table.columns().len(), 3);
        assert_eq!(
            snapshot
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.normalized")
                .count(),
            expected_normalization_count
        );
    }
}

#[test]
fn unresolved_table_relationship_is_dropped_with_a_diagnostic() {
    let archive = build_table_archive(SHEET_WITH_TABLE, &[]);
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read");
    let first = snapshot.sheet_by_name("First").expect("sheet");
    assert!(first.tables().is_empty());
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
            .count(),
        1
    );
}

#[test]
fn table_read_limits_fail_the_read_with_dedicated_codes() {
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_ONE),
        ],
    );
    let columns_limit = ReadLimits::default()
        .with_max_table_columns(2)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive.clone()),
        ReadOptions::new(columns_limit),
    )
    .expect_err("three columns must exceed a limit of two");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTableColumns);

    let name_limit = ReadLimits::default()
        .with_max_table_name_bytes(4)
        .expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(name_limit))
        .expect_err("'Sales' must exceed a four-byte name limit");
    assert_eq!(error.code(), XlsxErrorCode::TableNameTooLarge);

    let sheet = SHEET_WITH_TABLE.replace(
        r#"<tableParts count="1"><tablePart r:id="rId7"/></tableParts>"#,
        r#"<tableParts count="2"><tablePart r:id="rId7"/><tablePart r:id="rId7"/></tableParts>"#,
    );
    let archive = build_table_archive(
        &sheet,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_ONE),
        ],
    );
    let tables_limit = ReadLimits::default().with_max_tables(1).expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(tables_limit))
        .expect_err("two referenced parts must exceed a limit of one");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTables);

    let missing_relationship_ids = SHEET_WITH_TABLE.replace(
        r#"<tableParts count="1"><tablePart r:id="rId7"/></tableParts>"#,
        r#"<tableParts count="2"><tablePart/><tablePart/></tableParts>"#,
    );
    let archive = build_table_archive(&missing_relationship_ids, &[]);
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(tables_limit))
        .expect_err("invalid declarations must still consume the table budget");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTables);

    let duplicate_columns = TABLE_ONE.replace(
        "</tableColumns>",
        r#"</tableColumns><tableColumns count="3"><tableColumn id="8" name="A"/><tableColumn id="9" name="B"/><tableColumn id="10" name="C"/></tableColumns>"#,
    );
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &duplicate_columns),
        ],
    );
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(columns_limit))
        .expect_err("columns in duplicate containers must still consume the column budget");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTableColumns);

    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_WITH_METADATA),
        ],
    );
    let formula_limit = ReadLimits::default()
        .with_max_formula_bytes(5)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive.clone()),
        ReadOptions::new(formula_limit),
    )
    .expect_err("table formulas must use the per-formula budget");
    assert_eq!(error.code(), XlsxErrorCode::FormulaTooLarge);

    let total_formula_limit = ReadLimits::default()
        .with_max_total_formula_bytes(20)
        .expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(total_formula_limit))
        .expect_err("table formulas must use the workbook formula budget");
    assert_eq!(error.code(), XlsxErrorCode::TotalFormulaBytesTooLarge);

    for invalid_table in [
        TABLE_WITH_METADATA.replacen(r#"id="1""#, r#"id="0""#, 1),
        TABLE_WITH_METADATA.replacen(r#"<tableColumn id="5""#, r#"<tableColumn id="0""#, 1),
    ] {
        let archive = build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", &invalid_table),
            ],
        );
        let error = read_xlsx(
            Cursor::new(archive.clone()),
            ReadOptions::new(formula_limit),
        )
        .expect_err("invalid tables must still consume the per-formula budget");
        assert_eq!(error.code(), XlsxErrorCode::FormulaTooLarge);

        let error = read_xlsx(Cursor::new(archive), ReadOptions::new(total_formula_limit))
            .expect_err("invalid tables must still consume the total formula budget");
        assert_eq!(error.code(), XlsxErrorCode::TotalFormulaBytesTooLarge);
    }
}

#[test]
fn table_filter_resource_limits_are_exact_and_apply_to_invalid_tables() {
    let one_filter = TABLE_ONE.replace(
        "  <tableColumns",
        r#"  <autoFilter ref="A1:C3"><filterColumn colId="0"><filters><filter val="é"/></filters></filterColumn></autoFilter>
  <tableColumns"#,
    );
    let two_filters = one_filter.replace(
        r#"<filter val="é"/>"#,
        r#"<filter val="é"/><filter val="B"/>"#,
    );
    let archive_for = |table: &str| {
        build_table_archive(
            SHEET_WITH_TABLE,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
                ("xl/tables/table1.xml", table),
            ],
        )
    };

    let item_limit = ReadLimits::default()
        .with_max_table_filter_items(2)
        .expect("limit");
    let snapshot = read_xlsx(
        Cursor::new(archive_for(&two_filters)),
        ReadOptions::new(item_limit),
    )
    .expect("two filter items are exactly at the limit");
    assert!(snapshot.table("SalesDisplay").is_some());
    let item_limit = ReadLimits::default()
        .with_max_table_filter_items(1)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive_for(&two_filters)),
        ReadOptions::new(item_limit),
    )
    .expect_err("two filter items must exceed a limit of one");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTableFilterItems);

    let text_limit = ReadLimits::default()
        .with_max_table_filter_text_bytes(8)
        .expect("limit");
    let snapshot = read_xlsx(
        Cursor::new(archive_for(&one_filter)),
        ReadOptions::new(text_limit),
    )
    .expect("five ref bytes, one colId byte, and two UTF-8 value bytes are at the limit");
    assert!(snapshot.table("SalesDisplay").is_some());
    let text_limit = ReadLimits::default()
        .with_max_table_filter_text_bytes(7)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive_for(&one_filter)),
        ReadOptions::new(text_limit),
    )
    .expect_err("filter text must use decoded UTF-8 byte length");
    assert_eq!(error.code(), XlsxErrorCode::TableFilterTextTooLarge);

    let invalid_table = two_filters.replacen(r#"id="1""#, r#"id="0""#, 1);
    let error = read_xlsx(
        Cursor::new(archive_for(&invalid_table)),
        ReadOptions::new(item_limit),
    )
    .expect_err("invalid tables must still consume the filter-item budget");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTableFilterItems);
    let invalid_table = one_filter.replacen(r#"id="1""#, r#"id="0""#, 1);
    let error = read_xlsx(
        Cursor::new(archive_for(&invalid_table)),
        ReadOptions::new(text_limit),
    )
    .expect_err("invalid tables must still consume the filter-text budget");
    assert_eq!(error.code(), XlsxErrorCode::TableFilterTextTooLarge);

    let invalid_filter_range = two_filters.replace(r#"ref="A1:C3""#, r#"ref="invalid""#);
    let error = read_xlsx(
        Cursor::new(archive_for(&invalid_filter_range)),
        ReadOptions::new(item_limit),
    )
    .expect_err("invalid autoFilter ranges must not bypass descendant item accounting");
    assert_eq!(error.code(), XlsxErrorCode::TooManyTableFilterItems);

    let invalid_sort_range = TABLE_ONE.replace(
        "  <tableColumns",
        r#"  <autoFilter ref="A1:C3"><sortState ref="bad"><sortCondition ref="A2:A3" customList="01234567890123456789012345678901"/></sortState></autoFilter>
  <tableColumns"#,
    );
    let descendant_text_limit = ReadLimits::default()
        .with_max_table_filter_text_bytes(20)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive_for(&invalid_sort_range)),
        ReadOptions::new(descendant_text_limit),
    )
    .expect_err("invalid sort ranges must not bypass descendant text accounting");
    assert_eq!(error.code(), XlsxErrorCode::TableFilterTextTooLarge);
}

#[test]
fn nested_table_formula_text_consumes_formula_budgets() {
    let nested_formula = TABLE_WITH_METADATA.replace("[@Amount]*2", "A<future>123456</future>B");
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &nested_formula),
        ],
    );
    let formula_limit = ReadLimits::default()
        .with_max_formula_bytes(5)
        .expect("limit");
    let error = read_xlsx(
        Cursor::new(archive.clone()),
        ReadOptions::new(formula_limit),
    )
    .expect_err("nested markup text must consume the per-formula budget");
    assert_eq!(error.code(), XlsxErrorCode::FormulaTooLarge);

    let total_limit = ReadLimits::default()
        .with_max_total_formula_bytes(5)
        .expect("limit");
    let error = read_xlsx(Cursor::new(archive), ReadOptions::new(total_limit))
        .expect_err("nested markup text must consume the total formula budget");
    assert_eq!(error.code(), XlsxErrorCode::TotalFormulaBytesTooLarge);
}

#[test]
fn tables_survive_edit_write_reopen() {
    let source = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_WITH_METADATA),
        ],
    );
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let source_table = document
        .workbook()
        .table("SalesDisplay")
        .expect("source table")
        .clone();
    let sheet_id = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    let semantic_revision = draft.semantic_revision();
    draft
        .set_cell_value(
            sheet_id,
            address("A1"),
            CellValue::Number(crate::FiniteNumber::new(9.0).expect("finite")),
        )
        .expect("edit");
    assert_eq!(draft.semantic_revision(), semantic_revision + 1);
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    let table = reopened
        .workbook()
        .table("SalesDisplay")
        .expect("table survives");
    assert_eq!(table, &source_table);
    assert_eq!(table.table_type(), crate::TableType::QueryTable);
    assert!(
        table.columns()[1]
            .calculated_column_formula()
            .expect("calculated formula")
            .is_array()
    );
    let first = reopened.workbook().sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(
        literal(first, "A1"),
        &CellValue::Number(crate::FiniteNumber::new(9.0).expect("finite")),
        "the edit itself must survive the rewrite"
    );
    let worksheet = archive_text(output.bytes(), "xl/worksheets/sheet1.xml");
    assert!(worksheet.contains("<tableParts"), "{worksheet}");
    let table_part = archive_text(output.bytes(), "xl/tables/table1.xml");
    assert!(
        table_part.contains("displayName=\"SalesDisplay\""),
        "{table_part}"
    );
    let draft_table = draft
        .workbook()
        .table("SalesDisplay")
        .expect("draft keeps table");
    assert_eq!(draft_table.columns().len(), 3);
}

#[test]
fn opaque_table_part_survives_a_source_linked_cell_edit_byte_identically() {
    let opaque_table =
        TABLE_WITH_METADATA.replace(r#"ref="A1:C4""#, r#"ref="A1:C4" published="1""#);
    let source = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &opaque_table),
        ],
    );
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    assert!(
        document
            .workbook()
            .table("SalesDisplay")
            .expect("opaque table")
            .has_opaque_metadata()
    );

    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            SheetId::new(1).expect("sheet"),
            address("A2"),
            CellValue::Number(crate::FiniteNumber::new(7.0).expect("finite")),
        )
        .expect("cell edit");
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("source-linked write");

    assert_eq!(
        archive_text(&source, "xl/tables/table1.xml"),
        archive_text(output.bytes(), "xl/tables/table1.xml"),
        "an unrelated cell edit must not rewrite opaque table XML"
    );
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert!(
        reopened
            .workbook()
            .table("SalesDisplay")
            .expect("reopened table")
            .has_opaque_metadata()
    );
}

#[test]
fn preserved_write_keeps_table_and_merge_parts_byte_identical() {
    let sheet = SHEET_WITH_TABLE.replace(
        "<tableParts",
        r#"<mergeCells count="1"><mergeCell ref="A2:B3"/></mergeCells><tableParts"#,
    );
    let source = build_table_archive(
        &sheet,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", TABLE_ONE),
        ],
    );
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let output = crate::write_preserved_xlsx_bytes(&document, crate::WriteOptions::default())
        .expect("preserved write");
    for part in [
        "xl/tables/table1.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
    ] {
        assert_eq!(
            archive_text(&source, part),
            archive_text(&output, part),
            "{part} must round-trip byte-identically"
        );
    }
    let reopened =
        open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopened output");
    let first = reopened.workbook().sheet_by_name("First").expect("sheet");
    assert_eq!(first.tables().len(), 1);
    assert_eq!(first.merged_ranges().len(), 1);
}

#[test]
fn table_name_defaults_to_display_name_and_unknown_totals_functions_drop_the_table() {
    let unnamed = TABLE_ONE.replace(r#" name="Sales""#, "");
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &unnamed),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read");
    assert_eq!(
        snapshot
            .table("SalesDisplay")
            .expect("name defaults to displayName")
            .name()
            .as_str(),
        "SalesDisplay"
    );

    let unknown_totals =
        TABLE_ONE.replace(r#"totalsRowFunction="sum""#, r#"totalsRowFunction="bogus""#);
    let archive = build_table_archive(
        SHEET_WITH_TABLE,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_ONE_TABLE_RELS),
            ("xl/tables/table1.xml", &unknown_totals),
        ],
    );
    let snapshot = read_xlsx(Cursor::new(archive), ReadOptions::default()).expect("read");
    assert!(snapshot.table("Sales").is_none());
    assert_eq!(
        snapshot
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "xlsx.table.invalid")
            .count(),
        1
    );
}

#[test]
fn merged_ranges_survive_edit_write_reopen() {
    let sheet = SHEET_ONE.replace(
        "</worksheet>",
        r#"<mergeCells count="2"><mergeCell ref="A2:B3"/><mergeCell ref="D5:D7"/></mergeCells></worksheet>"#,
    );
    let source = build_archive(&sheet, SHARED_STRINGS);
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let sheet_id = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            address("G1"),
            CellValue::Number(crate::FiniteNumber::new(7.0).expect("finite")),
        )
        .expect("edit");
    let calculation = calculate_workbook(draft.workbook(), crate::CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    let first = reopened
        .workbook()
        .sheet_by_name("First")
        .expect("reopened sheet");
    let ranges: Vec<String> = first
        .merged_ranges()
        .iter()
        .map(|range| format!("{}:{}", range.start(), range.end()))
        .collect();
    assert_eq!(ranges, vec!["A2:B3", "D5:D7"]);
    let worksheet = archive_text(output.bytes(), "xl/worksheets/sheet1.xml");
    assert!(worksheet.contains("<mergeCells"), "{worksheet}");
}

fn build_archive(sheet_one: &str, shared_strings: &str) -> Vec<u8> {
    build_archive_with_sheets(sheet_one, SHEET_TWO, shared_strings)
}

fn build_archive_with_sheets(sheet_one: &str, sheet_two: &str, shared_strings: &str) -> Vec<u8> {
    let entries = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/styles.xml", STYLES),
        ("xl/sharedStrings.xml", shared_strings),
        ("xl/worksheets/sheet1.xml", sheet_one),
        ("xl/worksheets/sheet2.xml", sheet_two),
    ];
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, contents) in entries {
            writer
                .start_file(name, options)
                .expect("start fixture part");
            writer
                .write_all(contents.as_bytes())
                .expect("write fixture part");
        }
        writer.finish().expect("finish fixture archive");
    }
    output.into_inner()
}

fn archive_text(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("archive");
    let mut part = archive.by_name(name).expect("part");
    let mut text = String::new();
    part.read_to_string(&mut text).expect("UTF-8 part");
    text
}

fn address(value: &str) -> CellAddress {
    let split = value
        .bytes()
        .position(|byte| byte.is_ascii_digit())
        .expect("test address row");
    let (column, row) = value.split_at(split);
    let column = column
        .bytes()
        .fold(0_u32, |value, byte| value * 26 + u32::from(byte - b'A' + 1));
    CellAddress::from_indices(row.parse().expect("test row"), column).expect("test address")
}

fn cell<'a>(sheet: &'a crate::Sheet, address_value: &str) -> &'a crate::Cell {
    sheet.cell(address(address_value)).expect("fixture cell")
}

fn literal<'a>(sheet: &'a crate::Sheet, address_value: &str) -> &'a CellValue {
    let CellContent::Literal(value) = cell(sheet, address_value).content() else {
        panic!("fixture literal");
    };
    value
}

fn number(value: &CellValue) -> f64 {
    let CellValue::Number(value) = value else {
        panic!("fixture number");
    };
    value.get()
}
