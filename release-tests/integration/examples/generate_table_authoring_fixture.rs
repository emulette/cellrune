use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/tables/table2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
  <definedNames>
    <definedName name="SalesAmount">Sales[Amount]</definedName>
    <definedName name="EmptyAmount">EmptySales[Amount]</definedName>
  </definedNames>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const WORKSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>Item</t></is></c>
      <c r="B1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="C1" t="inlineStr"><is><t>Tax</t></is></c>
      <c r="E1"><f>SUM(Sales[Amount])</f></c>
      <c r="G1" t="inlineStr"><is><t>Item</t></is></c>
      <c r="H1" t="inlineStr"><is><t>Amount</t></is></c>
    </row>
    <row r="2">
      <c r="A2" t="inlineStr"><is><t>North</t></is></c>
      <c r="B2"><v>10</v></c>
      <c r="C2"><f>[@Amount]*0.1</f></c>
    </row>
    <row r="3">
      <c r="A3" t="inlineStr"><is><t>South</t></is></c>
      <c r="B3"><v>20</v></c>
      <c r="C3"><f>[@Amount]*0.1</f></c>
    </row>
    <row r="4">
      <c r="A4" t="inlineStr"><is><t>Total</t></is></c>
      <c r="B4"><f>SUBTOTAL(109,Sales[Amount])</f></c>
      <c r="C4"><f>SUBTOTAL(109,Sales[Tax])</f></c>
    </row>
  </sheetData>
  <tableParts count="2"><tablePart r:id="rId1"/><tablePart r:id="rId2"/></tableParts>
</worksheet>"#;

const WORKSHEET_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table2.xml"/>
</Relationships>"#;

const TABLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
       id="1" name="Sales" displayName="Sales" ref="A1:C4"
       headerRowCount="1" totalsRowCount="1" totalsRowShown="1">
  <autoFilter ref="A1:C3">
    <sortState ref="A2:C3"><sortCondition ref="B2:B3" descending="1"/></sortState>
  </autoFilter>
  <sortState ref="A2:C3"><sortCondition ref="B2:B3"/></sortState>
  <tableColumns count="3">
    <tableColumn id="1" name="Item" totalsRowLabel="Total"/>
    <tableColumn id="2" name="Amount" totalsRowFunction="sum"/>
    <tableColumn id="3" name="Tax" totalsRowFunction="sum">
      <calculatedColumnFormula>[@Amount]*0.1</calculatedColumnFormula>
    </tableColumn>
  </tableColumns>
</table>"#;

const EMPTY_TABLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
       id="2" name="EmptySales" displayName="EmptySales" ref="G1:H1"
       headerRowCount="1" totalsRowCount="0">
  <autoFilter ref="G1:H1"/>
  <tableColumns count="2">
    <tableColumn id="1" name="Item"/>
    <tableColumn id="2" name="Amount">
      <calculatedColumnFormula>[@Amount]</calculatedColumnFormula>
    </tableColumn>
  </tableColumns>
</table>"#;

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map_or_else(default_output, PathBuf::from);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (path, content) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/worksheets/sheet1.xml", WORKSHEET),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            WORKSHEET_RELATIONSHIPS,
        ),
        ("xl/tables/table1.xml", TABLE),
        ("xl/tables/table2.xml", EMPTY_TABLE),
    ] {
        writer.start_file(path, options).expect("fixture entry");
        writer
            .write_all(content.as_bytes())
            .expect("fixture content");
    }
    let bytes = writer.finish().expect("fixture archive").into_inner();
    fs::write(&output, bytes).expect("write fixture");
    println!("{}", output.display());
}

fn default_output() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../binding-contract/table-authoring-v2.xlsx")
}
