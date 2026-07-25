use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use super::read_xlsx_bytes;
use crate::{
    CellAddress, CellContent, CellValue, DefinedNameScope, DiagnosticSeverity, FormulaCell,
    FormulaMetadata, ReadLimits, ReadOptions, SavedResult, SharedFormulaRole, XlsxErrorCode,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
  <definedNames>
    <definedName name="TaxRate">Sheet1!$A$1</definedName>
    <definedName name="LocalValue" localSheetId="0" hidden="1">$B$1</definedName>
  </definedNames>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const FORMULA_MATRIX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><f ca="1">1+1</f><v>2</v></c>
      <c r="B1" t="str"><f>&quot;text&quot;</f><v>text</v></c>
      <c r="C1" t="b"><f>1=1</f><v>1</v></c>
      <c r="D1" t="e"><f>NA()</f><v>#N/A</v></c>
      <c r="E1"><f>SUM(A1:D1)</f></c>
      <c r="F1"><f>1/0</f><v>not-a-number</v></c>
      <c r="G1" t="inlineStr"><f>&quot;inline&quot;</f><is><t>inline</t></is></c>
      <c r="H1"><f>2+2</f><v/></c>
      <c r="I1" t="str"><f>&quot;&quot;</f><v/></c>
    </row>
  </sheetData>
</worksheet>"#;

const CONTAINER_MATRIX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><f t="shared" ref="A1:A2" si="7">B1+$C$1+D$1+$E1</f></c>
      <c r="C1"><f t="array" ref="C1:C2" aca="1">ROW(C1:C2)</f></c>
      <c r="D1"><f t="dataTable" ref="D1:E2" r1="A1" dtr="1"/><v>1</v></c>
      <c r="F1" cm="1"><f t="array" ref="F1:F2" aca="1">SEQUENCE(2)</f></c>
    </row>
    <row r="2"><c r="A2"><f t="shared" si="7"/></c></row>
  </sheetData>
</worksheet>"#;

const EXCEL_FORMULA_VARIANTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="B1"><f t="shared" ref="A1:B2" si="9">A1+1</f><v>2</v></c>
    </row>
    <row r="2">
      <c r="B2"><f t="shared" si="9"/><v>3</v></c>
      <c r="C2" t="e"><f ca="1"/><v>#VALUE!</v></c>
    </row>
  </sheetData>
</worksheet>"#;

const METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray">
  <metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes>
  <futureMetadata name="XLDAPR" count="1">
    <bk><extLst><ext uri="dynamic-array"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk>
  </futureMetadata>
  <cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata>
</metadata>"#;

#[test]
fn preserves_formula_text_and_each_saved_result_state() {
    let archive = build_archive(FORMULA_MATRIX, None, false);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("formula matrix");
    let sheet = snapshot.sheet_by_name("Sheet1").expect("fixture sheet");

    let a1 = formula(sheet, "A1");
    assert_eq!(a1.text().expect("A1 text").as_str(), "1+1");
    assert!(a1.recalculate_always());
    assert_eq!(saved_number(a1), 2.0);
    assert_eq!(
        saved_value(formula(sheet, "B1")),
        &CellValue::Text("text".into())
    );
    assert_eq!(saved_value(formula(sheet, "C1")), &CellValue::Logical(true));
    assert_eq!(
        saved_value(formula(sheet, "D1")),
        &CellValue::Error(crate::ExcelError::NotAvailable)
    );
    assert!(matches!(
        formula(sheet, "E1").saved_result(),
        SavedResult::Missing
    ));
    assert!(matches!(
        formula(sheet, "H1").saved_result(),
        SavedResult::Missing
    ));
    assert_eq!(
        saved_value(formula(sheet, "I1")),
        &CellValue::Text(String::new())
    );

    let SavedResult::Invalid(invalid_number) = formula(sheet, "F1").saved_result() else {
        panic!("invalid numeric saved result");
    };
    assert_eq!(invalid_number.code().as_str(), "xlsx.saved_result.invalid");
    assert_eq!(invalid_number.raw_value(), Some("not-a-number"));

    let SavedResult::Invalid(unsupported_inline) = formula(sheet, "G1").saved_result() else {
        panic!("unsupported inline saved result");
    };
    assert_eq!(
        unsupported_inline.code().as_str(),
        "xlsx.saved_result.unsupported_type"
    );
    assert_eq!(unsupported_inline.raw_value(), Some("inline"));
}

#[test]
fn expands_shared_formulas_and_classifies_formula_containers() {
    let archive = build_archive(CONTAINER_MATRIX, Some(METADATA), false);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("container matrix");
    let sheet = snapshot.sheet_by_name("Sheet1").expect("fixture sheet");

    let anchor = formula(sheet, "A1");
    assert_eq!(
        anchor.text().expect("anchor text").as_str(),
        "B1+$C$1+D$1+$E1"
    );
    assert!(matches!(
        anchor.metadata(),
        FormulaMetadata::Shared {
            group_index: 7,
            role: SharedFormulaRole::Anchor,
            range: Some(_),
        }
    ));
    let follower = formula(sheet, "A2");
    assert_eq!(
        follower.text().expect("expanded follower").as_str(),
        "B2+$C$1+D$1+$E2"
    );
    assert!(matches!(
        follower.metadata(),
        FormulaMetadata::Shared {
            group_index: 7,
            role: SharedFormulaRole::Follower { .. },
            range: None,
        }
    ));

    assert!(matches!(
        formula(sheet, "C1").metadata(),
        FormulaMetadata::Array {
            always_calculate: true,
            ..
        }
    ));
    let data_table = formula(sheet, "D1");
    assert!(data_table.text().is_none());
    assert!(matches!(
        data_table.metadata(),
        FormulaMetadata::DataTable {
            row_oriented: true,
            two_dimensional: false,
            ..
        }
    ));
    assert!(matches!(
        formula(sheet, "F1").metadata(),
        FormulaMetadata::DynamicArray {
            range: Some(_),
            always_calculate: true,
        }
    ));

    assert_eq!(snapshot.defined_names().len(), 2);
    assert_eq!(snapshot.defined_names()[0].name(), "TaxRate");
    assert_eq!(
        snapshot.defined_names()[0].formula().as_str(),
        "Sheet1!$A$1"
    );
    assert_eq!(
        snapshot.defined_names()[0].scope(),
        DefinedNameScope::Workbook
    );
    assert!(snapshot.defined_names()[1].hidden());
    assert!(matches!(
        snapshot.defined_names()[1].scope(),
        DefinedNameScope::Sheet(_)
    ));
}

#[test]
fn accepts_excel_shared_anchors_inside_ranges_and_empty_normal_formulas() {
    let archive = build_archive(EXCEL_FORMULA_VARIANTS, None, false);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("formula variants");
    let sheet = snapshot.sheet_by_name("Sheet1").expect("fixture sheet");

    let anchor = formula(sheet, "B1");
    assert_eq!(anchor.text().expect("anchor text").as_str(), "A1+1");
    assert!(matches!(
        anchor.metadata(),
        FormulaMetadata::Shared {
            role: SharedFormulaRole::Anchor,
            ..
        }
    ));
    let follower = formula(sheet, "B2");
    assert_eq!(follower.text().expect("shifted follower").as_str(), "A2+1");

    let empty = formula(sheet, "C2");
    assert!(empty.text().is_none());
    assert!(empty.recalculate_always());
    assert_eq!(
        saved_value(empty),
        &CellValue::Error(crate::ExcelError::Value)
    );
    assert!(matches!(empty.metadata(), FormulaMetadata::Normal));
}

#[test]
fn rejects_invalid_formula_metadata_and_enforces_formula_budget() {
    let unknown_shared =
        CONTAINER_MATRIX.replace("<f t=\"shared\" si=\"7\"/>", "<f t=\"shared\" si=\"8\"/>");
    let error = read_xlsx_bytes(
        &build_archive(&unknown_shared, Some(METADATA), false),
        ReadOptions::default(),
    )
    .expect_err("unknown shared formula group");
    assert_eq!(error.code(), XlsxErrorCode::InvalidFormulaMetadata);

    let error = read_xlsx_bytes(
        &build_archive(CONTAINER_MATRIX, None, false),
        ReadOptions::default(),
    )
    .expect_err("cell metadata part is required");
    assert_eq!(error.code(), XlsxErrorCode::InvalidCellMetadata);

    let limits = ReadLimits::default()
        .with_max_formula_bytes(3)
        .expect("nonzero formula limit");
    let error = read_xlsx_bytes(
        &build_archive(FORMULA_MATRIX, None, false),
        ReadOptions::new(limits),
    )
    .expect_err("formula byte budget");
    assert_eq!(error.code(), XlsxErrorCode::FormulaTooLarge);

    let limits = ReadLimits::default()
        .with_max_total_formula_bytes(20)
        .expect("nonzero total formula limit");
    let error = read_xlsx_bytes(
        &build_archive(FORMULA_MATRIX, None, false),
        ReadOptions::new(limits),
    )
    .expect_err("total formula byte budget");
    assert_eq!(error.code(), XlsxErrorCode::TotalFormulaBytesTooLarge);
}

#[test]
fn reports_external_links_and_macros_without_opening_or_executing_them() {
    let archive = build_archive(FORMULA_MATRIX, None, true);
    let snapshot = read_xlsx_bytes(&archive, ReadOptions::default()).expect("diagnostic workbook");
    assert_eq!(snapshot.diagnostics().len(), 2);
    assert_eq!(
        snapshot.diagnostics()[0].code().as_str(),
        "xlsx.external_link"
    );
    assert_eq!(snapshot.diagnostics()[1].code().as_str(), "xlsx.macro");
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Warning)
    );
}

