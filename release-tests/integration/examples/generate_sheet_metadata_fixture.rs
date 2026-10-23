use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
  <Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Lists" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="2">
    <font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font>
    <font><b/><sz val="12"/><color rgb="FFFF0000"/><name val="Arial"/></font>
  </fonts>
  <fills count="3">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill>
  </fills>
  <borders count="2">
    <border><left/><right/><top/><bottom/><diagonal/></border>
    <border><left/><right/><top/><bottom style="thin"><color indexed="64"/></bottom><diagonal/></border>
  </borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="2">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment horizontal="center" wrapText="1"/></xf>
  </cellXfs>
</styleSheet>"#;

const DATA_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/docs?a=1&amp;b=2" TargetMode="External"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/>
</Relationships>"#;

const COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors><author>Ana</author><author>Bo</author></authors>
  <commentList>
    <comment ref="A1" authorId="0"><text><r><rPr><b/></rPr><t>Ana:</t></r><r><t xml:space="preserve"> check &amp; confirm</t></r></text></comment>
    <comment ref="C3" authorId="1"><text><t>Plain note</t></text></comment>
  </commentList>
</comments>"#;

const DATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <cols><col min="1" max="2" width="18.5" customWidth="1"/></cols>
  <sheetData><row r="1" ht="24" customHeight="1"><c r="A1" s="1" t="inlineStr"><is><t>Yes</t></is></c></row></sheetData>
  <dataValidations count="2">
    <dataValidation type="list" allowBlank="1" showErrorMessage="1" errorStyle="warning" error="Pick one" sqref="A1:A5 C1"><formula1>"Yes,No"</formula1></dataValidation>
    <dataValidation type="whole" operator="notBetween" showDropDown="1" promptTitle="Range" prompt="Outside 1-9" sqref="B2"><formula1>1</formula1><formula2>9</formula2></dataValidation>
  </dataValidations>
  <hyperlinks>
    <hyperlink ref="B4" r:id="rId1" display="Docs" tooltip="Open the docs"/>
    <hyperlink ref="B5:C5" location="Lists!A1" display="Lists"/>
  </hyperlinks>
  <extLst>
    <ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">
      <x14:dataValidations count="1" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">
        <x14:dataValidation type="list" allowBlank="1"><x14:formula1><xm:f>Lists!$A$1:$A$2</xm:f></x14:formula1><xm:sqref>D1</xm:sqref></x14:dataValidation>
      </x14:dataValidations>
    </ext>
  </extLst>
</worksheet>"#;

const LISTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>North</t></is></c></row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>South</t></is></c></row>
  </sheetData>
</worksheet>"#;

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
        ("xl/worksheets/sheet1.xml", DATA),
        ("xl/worksheets/sheet2.xml", LISTS),
        ("xl/styles.xml", STYLES),
        ("xl/worksheets/_rels/sheet1.xml.rels", DATA_RELATIONSHIPS),
        ("xl/comments1.xml", COMMENTS),
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../binding-contract/sheet-metadata.xlsx")
}
