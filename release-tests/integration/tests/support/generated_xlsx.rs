use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

static TEMPORARY_WORKBOOK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <workbookPr date1904="0"/>
  <sheets>
    <sheet name="Inputs" sheetId="1" r:id="rId1"/>
    <sheet name="Calculations" sheetId="2" r:id="rId2"/>
  </sheets>
  <definedNames>
    <definedName name="InputAmount">Inputs!$B$2</definedName>
  </definedNames>
  <calcPr calcId="191029" calcMode="auto" fullCalcOnLoad="0" forceFullCalc="0"/>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts>
  <cellXfs count="2">
    <xf numFmtId="0"/>
    <xf numFmtId="164" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"#;

const XLSB_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.bin" ContentType="application/vnd.ms-excel.sheet.binary.macroEnabled.main"/>
</Types>"#;

const XLSB_ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.bin"/>
</Relationships>"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProducerProfile {
    Excel,
    OpenPyxl,
    LibreOffice,
    GoogleSheets,
}

impl ProducerProfile {
    const fn retains_saved_results(self) -> bool {
        !matches!(self, Self::OpenPyxl)
    }
}

pub fn generated_workbook(profile: ProducerProfile) -> Vec<u8> {
    generated_workbook_with_archive_comment(profile, None)
}

pub fn generated_formula_fixture(formulas: &[&str]) -> Vec<u8> {
    let rows = formulas
        .iter()
        .enumerate()
        .map(|(index, formula)| {
            let row = index + 2;
            let escaped = formula
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            format!(r#"<row r="{row}"><c r="B{row}"><f>{escaped}</f></c></row>"#)
        })
        .collect::<String>();
    let calculations = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>{rows}</sheetData>
</worksheet>"#
    );
    build_archive(
        &[
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
            ("xl/styles.xml", STYLES),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
            ),
            ("xl/worksheets/sheet2.xml", &calculations),
        ],
        None,
    )
}

pub fn generated_table_reference_fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let worksheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>Region</t></is></c>
      <c r="B1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="C1" t="inlineStr"><is><t>Echo</t></is></c>
      <c r="H1"><f>SUM(Sales[Amount])</f></c>
    </row>
    <row r="2">
      <c r="A2" t="inlineStr"><is><t>North</t></is></c>
      <c r="B2"><v>10</v></c>
      <c r="C2"><f>[@Amount]</f></c>
      <c r="H2"><f>AREAS((A1:A2,C1:C2,A1:A2))</f></c>
    </row>
    <row r="3">
      <c r="A3" t="inlineStr"><is><t>South</t></is></c>
      <c r="B3"><v>20</v></c>
      <c r="C3"><f>[@Amount]</f></c>
      <c r="H3"><f>AREAS(A1:C3 A2:B4)</f></c>
    </row>
    <row r="4">
      <c r="A4" t="inlineStr"><is><t>West</t></is></c>
      <c r="B4"><v>30</v></c>
      <c r="C4"><f>[@Amount]</f></c>
      <c r="H4"><f>SUM(Sales[[#Data],[#Totals],[Amount]])</f></c>
    </row>
    <row r="5">
      <c r="A5" t="inlineStr"><is><t>Total</t></is></c>
      <c r="B5"><v>60</v></c>
      <c r="H5"><f>AREAS(Sales[#Totals])</f></c>
    </row>
  </sheetData>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#;
    let worksheet_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    let table = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
       id="1" name="Sales" displayName="Sales" ref="A1:C5"
       headerRowCount="1" totalsRowCount="1" totalsRowShown="1">
  <autoFilter ref="A1:C4"/>
  <tableColumns count="3">
    <tableColumn id="1" name="Region"/>
    <tableColumn id="2" name="Amount"/>
    <tableColumn id="3" name="Echo"/>
  </tableColumns>
</table>"#;
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/worksheets/sheet1.xml", worksheet),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                worksheet_relationships,
            ),
            ("xl/tables/table1.xml", table),
        ],
        None,
    )
}

pub fn generated_table_topology_fixture(data_rows: u32) -> Vec<u8> {
    assert!(data_rows > 0, "table topology fixture needs data rows");
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let mut rows = String::from(
        r#"<row r="1">
      <c r="A1" t="inlineStr"><is><t>Label</t></is></c>
      <c r="B1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="C1" t="inlineStr"><is><t>Echo</t></is></c>
      <c r="E1"><f>SUM(Sales[Amount])</f></c>
      <c r="F1"><f>ROWS(Sales[Amount])</f></c>
    </row>"#,
    );
    for index in 1..=data_rows {
        let row = index + 1;
        let amount = index * 10;
        rows.push_str(&format!(
            r#"<row r="{row}">
      <c r="A{row}" t="inlineStr"><is><t>Row {index}</t></is></c>
      <c r="B{row}"><v>{amount}</v></c>
      <c r="C{row}"><f>Sales[@Amount]</f></c>
    </row>"#,
        ));
    }
    let end_row = data_rows + 1;
    let worksheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>{rows}</sheetData>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#,
    );
    let worksheet_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
       id="1" name="Sales" displayName="Sales" ref="A1:C{end_row}" headerRowCount="1">
  <autoFilter ref="A1:C{end_row}"/>
  <tableColumns count="3">
    <tableColumn id="1" name="Label"/>
    <tableColumn id="2" name="Amount"/>
    <tableColumn id="3" name="Echo"/>
  </tableColumns>
</table>"#,
    );
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/worksheets/sheet1.xml", &worksheet),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                worksheet_relationships,
            ),
            ("xl/tables/table1.xml", &table),
        ],
        None,
    )
}

