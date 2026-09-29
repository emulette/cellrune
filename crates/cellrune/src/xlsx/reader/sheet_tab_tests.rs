use std::io::{Cursor, Read, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{CompressionMethod, ZipArchive};

use crate::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellValue,
    DefinedName, DefinedNameScope, FormulaText, OpenOptions, RecalculationWriteOptions, SheetId,
    SheetName, ValidationError, WorkbookDraft, XlsxDocument, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

// Tab order: Data, Chart, Summary. The chartsheet's sheetId is larger than every worksheet's.
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Chart" sheetId="7" r:id="rId3"/>
    <sheet name="Summary" sheetId="2" r:id="rId2"/>
  </sheets>
  <definedNames>
    <definedName name="ChartLocal" localSheetId="1">1</definedName>
    <definedName name="Local" localSheetId="2">7</definedName>
  </definedNames>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
</Relationships>"#;

const DATA_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1"><f>SHEET()</f></c><c r="B1"><v>10</v></c></row>
  </sheetData>
</worksheet>"#;

const SUMMARY_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><f>SHEET()</f></c>
      <c r="B1"><v>5</v></c>
      <c r="C1"><f>SHEETS()</f></c>
      <c r="D1"><f>SHEET("Chart")</f></c>
      <c r="E1"><f>SHEETS(Data:Summary!A1)</f></c>
      <c r="F1"><f>SUM(Data:Summary!B1)</f></c>
      <c r="G1"><f>Local</f></c>
      <c r="H1"><f>SHEET(Data!A1)</f></c>
    </row>
  </sheetData>
</worksheet>"#;

const CHARTSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"/></sheetViews></chartsheet>"#;

#[test]
fn non_worksheet_tabs_keep_their_tab_position_without_joining_calculation() {
    for relationship_type in [
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/dialogsheet",
        "http://schemas.microsoft.com/office/2006/relationships/xlMacrosheet",
    ] {
        let relationships = WORKBOOK_RELATIONSHIPS.replace(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
            relationship_type,
        );
        let document = open_xlsx_document_bytes(
            &build_archive_with_relationships(&relationships),
            OpenOptions::default(),
        )
        .expect(relationship_type);
        assert_non_worksheet_tab_is_skipped(&document);
    }
}

fn assert_non_worksheet_tab_is_skipped(document: &XlsxDocument) {
    let workbook = document.workbook();
    let names = workbook
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Data", "Summary"]);
    assert!(
        workbook
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "xlsx.sheet.non_worksheet")
    );
    assert_eq!(
        workbook.defined_names()[0].scope(),
        DefinedNameScope::Sheet(sheet_id(7))
    );
    assert_eq!(
        workbook.defined_names()[1].scope(),
        DefinedNameScope::Sheet(sheet_id(2))
    );
    assert_tab_results(document, 3.0);
}

#[test]
fn chartsheets_survive_a_source_linked_write_in_their_tab_position() {
    let source = build_archive();
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let mut draft = WorkbookDraft::from_document(&document);
    let added = draft
        .add_sheet(SheetName::new("Added").expect("sheet name"))
        .expect("add sheet");
    assert_eq!(added, sheet_id(8), "sheet IDs stay unique across every tab");
    assert!(matches!(
        draft.add_sheet(SheetName::new("chart").expect("sheet name")),
        Err(ValidationError::DuplicateSheetName { .. })
    ));
    draft
        .set_defined_name(
            DefinedName::new(
                "Added",
                DefinedNameScope::Sheet(added),
                FormulaText::from_xlsx("1").expect("formula"),
                false,
            )
            .expect("defined name"),
        )
        .expect("regenerate defined names");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write workbook with a chartsheet");

    let workbook_xml = part_text(output.bytes(), "xl/workbook.xml");
    let data = workbook_xml.find(r#"name="Data""#).expect("Data tab");
    let chart = workbook_xml.find(r#"name="Chart""#).expect("Chart tab");
    let summary = workbook_xml.find(r#"name="Summary""#).expect("Summary tab");
    let appended = workbook_xml.find(r#"name="Added""#).expect("Added tab");
    assert!(
        data < chart && chart < summary && summary < appended,
        "{workbook_xml}"
    );
    for local in [
        r#"name="ChartLocal" localSheetId="1""#,
        r#"name="Local" localSheetId="2""#,
        r#"name="Added" localSheetId="3""#,
    ] {
        assert!(workbook_xml.contains(local), "{local}: {workbook_xml}");
    }
    assert_eq!(
        part_text(output.bytes(), "xl/chartsheets/sheet1.xml"),
        CHARTSHEET
    );

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert_tab_results(&reopened, 4.0);
}

fn assert_tab_results(document: &XlsxDocument, tab_count: f64) {
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let value = |sheet: u32, address: &str| {
        let cell = CalculationCellId::new(
            sheet_id(sheet),
            CellAddress::from_a1(address).expect("address"),
        );
        match calculation.cell(cell) {
            Some(CalculationCellResult::Value(CellValue::Number(number))) => number.get(),
            other => panic!("{address}: {other:?}"),
        }
    };
    assert_eq!(value(1, "A1"), 1.0, "SHEET() on the first tab");
    assert_eq!(value(2, "A1"), 3.0, "SHEET() after the chartsheet");
    assert_eq!(value(2, "D1"), 2.0, "SHEET of the chartsheet name");
    assert_eq!(
        value(2, "E1"),
        3.0,
        "SHEETS of a 3-D span across the chartsheet"
    );
    assert_eq!(
        value(2, "F1"),
        15.0,
        "3-D SUM skips the cell-less chartsheet"
    );
    assert_eq!(value(2, "G1"), 7.0, "localSheetId counts the chartsheet");
    assert_eq!(value(2, "H1"), 1.0, "SHEET of a reference");
    assert_eq!(value(2, "C1"), tab_count, "SHEETS() counts every tab");
}

fn sheet_id(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet id")
}

fn build_archive() -> Vec<u8> {
    build_archive_with_relationships(WORKBOOK_RELATIONSHIPS)
}

fn build_archive_with_relationships(relationships: &str) -> Vec<u8> {
    let entries = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", DATA_SHEET),
        ("xl/worksheets/sheet2.xml", SUMMARY_SHEET),
        ("xl/chartsheets/sheet1.xml", CHARTSHEET),
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

fn part_text(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("archive");
    let mut part = archive.by_name(name).expect("part");
    let mut text = String::new();
    part.read_to_string(&mut text).expect("UTF-8 part");
    text
}
