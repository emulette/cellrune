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
  <calcPr calcId="7" calcMode="manual" fullCalcOnLoad="1" forceFullCalc="0"/>
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

fn build_archive(sheet_one: &str, shared_strings: &str) -> Vec<u8> {
    let entries = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/styles.xml", STYLES),
        ("xl/sharedStrings.xml", shared_strings),
        ("xl/worksheets/sheet1.xml", sheet_one),
        ("xl/worksheets/sheet2.xml", SHEET_TWO),
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