/// The Prika investigation's totals table: `tblChar` on `A1:D5` with data rows 2..4, a
/// `hp2` calculated column, and a totals row labelled `Total` that sums `hp` and `hp2`.
pub fn generated_totals_table_fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Chars" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let mut rows = String::from(
        r#"<row r="1">
      <c r="A1" t="inlineStr"><is><t>id</t></is></c>
      <c r="B1" t="inlineStr"><is><t>name</t></is></c>
      <c r="C1" t="inlineStr"><is><t>hp</t></is></c>
      <c r="D1" t="inlineStr"><is><t>hp2</t></is></c>
    </row>"#,
    );
    for (index, name) in ["a", "b", "c"].into_iter().enumerate() {
        let row = index + 2;
        let id = index + 1;
        let hp = id * 10;
        rows.push_str(&format!(
            r#"<row r="{row}">
      <c r="A{row}"><v>{id}</v></c>
      <c r="B{row}" t="inlineStr"><is><t>{name}</t></is></c>
      <c r="C{row}"><v>{hp}</v></c>
      <c r="D{row}"><f>tblChar[[#This Row],[hp]]*2</f></c>
    </row>"#,
        ));
    }
    rows.push_str(
        r#"<row r="5">
      <c r="A5" t="inlineStr"><is><t>Total</t></is></c>
      <c r="C5"><f>SUBTOTAL(109,tblChar[hp])</f></c>
      <c r="D5"><f>SUBTOTAL(109,tblChar[hp2])</f></c>
    </row>"#,
    );
    let worksheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>{rows}</sheetData>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#,
    );
    let worksheet_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    let table = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
       id="1" name="tblChar" displayName="tblChar" ref="A1:D5"
       headerRowCount="1" totalsRowCount="1" totalsRowShown="1">
  <autoFilter ref="A1:D4"/>
  <tableColumns count="4">
    <tableColumn id="1" name="id" totalsRowLabel="Total"/>
    <tableColumn id="2" name="name"/>
    <tableColumn id="3" name="hp" totalsRowFunction="sum"/>
    <tableColumn id="4" name="hp2" totalsRowFunction="sum"><calculatedColumnFormula>tblChar[[#This Row],[hp]]*2</calculatedColumnFormula></tableColumn>
  </tableColumns>
</table>"#;
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/worksheets/sheet1.xml", &worksheet),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                worksheet_relationships,
            ),
            ("xl/tables/table1.xml", table),
        ],
        None,
    )
}