fn build_archive(sheet: &str, metadata: Option<&str>, compatibility_parts: bool) -> Vec<u8> {
    let mut content_types = metadata.map_or_else(
        || CONTENT_TYPES.to_owned(),
        |_| {
            CONTENT_TYPES.replace(
                "</Types>",
                "  <Override PartName=\"/xl/metadata.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml\"/>\n</Types>",
            )
        },
    );
    if compatibility_parts {
        content_types = content_types.replace(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        );
    }
    let mut relationships = WORKBOOK_RELATIONSHIPS.replace("</Relationships>", "");
    if metadata.is_some() {
        relationships.push_str(
            "  <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata\" Target=\"metadata.xml\"/>\n",
        );
    }
    if compatibility_parts {
        relationships.push_str(
            "  <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink\" Target=\"https://example.invalid/book.xlsx\" TargetMode=\"External\"/>\n  <Relationship Id=\"rId4\" Type=\"http://schemas.microsoft.com/office/2006/relationships/vbaProject\" Target=\"vbaProject.bin\"/>\n",
        );
    }
    relationships.push_str("</Relationships>");

    let mut entries = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", sheet),
    ];
    if let Some(metadata) = metadata {
        entries.push(("xl/metadata.xml", metadata));
    }

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

fn formula<'a>(sheet: &'a crate::Sheet, address: &str) -> &'a FormulaCell {
    let CellContent::Formula(formula) = sheet
        .cell(parse_address(address))
        .unwrap_or_else(|| panic!("missing formula {address}"))
        .content()
    else {
        panic!("non-formula {address}");
    };
    formula
}

fn parse_address(value: &str) -> CellAddress {
    let split = value
        .bytes()
        .position(|byte| byte.is_ascii_digit())
        .expect("test address row");
    let (column, row) = value.split_at(split);
    let column = column
        .bytes()
        .fold(0_u32, |index, byte| index * 26 + u32::from(byte - b'A' + 1));
    CellAddress::from_indices(row.parse().expect("test row"), column).expect("test address")
}

fn saved_value(formula: &FormulaCell) -> &CellValue {
    let SavedResult::Present(value) = formula.saved_result() else {
        panic!("saved result");
    };
    value
}

fn saved_number(formula: &FormulaCell) -> f64 {
    let CellValue::Number(value) = saved_value(formula) else {
        panic!("numeric saved result");
    };
    value.get()
}
