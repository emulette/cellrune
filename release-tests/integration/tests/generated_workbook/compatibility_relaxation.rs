use cellrune::{
    CalculationCellResult, CalculationOptions, CellAddress, CellContent, CellValue, ExcelError,
    OpenOptions, ReadOptions, SheetId, calculate_workbook, open_xlsx_document_bytes,
    read_xlsx_bytes,
};

use crate::support::generated_xlsx::generated_package;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const SHEET_CONTENT: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";

/// A package whose `Sheet1` is `worksheet`, with optional styles, a second sheet `Sheet2`, and a
/// chartsheet `Chart1` after them, with `defined_names` in the workbook.
fn package(worksheet: &str, styles: Option<&str>, defined_names: &str) -> Vec<u8> {
    let mut content_types = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="{SHEET_CONTENT}"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="{SHEET_CONTENT}"/>
  <Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/>"#
    );
    let mut relationships = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>"#,
    );
    if styles.is_some() {
        content_types.push_str(r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#);
        relationships.push_str(r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    }
    content_types.push_str("</Types>");
    relationships.push_str("</Relationships>");
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1"/>
    <sheet name="Sheet2" sheetId="2" r:id="rId2"/>
    <sheet name="Chart1" sheetId="3" r:id="rId3"/>
  </sheets>
  <definedNames>{defined_names}</definedNames>
</workbook>"#
    );
    let sheet2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>10</v></c></row></sheetData></worksheet>"#;
    let chartsheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    let mut entries = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", worksheet),
        ("xl/worksheets/sheet2.xml", sheet2),
        ("xl/chartsheets/sheet1.xml", chartsheet),
    ];
    if let Some(styles) = styles {
        entries.push(("xl/styles.xml", styles));
    }
    generated_package(&entries)
}

fn worksheet(sheet_data: &str, extra: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{extra}<sheetData>{sheet_data}</sheetData></worksheet>"#
    )
}

fn content(workbook: &cellrune::WorkbookSnapshot, address: &str) -> Option<CellContent> {
    workbook.sheets()[0]
        .cell(CellAddress::from_a1(address).expect("address"))
        .map(|cell| cell.content().clone())
}

fn cell(workbook: &cellrune::WorkbookSnapshot, address: &str) -> Option<CellContent> {
    content(workbook, address)
}

fn literal(value: CellValue) -> Option<CellContent> {
    Some(CellContent::Literal(value))
}

#[test]
fn an_xf_without_the_optional_number_format_id_reads_as_general() {
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="2"><xf/><xf fontId="0"/></cellXfs></styleSheet>"#;
    let bytes = package(
        &worksheet(r#"<row r="1"><c r="A1" s="1"><v>1</v></c></row>"#, ""),
        Some(styles),
        "",
    );
    let document = open_xlsx_document_bytes(&bytes, OpenOptions::default())
        .expect("an absent numFmtId no longer fails the open");
    let number_format = document.workbook().sheets()[0]
        .cell(CellAddress::from_a1("A1").expect("address"))
        .expect("A1")
        .number_format()
        .clone();
    assert_eq!(number_format.id(), 0);
}

#[test]
fn cells_and_rows_without_the_optional_reference_follow_the_previous_one() {
    let bytes = package(
        &worksheet(
            r#"<row r="2"><c><v>1</v></c><c><v>2</v></c><c r="E2"><v>3</v></c><c><v>4</v></c></row><row><c><v>5</v></c></row><row r="7"><c r="B7"><v>6</v></c></row><row><c><v>7</v></c></row>"#,
            "",
        ),
        None,
        "",
    );
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())
        .expect("absent cell and row references no longer fail the read");
    for (address, value) in [
        ("A2", 1.0),
        ("B2", 2.0),
        ("E2", 3.0),
        ("F2", 4.0),
        ("A3", 5.0),
        ("B7", 6.0),
        ("A8", 7.0),
    ] {
        assert_eq!(
            cell(&workbook, address),
            literal(CellValue::number(value).expect("number")),
            "{address}"
        );
    }
}

#[test]
fn newer_error_values_read_and_report_unsupported_when_calculated() {
    let bytes = package(
        &worksheet(
            r##"<row r="1"><c r="A1" t="e"><v>#FIELD!</v></c><c r="B1" t="e"><v>#BLOCKED!</v></c><c r="C1" t="e"><v>#CONNECT!</v></c><c r="D1" t="e"><v>#BUSY!</v></c><c r="E1" t="e"><v>#UNKNOWN!</v></c><c r="F1" t="e"><v>#PYTHON!</v></c><c r="G1"><f>A1</f></c></row>"##,
            "",
        ),
        None,
        "",
    );
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())
        .expect("newer error values no longer fail the read");
    for (address, error) in [
        ("A1", ExcelError::Field),
        ("B1", ExcelError::Blocked),
        ("C1", ExcelError::Connect),
        ("D1", ExcelError::Busy),
        ("E1", ExcelError::Unknown),
        ("F1", ExcelError::Python),
    ] {
        assert_eq!(
            cell(&workbook, address),
            literal(CellValue::Error(error)),
            "{address}"
        );
    }
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let result = calculation
        .cell(cellrune::CalculationCellId::new(
            SheetId::new(1).expect("sheet"),
            CellAddress::from_a1("G1").expect("address"),
        ))
        .expect("G1");
    assert!(
        matches!(result, CalculationCellResult::Unavailable(_)),
        "a formula reading an unmodeled error reports it instead of guessing: {result:?}"
    );
}

#[test]
fn shared_formula_references_wrap_past_the_grid_edge() {
    let bytes = package(
        &worksheet(
            r#"<row r="1"><c r="A1"><f t="shared" ref="A1:B1" si="0">XFD1+1</f><v>1</v></c><c r="B1"><f t="shared" si="0"/><v>2</v></c></row>"#,
            "",
        ),
        None,
        "",
    );
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())
        .expect("a shared formula shifted past the grid edge no longer fails the read");
    let Some(CellContent::Formula(formula)) = cell(&workbook, "B1") else {
        panic!("B1 is a formula");
    };
    assert_eq!(formula.text().map(|text| text.as_str()), Some("A1+1"));
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let b1 = calculation
        .cell(cellrune::CalculationCellId::new(
            SheetId::new(1).expect("sheet"),
            CellAddress::from_a1("B1").expect("address"),
        ))
        .expect("B1");
    assert_eq!(
        b1,
        &CalculationCellResult::Value(CellValue::number(2.0).expect("number"))
    );
}

#[test]
fn malformed_panes_open_unmodeled_with_a_diagnostic() {
    for view in [
        r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomRight" state="frozen"/></sheetView></sheetViews>"#,
        r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1.5" state="frozen"/></sheetView></sheetViews>"#,
        r#"<sheetViews><sheetView/></sheetViews>"#,
    ] {
        let bytes = package(
            &worksheet(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, view),
            None,
            "",
        );
        let document = open_xlsx_document_bytes(&bytes, OpenOptions::default())
            .expect("a malformed pane no longer fails a document open");
        let sheet = SheetId::new(1).expect("sheet");
        assert_eq!(document.presentation().frozen_pane(sheet), None, "{view}");
        assert!(
            document
                .presentation()
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "xlsx.pane.invalid"),
            "{view}"
        );
        let saved =
            cellrune::write_preserved_xlsx_bytes(&document, cellrune::WriteOptions::default())
                .expect("preserved copy");
        open_xlsx_document_bytes(&saved, OpenOptions::default()).expect("reopen");
    }
}