/// Sheet `Data` filters `B1:D10` with a sheet-level auto-filter whose columns 0 and 2 carry
/// criteria under a sort state, named by the hidden `_xlnm._FilterDatabase`. Sheet `Sales` holds
/// table `Sales` on `B1:D4` with column IDs 1, 5, and 3 and a filter on its second column; the
/// text headers `Code` in A1 and `Margin` in E1 sit beside it, and G1:H1 count its headers and
/// sum its `Margin` column.
pub fn generated_column_widening_fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Sales" sheetId="2" r:id="rId2"/>
  </sheets>
  <definedNames><definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$B$1:$D$10</definedName></definedNames>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let mut data_rows = String::from(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Id</t></is></c><c r="B1" t="inlineStr"><is><t>Region</t></is></c><c r="C1" t="inlineStr"><is><t>Amount</t></is></c><c r="D1" t="inlineStr"><is><t>Note</t></is></c><c r="E1" t="inlineStr"><is><t>Extra</t></is></c></row>"#,
    );
    for row in 2..=10 {
        let region = if row % 2 == 0 { "North" } else { "South" };
        data_rows.push_str(&format!(
            r#"<row r="{row}"><c r="A{row}"><v>{id}</v></c><c r="B{row}" t="inlineStr"><is><t>{region}</t></is></c><c r="C{row}"><v>{amount}</v></c><c r="D{row}" t="inlineStr"><is><t>n{row}</t></is></c><c r="E{row}"><v>{row}</v></c></row>"#,
            id = row - 1,
            amount = row * 10,
        ));
    }
    let data = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>{data_rows}</sheetData>
  <autoFilter ref="B1:D10"><filterColumn colId="0"><filters><filter val="North"/></filters></filterColumn><filterColumn colId="2"><customFilters><customFilter operator="notEqual" val="n4"/></customFilters></filterColumn><sortState ref="B2:D10"><sortCondition descending="1" ref="C2:C10"/></sortState></autoFilter>
</worksheet>"#,
    );
    let sales = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>Code</t></is></c>
      <c r="B1" t="inlineStr"><is><t>Region</t></is></c>
      <c r="C1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="D1" t="inlineStr"><is><t>Note</t></is></c>
      <c r="E1" t="inlineStr"><is><t>Margin</t></is></c>
      <c r="G1"><f>COUNTA(Sales[#Headers])</f></c>
      <c r="H1"><f>SUM(Sales[Margin])</f></c>
    </row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>c1</t></is></c><c r="B2" t="inlineStr"><is><t>North</t></is></c><c r="C2"><v>10</v></c><c r="D2" t="inlineStr"><is><t>x</t></is></c><c r="E2"><v>1</v></c></row>
    <row r="3"><c r="A3" t="inlineStr"><is><t>c2</t></is></c><c r="B3" t="inlineStr"><is><t>South</t></is></c><c r="C3"><v>20</v></c><c r="D3" t="inlineStr"><is><t>y</t></is></c><c r="E3"><v>2</v></c></row>
    <row r="4"><c r="A4" t="inlineStr"><is><t>c3</t></is></c><c r="B4" t="inlineStr"><is><t>West</t></is></c><c r="C4"><v>30</v></c><c r="D4" t="inlineStr"><is><t>z</t></is></c><c r="E4"><v>3</v></c></row>
  </sheetData>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#;
    let sales_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    let table = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Sales" displayName="Sales" ref="B1:D4" headerRowCount="1">
  <autoFilter ref="B1:D4"><filterColumn colId="1"><filters><filter val="20"/></filters></filterColumn></autoFilter>
  <tableColumns count="3">
    <tableColumn id="1" name="Region"/>
    <tableColumn id="5" name="Amount"/>
    <tableColumn id="3" name="Note"/>
  </tableColumns>
  <tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/>
</table>"#;
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/worksheets/sheet1.xml", &data),
            ("xl/worksheets/sheet2.xml", sales),
            ("xl/worksheets/_rels/sheet2.xml.rels", sales_relationships),
            ("xl/tables/table1.xml", table),
        ],
        None,
    )
}

