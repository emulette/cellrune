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