/// A workbook of `sheet_count` worksheets named `Sheet1`, `Sheet2`, and so on, each holding
/// 1 in A1.
pub fn generated_workbook_with_sheet_count(sheet_count: u64) -> Vec<u8> {
    generated_workbook_with_worksheets(
        sheet_count,
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
    )
}

/// A one-sheet workbook whose column B carries style 2 (`0.00%`), whose rows 2 and 5 carry style 1
/// (`0.00` in bold) with `customFormat`, and whose row 4 declares style 1 without `customFormat`.
/// A1 holds a value in style 2 and D2 a value in no style; B1 (no style) and C1 (style 1) are
/// blank cells, and so are E1 and E2, which name style 9, a `cellXfs` entry that does not exist.
pub fn generated_row_column_style_workbook() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Styled" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="2"><font><sz val="11"/></font><font><b/><sz val="11"/></font></fonts>
  <cellXfs count="3">
    <xf numFmtId="0" fontId="0"/>
    <xf numFmtId="2" fontId="1" applyNumberFormat="1" applyFont="1"/>
    <xf numFmtId="10" fontId="0" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"#;
    let worksheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <cols><col min="2" max="2" width="12" style="2" customWidth="1"/></cols>
  <sheetData>
    <row r="1"><c r="A1" s="2"><v>0.5</v></c><c r="B1"/><c r="C1" s="1"/><c r="E1" s="9"/></row>
    <row r="2" s="1" customFormat="1"><c r="D2"><v>1</v></c><c r="E2" s="9"/></row>
    <row r="4" s="1"/>
    <row r="5" s="1" customFormat="1"/>
  </sheetData>
</worksheet>"#;
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/styles.xml", styles),
            ("xl/worksheets/sheet1.xml", worksheet),
        ],
        None,
    )
}

/// A one-sheet workbook whose empty cell table is written as `<sheetData/>`, as Excel saves an
/// empty worksheet.
pub fn generated_empty_sheet_data_workbook() -> Vec<u8> {
    generated_workbook_with_worksheets(
        1,
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1"/><sheetData/></worksheet>"#,
    )
}

/// A one-sheet workbook with the legacy array formula `{=SEQUENCE(2)}` in D1:D2.
pub fn generated_legacy_array_workbook() -> Vec<u8> {
    generated_workbook_with_worksheets(
        1,
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="D1"><f t="array" ref="D1:D2">SEQUENCE(2)</f><v>1</v></c></row><row r="2"><c r="D2"><v>2</v></c></row></sheetData></worksheet>"#,
    )
}

/// A one-sheet workbook with a one-column shared formula group anchored at B1 over B1:B3 and a
/// two-dimensional group anchored at D1 over D1:E2.
pub fn generated_shared_formula_workbook() -> Vec<u8> {
    generated_workbook_with_worksheets(
        1,
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="shared" ref="B1:B3" si="0">A1*2</f><v>2</v></c><c r="D1"><f t="shared" ref="D1:E2" si="1">$A1+10</f><v>11</v></c><c r="E1"><f t="shared" si="1"/><v>11</v></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><f t="shared" si="0"/><v>4</v></c><c r="D2"><f t="shared" si="1"/><v>12</v></c><c r="E2"><f t="shared" si="1"/><v>12</v></c></row><row r="3"><c r="A3"><v>3</v></c><c r="B3"><f t="shared" si="0"/><v>6</v></c></row></sheetData></worksheet>"#,
    )
}

fn generated_workbook_with_worksheets(sheet_count: u64, worksheet: &str) -> Vec<u8> {
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    let mut sheets = String::new();
    let mut relationships = String::new();
    let mut worksheet_names = Vec::new();
    for index in 1..=sheet_count {
        content_types.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        sheets.push_str(&format!(
            r#"<sheet name="Sheet{index}" sheetId="{index}" r:id="rId{index}"/>"#
        ));
        relationships.push_str(&format!(
            r#"<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>"#
        ));
        worksheet_names.push(format!("xl/worksheets/sheet{index}.xml"));
    }
    content_types.push_str("</Types>");
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>{sheets}</sheets>
</workbook>"#
    );
    let workbook_relationships = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationships}</Relationships>"#
    );
    let mut entries = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook.as_str()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships.as_str(),
        ),
    ];
    entries.extend(
        worksheet_names
            .iter()
            .map(|name| (name.as_str(), worksheet)),
    );
    build_archive(&entries, None)
}

/// Sheets `Data` and `Summary` whose conditional formats, data validations, sparklines,
/// hyperlinks, chart series, and pivot cache name `Data` outside any cell formula.
pub fn generated_sheet_reference_fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
  <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Summary" sheetId="2" r:id="rId2"/>
  </sheets>
  <pivotCaches><pivotCache cacheId="1" r:id="rId3"/></pivotCaches>
</workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/>
</Relationships>"#;
    let data = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>4</v></c><c r="C1"><v>7</v></c></row>
    <row r="2"><c r="A2"><v>2</v></c><c r="B2"><v>5</v></c></row>
    <row r="3"><c r="A3"><v>3</v></c><c r="B3"><v>6</v></c></row>
  </sheetData>
  <hyperlinks><hyperlink ref="D1" location="Summary!A1" display="Summary"/></hyperlinks>
</worksheet>"#;
    let summary = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>
    <row r="1"><c r="A1"><f>SUM(Data!A1:A3)</f><v>6</v></c></row>
  </sheetData>
  <conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"><formula>Data!$A$1&gt;0</formula></cfRule></conditionalFormatting>
  <conditionalFormatting sqref="A2:A3"><cfRule type="colorScale" priority="3"><colorScale><cfvo type="formula" val="Data!$A$1"/><cfvo type="max"/><color rgb="FFF8696B"/><color rgb="FF63BE7B"/></colorScale></cfRule></conditionalFormatting>
  <dataValidations count="1"><dataValidation type="list" allowBlank="1" sqref="B1"><formula1>Data!$A$1:$A$3</formula1></dataValidation></dataValidations>
  <hyperlinks>
    <hyperlink ref="C1" location="Data!A1" display="Data"/>
    <hyperlink ref="C2" r:id="rId2" location="Data!A1"/>
  </hyperlinks>
  <drawing r:id="rId1"/>
  <extLst>
    <ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">
      <x14:conditionalFormattings>
        <x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="expression" priority="2" id="{00000000-0000-0000-0000-000000000001}"><xm:f>Data!$A$2&gt;1</xm:f><x14:dxf/></x14:cfRule><xm:sqref>A2</xm:sqref></x14:conditionalFormatting>
      </x14:conditionalFormattings>
    </ext>
    <ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">
      <x14:dataValidations count="1" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">
        <x14:dataValidation type="list" allowBlank="1"><x14:formula1><xm:f>Data!$B$1:$B$3</xm:f></x14:formula1><xm:sqref>D1</xm:sqref></x14:dataValidation>
      </x14:dataValidations>
    </ext>
    <ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">
      <x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">
        <x14:sparklineGroup><x14:colorSeries rgb="FF376092"/><x14:sparklines><x14:sparkline><xm:f>Data!A1:C1</xm:f><xm:sqref>E1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup>
      </x14:sparklineGroups>
    </ext>
  </extLst>
</worksheet>"#;
    let summary_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/other.xlsx" TargetMode="External"/>
</Relationships>"#;
    let drawing = r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
          xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>15</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
    <xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame>
    <xdr:clientData/>
  </xdr:twoCellAnchor>
</xdr:wsDr>"#;
    let drawing_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
</Relationships>"#;
    let chart = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:plotArea>
      <c:barChart>
        <c:barDir val="col"/>
        <c:ser>
          <c:idx val="0"/><c:order val="0"/>
          <c:tx><c:strRef><c:f>Summary!$A$1</c:f></c:strRef></c:tx>
          <c:val><c:numRef><c:f>Data!$A$1:$A$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt><c:pt idx="2"><c:v>3</c:v></c:pt></c:numCache></c:numRef></c:val>
        </c:ser>
        <c:axId val="1"/><c:axId val="2"/>
      </c:barChart>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#;
    let pivot_cache = r#"<?xml version="1.0" encoding="UTF-8"?>
<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" refreshOnLoad="1" recordCount="0">
  <cacheSource type="worksheet"><worksheetSource ref="A1:B3" sheet="Data"/></cacheSource>
  <cacheFields count="2"><cacheField name="A" numFmtId="0"><sharedItems/></cacheField><cacheField name="B" numFmtId="0"><sharedItems/></cacheField></cacheFields>
</pivotCacheDefinition>"#;
    build_archive(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_relationships),
            ("xl/worksheets/sheet1.xml", data),
            ("xl/worksheets/sheet2.xml", summary),
            ("xl/worksheets/_rels/sheet2.xml.rels", summary_relationships),
            ("xl/drawings/drawing1.xml", drawing),
            ("xl/drawings/_rels/drawing1.xml.rels", drawing_relationships),
            ("xl/charts/chart1.xml", chart),
            ("xl/pivotCache/pivotCacheDefinition1.xml", pivot_cache),
        ],
        None,
    )
}

pub fn generated_workbook_with_comment(profile: ProducerProfile, comment: &str) -> Vec<u8> {
    generated_workbook_with_archive_comment(profile, Some(comment))
}

fn generated_workbook_with_archive_comment(
    profile: ProducerProfile,
    archive_comment: Option<&str>,
) -> Vec<u8> {
    let inputs = inputs_worksheet(profile);
    let calculations = calculations_worksheet(profile);
    build_archive(
        &[
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
            ("xl/styles.xml", STYLES),
            ("xl/worksheets/sheet1.xml", &inputs),
            ("xl/worksheets/sheet2.xml", &calculations),
        ],
        archive_comment,
    )
}

pub fn generated_xlsb_package() -> Vec<u8> {
    build_archive(
        &[
            ("[Content_Types].xml", XLSB_CONTENT_TYPES),
            ("_rels/.rels", XLSB_ROOT_RELATIONSHIPS),
            ("xl/workbook.bin", "binary workbook placeholder"),
        ],
        None,
    )
}

fn inputs_worksheet(profile: ProducerProfile) -> String {
    let logical = match profile {
        ProducerProfile::LibreOffice => r#"<c r="B4" t="b"><f>TRUE()</f><v>1</v></c>"#,
        _ => r#"<c r="B4" t="b"><v>1</v></c>"#,
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="2"><c r="B2"><v>42.5</v></c></row>
    <row r="3"><c r="B3" t="inlineStr"><is><t>CellRune</t></is></c></row>
    <row r="4">{logical}</row>
    <row r="5"><c r="B5" s="1"><v>46225</v></c></row>
    <row r="6"><c r="B6" t="inlineStr"><is><t>한글 Ω</t></is></c></row>
    <row r="7"><c r="B7"><v>-3.25</v></c></row>
  </sheetData>
</worksheet>"#
    )
}

fn calculations_worksheet(profile: ProducerProfile) -> String {
    let double = formula_cell(profile, "B2", "Inputs!B2*2", None, "85");
    let sum = formula_cell(profile, "B3", "SUM(Inputs!B2,7.5)", None, "50");
    let lower = formula_cell(profile, "B4", "LOWER(Inputs!B3)", Some("str"), "cellrune");
    let not = formula_cell(profile, "B5", "NOT(Inputs!B4)", Some("b"), "0");
    let empty_text = formula_cell(
        profile,
        "B6",
        "IF(Inputs!B2&gt;0,&quot;&quot;,&quot;x&quot;)",
        Some("str"),
        "",
    );
    let date = formula_cell(profile, "B7", "Inputs!B5+1", None, "46226")
        .replace(r#"r="B7""#, r#"r="B7" s="1""#);
    let division_by_zero = formula_cell(
        profile,
        "B8",
        "1/0",
        Some(if matches!(profile, ProducerProfile::GoogleSheets) {
            "str"
        } else {
            "e"
        }),
        "#DIV/0!",
    );
    let unicode = formula_cell(
        profile,
        "B9",
        "Inputs!B6&amp;&quot; / &quot;&amp;TEXT(Inputs!B2,&quot;0.0&quot;)",
        Some("str"),
        "한글 Ω / 42.5",
    );
    let false_literal = match profile {
        ProducerProfile::LibreOffice => r#"<c r="C5" t="b"><f>FALSE()</f><v>0</v></c>"#,
        _ => r#"<c r="C5" t="b"><v>0</v></c>"#,
    };
    let error_literal = match profile {
        ProducerProfile::LibreOffice => r#"<c r="C8" t="e"><f>#DIV/0!</f><v>#DIV/0!</v></c>"#,
        _ => r#"<c r="C8" t="e"><v>#DIV/0!</v></c>"#,
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="2">{double}</row>
    <row r="3">{sum}</row>
    <row r="4">{lower}</row>
    <row r="5">{not}{false_literal}</row>
    <row r="6">{empty_text}</row>
    <row r="7">{date}</row>
    <row r="8">{division_by_zero}{error_literal}</row>
    <row r="9">{unicode}</row>
  </sheetData>
</worksheet>"#
    )
}

fn formula_cell(
    profile: ProducerProfile,
    address: &str,
    formula: &str,
    value_type: Option<&str>,
    saved_value: &str,
) -> String {
    let value_type = value_type.map_or_else(String::new, |kind| format!(r#" t="{kind}""#));
    let saved_result = if profile.retains_saved_results() {
        format!("<v>{saved_value}</v>")
    } else {
        String::new()
    };
    format!(r#"<c r="{address}"{value_type}><f>{formula}</f>{saved_result}</c>"#)
}

/// A package of exactly the given parts, stored uncompressed.
pub fn generated_package(entries: &[(&str, &str)]) -> Vec<u8> {
    build_archive(entries, None)
}

fn build_archive(entries: &[(&str, &str)], archive_comment: Option<&str>) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output);
        if let Some(comment) = archive_comment {
            let _ = writer.set_comment(comment);
        }
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, contents) in entries {
            writer
                .start_file(*name, options)
                .expect("start generated XLSX part");
            writer
                .write_all(contents.as_bytes())
                .expect("write generated XLSX part");
        }
        writer.finish().expect("finish generated XLSX archive");
    }
    output.into_inner()
}

pub struct TemporaryWorkbook {
    path: PathBuf,
}

impl TemporaryWorkbook {
    pub fn new(bytes: &[u8]) -> Self {
        let sequence = TEMPORARY_WORKBOOK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cellrune-generated-{}-{sequence}.xlsx",
            std::process::id()
        ));
        fs::write(&path, bytes).expect("write temporary generated workbook");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryWorkbook {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
