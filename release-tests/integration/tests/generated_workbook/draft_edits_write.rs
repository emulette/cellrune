use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use cellrune::{
    CalculationCellId, CalculationOptions, CellAddress, CellValue, DefinedName, DefinedNameScope,
    EditBatch, FormulaText, OpenOptions, SheetId, SheetName, WorkbookChange, WorkbookDraft,
    WriteOptions, XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_edits_bytes,
};
use zip::read::ZipArchive;

use crate::support::generated_xlsx::{generated_column_widening_fixture, generated_package};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>
</Relationships>"#;

const OTHER_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>5</v></c><c r="B1"><f>A1*3</f><v>15</v></c></row></sheetData></worksheet>"#;

/// A two-sheet workbook whose first sheet holds `data_cells` after the values A1:A3 = 1, 2, 3,
/// with a calculation chain listing `chain_cells` and the given defined names.
fn package(data_cells: &str, chain_cells: &[&str], defined_names: &str) -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets>{defined_names}<calcPr calcId="191029"/></workbook>"#
    );
    let data = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c>{data_cells}</row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row></sheetData></worksheet>"#
    );
    let chain = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{}<c r="B1" i="2"/></calcChain>"#,
        chain_cells
            .iter()
            .enumerate()
            .map(|(index, cell)| if index == 0 {
                format!(r#"<c r="{cell}" i="1"/>"#)
            } else {
                format!(r#"<c r="{cell}"/>"#)
            })
            .collect::<String>()
    );
    generated_package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/worksheets/sheet1.xml", data.as_str()),
        ("xl/worksheets/sheet2.xml", OTHER_SHEET),
        ("xl/calcChain.xml", chain.as_str()),
    ])
}

const STATIC_CELLS: &str = r#"<c r="B1"><f>SUM(A1:A3)</f><v>6</v></c><c r="C1"><f>CUBEVALUE("cube")</f><v>0</v></c><c r="D1"><f>A1*2</f><v>2</v></c><c r="E1"><f>SUM(A:A)</f><v>6</v></c><c r="F1"><f>B1+1</f><v>7</v></c>"#;
const STATIC_CHAIN: &[&str] = &["B1", "C1", "D1", "E1", "F1"];

#[test]
fn unrelated_edits_keep_saved_results_the_calculation_chain_and_calc_properties() {
    let source = package(STATIC_CELLS, STATIC_CHAIN, "");
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(data_sheet(&draft), address("Z50"), number(9.0))
        .expect("unrelated edit");

    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("an unrelated edit beside an uncalculable formula must save");

    let report = output.report();
    assert!(report.affected_formulas().is_empty());
    assert!(!report.requests_host_recalculation());
    assert!(report.removed_parts().is_empty());
    assert_eq!(
        report
            .changed_parts()
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>(),
        ["xl/worksheets/sheet1.xml"]
    );
    let before = archive_parts(&source);
    let after = archive_parts(output.bytes());
    for (name, bytes) in &before {
        if name != "xl/worksheets/sheet1.xml" {
            assert_eq!(after.get(name), Some(bytes), "{name} must keep its bytes");
        }
    }
    let sheet = part_text(output.bytes(), "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(STATIC_CELLS), "{sheet}");
    assert!(sheet.contains(r#"<c r="Z50"><v>9</v></c>"#), "{sheet}");
}

#[test]
fn edits_remove_only_the_saved_results_they_can_change() {
    let source = package(STATIC_CELLS, STATIC_CHAIN, "");
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let sheet_id = data_sheet(&draft);
    draft
        .set_cell_value(sheet_id, address("A2"), number(20.0))
        .expect("edit a referenced value");

    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");

    assert_eq!(
        affected(output.report().affected_formulas()),
        cells(sheet_id, &["B1", "E1", "F1"])
    );
    assert!(output.report().requests_host_recalculation());
    assert!(
        output.report().removed_parts().is_empty(),
        "the formula layout is unchanged"
    );
    let sheet = part_text(output.bytes(), "xl/worksheets/sheet1.xml");
    for kept in [
        r#"<c r="C1"><f>CUBEVALUE("cube")</f><v>0</v></c>"#,
        r#"<c r="D1"><f>A1*2</f><v>2</v></c>"#,
    ] {
        assert!(sheet.contains(kept), "{sheet}");
    }
    for cleared in ["B1", "E1", "F1"] {
        assert!(
            !sheet.contains(&format!(r#"<c r="{cleared}"><f>"#))
                || !cell_xml(&sheet, cleared).contains("<v>"),
            "{cleared} must lose its saved result: {sheet}"
        );
    }
    let workbook = part_text(output.bytes(), "xl/workbook.xml");
    assert!(workbook.contains(r#"fullCalcOnLoad="1""#), "{workbook}");
    assert!(workbook.contains(r#"calcId="191029""#), "{workbook}");
    assert!(!workbook.contains(r#"forceFullCalc="1""#), "{workbook}");
    assert!(archive_parts(output.bytes()).contains_key("xl/calcChain.xml"));
}

const RULE_CELLS: &str = r#"<c r="G1"><f>Derived+0</f><v>2</v></c><c r="H1" t="e"><f>Rate*2</f><v>#NAME?</v></c><c r="I1"><f>SHEETS()</f><v>2</v></c><c r="J1"><f>INDIRECT("A1")</f><v>1</v></c><c r="K1" t="e"><f>ROUND(A2)</f><v>#VALUE!</v></c><c r="L1"><f>A3*1</f><v>3</v></c><c r="M1" t="e"><f>SUM(A1</f><v>#NAME?</v></c><c r="N1"><f>SUM(B2:INDEX(D2:D20,A1))</f><v>0</v></c><c r="O1"><f t="array" ref="O1:O2">A1:A2*10</f><v>10</v></c>"#;
const RULE_CHAIN: &[&str] = &["G1", "H1", "I1", "J1", "K1", "L1", "M1", "N1", "O1"];
const RULE_NAMES: &str = r#"<definedNames><definedName name="Base">Data!$A$1</definedName><definedName name="Derived">Base*2</definedName></definedNames>"#;

/// Formulas whose references are unknown before evaluation are affected by any edit.
const ALWAYS: &[&str] = &["J1", "K1", "M1"];

#[test]
fn names_sheets_and_unknowable_references_follow_the_impact_rules() {
    let source = package(RULE_CELLS, RULE_CHAIN, RULE_NAMES);
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let sheet_id = data_sheet(&WorkbookDraft::from_document(&document));
    let rule = |changes: Vec<WorkbookChange>, expected: &[&str]| {
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new(changes))
            .expect("apply edit");
        let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");
        let mut expected = expected.to_vec();
        expected.extend_from_slice(ALWAYS);
        assert_eq!(
            affected(output.report().affected_formulas()),
            cells(sheet_id, &expected)
        );
        output
    };

    rule(
        vec![WorkbookChange::set_defined_name(
            DefinedName::new(
                "Rate",
                DefinedNameScope::Workbook,
                FormulaText::from_user_input("=3").expect("formula"),
                false,
            )
            .expect("name"),
        )],
        &["H1"],
    );
    rule(
        vec![WorkbookChange::set_defined_name(
            DefinedName::new(
                "Base",
                DefinedNameScope::Workbook,
                FormulaText::from_user_input("=Data!$A$2").expect("formula"),
                false,
            )
            .expect("name"),
        )],
        &["G1"],
    );
    let added = rule(
        vec![WorkbookChange::add_sheet(
            SheetName::new("New").expect("sheet name"),
        )],
        &["I1"],
    );
    assert!(
        added.report().removed_parts().is_empty(),
        "a sheet without formulas leaves the formula layout unchanged"
    );
    rule(
        vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("C5"),
            number(1.0),
        )],
        &["N1"],
    );
    rule(
        vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("A1"),
            number(4.0),
        )],
        &["G1", "N1", "O1"],
    );
    let new_formula = rule(
        vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("Z1"),
            FormulaText::from_user_input("=1+1").expect("formula"),
        )],
        &["Z1"],
    );
    assert_eq!(
        new_formula
            .report()
            .removed_parts()
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>(),
        ["xl/calcChain.xml"],
        "a new formula invalidates the calculation chain"
    );
}

#[test]
fn widening_a_table_affects_the_formulas_that_refer_to_it() {
    let document =
        open_xlsx_document_bytes(&generated_column_widening_fixture(), OpenOptions::default())
            .expect("open");
    let sales = document.workbook().sheets()[1].id();
    let table_id = document.workbook().sheets()[1].tables()[0].id();
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::widen_table_columns(
            table_id,
            cellrune::CellRange::new(address("B1"), address("E4")).expect("range"),
        )]))
        .expect("widen the table");

    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");

    assert_eq!(
        affected(output.report().affected_formulas()),
        cells(sales, &["G1", "H1"])
    );
    assert!(output.report().requests_host_recalculation());
    assert!(
        output
            .report()
            .changed_parts()
            .iter()
            .any(|part| part.as_str() == "xl/tables/table1.xml")
    );
}

const METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="dynamic-array"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;

/// Dynamic arrays: P1 spills SEQUENCE(3) over P1:P3 and Q1 reads P1; R1 failed to spill over R2.
fn spill_package() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/metadata.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml"/>
</Types>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><calcPr calcId="191029"/></workbook>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/>
</Relationships>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="P1" cm="1"><f t="array" ref="P1:P3">SEQUENCE(3)</f><v>1</v></c><c r="Q1"><f>P1*1</f><v>1</v></c><c r="R1" cm="1" t="e"><f t="array" ref="R1">SEQUENCE(2)</f><v>#SPILL!</v></c></row><row r="2"><c r="P2"><v>2</v></c><c r="R2"><v>7</v></c></row><row r="3"><c r="P3"><v>3</v></c></row></sheetData></worksheet>"#;
    generated_package(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/metadata.xml", METADATA),
    ])
}

#[test]
fn spills_are_affected_by_writes_inside_them_and_by_clears_that_free_them() {
    let document =
        open_xlsx_document_bytes(&spill_package(), OpenOptions::default()).expect("open");
    let sheet_id = document.workbook().sheets()[0].id();
    let write = |change: WorkbookChange| {
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new([change]))
            .expect("apply edit");
        let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");
        affected(output.report().affected_formulas())
    };

    assert_eq!(
        write(WorkbookChange::set_cell_value(
            sheet_id,
            address("P2"),
            number(5.0)
        )),
        cells(sheet_id, &["P1", "Q1"])
    );
    assert_eq!(
        write(WorkbookChange::clear_cell(sheet_id, address("R2"))),
        cells(sheet_id, &["R1"])
    );
    assert_eq!(
        write(WorkbookChange::set_cell_value(
            sheet_id,
            address("A1"),
            number(2.0)
        )),
        BTreeSet::new(),
        "an edit outside every spill leaves the anchors alone"
    );
}

#[test]
fn qualified_names_added_sheets_and_names_with_rejected_calls_are_traced() {
    let write = |cells: &str, names: &str, change: &dyn Fn(SheetId) -> WorkbookChange| {
        let document =
            open_xlsx_document_bytes(&package(cells, &["B1"], names), OpenOptions::default())
                .expect("open");
        let sheet_id = document.workbook().sheets()[0].id();
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new([change(sheet_id)]))
            .expect("apply edit");
        let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");
        (sheet_id, affected(output.report().affected_formulas()))
    };
    let set_a1 = |sheet_id| WorkbookChange::set_cell_value(sheet_id, address("A1"), number(5.0));

    // CellRune does not calculate a sheet-qualified name, but the cells it reads still count.
    let (sheet_id, qualified) = write(
        r#"<c r="B1"><f>Data!TaxRate*2</f><v>2</v></c><c r="C1"><f>B1+1</f><v>3</v></c>"#,
        r#"<definedNames><definedName name="TaxRate" localSheetId="0">Data!$A$1</definedName></definedNames>"#,
        &set_a1,
    );
    assert_eq!(qualified, cells(sheet_id, &["B1", "C1"]));

    let (sheet_id, added_sheet) = write(
        r#"<c r="B1"><f>IFERROR(New!A1,-1)</f><v>-1</v></c><c r="C1"><f>A1*2</f><v>2</v></c>"#,
        "",
        &|_| WorkbookChange::add_sheet(SheetName::new("New").expect("sheet name")),
    );
    assert_eq!(added_sheet, cells(sheet_id, &["B1"]));

    let (sheet_id, rejected) = write(
        r#"<c r="B1"><f>BadCall</f><v>1</v></c><c r="C1"><f>A2*2</f><v>4</v></c>"#,
        r#"<definedNames><definedName name="BadCall">ROUND(Data!A1)</definedName></definedNames>"#,
        &set_a1,
    );
    assert_eq!(rejected, cells(sheet_id, &["B1"]));
}

#[test]
fn edits_require_a_draft_opened_from_a_document() {
    let error = write_xlsx_draft_edits_bytes(&WorkbookDraft::new(), WriteOptions::default())
        .expect_err("a new draft has no package to edit");
    assert_eq!(error.code(), XlsxWriteErrorCode::InvalidPackagePlan);
}

/// A workbook mixing every formula kind the differential check edits: plain and whole-column
/// ranges, an `INDEX` range end, defined names, another sheet, a shared formula group (J1:J3), a
/// legacy array (K1:K2), a declared dynamic array spill (L1:L3), and a table T (N1:O4).
fn differential_package() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/metadata.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets>{RULE_NAMES}<calcPr calcId="191029"/></workbook>"#
    );
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/>
</Relationships>"#;
    let sheet_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    let data = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>SUM(A1:A3)</f><v>6</v></c><c r="C1"><f>VLOOKUP(2,A1:B3,1,FALSE)</f><v>2</v></c><c r="D1"><f>SUM(A:A)</f><v>6</v></c><c r="E1"><f>B1*D1</f><v>36</v></c><c r="F1"><f>SUM(B2:INDEX(D2:D9,A1))</f><v>0</v></c><c r="G1"><f>COUNT(C2:E9)</f><v>0</v></c><c r="H1"><f>Derived+F1</f><v>2</v></c><c r="I1"><f>MAX(Other!A1:B3)</f><v>15</v></c><c r="J1"><f t="shared" ref="J1:J3" si="0">A1*3</f><v>3</v></c><c r="K1"><f t="array" ref="K1:K2">A1:A2*10</f><v>10</v></c><c r="L1" cm="1"><f t="array" ref="L1:L3">SEQUENCE(3)</f><v>1</v></c><c r="M1"><f>SUM(L:L)+J2</f><v>12</v></c><c r="N1" t="inlineStr"><is><t>Key</t></is></c><c r="O1" t="inlineStr"><is><t>Val</t></is></c><c r="P1"><f>SUM(T[Val])</f><v>6</v></c></row><row r="2"><c r="A2"><v>2</v></c><c r="J2"><f t="shared" si="0"/><v>6</v></c><c r="K2"><v>20</v></c><c r="L2"><v>2</v></c><c r="N2" t="inlineStr"><is><t>a</t></is></c><c r="O2"><v>1</v></c></row><row r="3"><c r="A3"><v>3</v></c><c r="J3"><f t="shared" si="0"/><v>9</v></c><c r="L3"><v>3</v></c><c r="N3" t="inlineStr"><is><t>b</t></is></c><c r="O3"><v>2</v></c></row><row r="4"><c r="N4" t="inlineStr"><is><t>c</t></is></c><c r="O4"><v>3</v></c></row></sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>"#;
    let table = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="T" displayName="T" ref="N1:O4" headerRowCount="1"><autoFilter ref="N1:O4"/><tableColumns count="2"><tableColumn id="1" name="Key"/><tableColumn id="2" name="Val"/></tableColumns></table>"#;
    generated_package(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_relationships),
        ("xl/worksheets/sheet2.xml", OTHER_SHEET),
        ("xl/metadata.xml", METADATA),
        ("xl/tables/table1.xml", table),
    ])
}

/// The main omission check: every formula whose calculated result changes across a generated
/// edit sequence must be in the affected set.
#[test]
fn affected_formulas_cover_every_result_the_edits_change() {
    let document =
        open_xlsx_document_bytes(&differential_package(), OpenOptions::default()).expect("open");
    let before = calculate_workbook(document.workbook(), CalculationOptions::default());
    let sheet_id = data_sheet(&WorkbookDraft::from_document(&document));
    let other_id = document.workbook().sheets()[1].id();
    let targets = [
        "A1", "A2", "A3", "A4", "B2", "C3", "D5", "E7", "C9", "B3", "J1", "J2", "K2", "L2", "L4",
        "N1", "O1", "O3", "N5",
    ];
    let mut state = 0x9e37_79b9_u32;
    let mut next = |bound: usize| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as usize % bound
    };
    let mut saved = 0;
    for sequence in 0..120 {
        let mut draft = WorkbookDraft::from_document(&document);
        for _ in 0..=next(3) {
            let target = address(targets[next(targets.len())]);
            let change = match next(6) {
                0 => WorkbookChange::clear_cell(sheet_id, target),
                1 => WorkbookChange::set_cell_formula(
                    sheet_id,
                    target,
                    FormulaText::from_user_input("=A1+1").expect("formula"),
                ),
                2 => WorkbookChange::set_cell_value(other_id, address("B2"), number(99.0)),
                3 => WorkbookChange::set_defined_name(
                    DefinedName::new(
                        "Base",
                        DefinedNameScope::Workbook,
                        FormulaText::from_user_input("=Data!$A$3").expect("formula"),
                        false,
                    )
                    .expect("name"),
                ),
                4 => WorkbookChange::set_cell_value(
                    sheet_id,
                    target,
                    CellValue::Text(format!("h{}", next(9))),
                ),
                _ => WorkbookChange::set_cell_value(sheet_id, target, number(next(9) as f64)),
            };
            // An edit the draft rejects simply does not happen.
            let _ = draft.apply_changes(EditBatch::new([change]));
        }
        let after = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
            .unwrap_or_else(|error| panic!("sequence {sequence} must save: {error}"));
        saved += 1;
        let affected = affected(output.report().affected_formulas());
        for sheet in draft.workbook().sheets() {
            for cell in sheet.cells() {
                if !matches!(cell.content(), cellrune::CellContent::Formula(_)) {
                    continue;
                }
                let id = CalculationCellId::new(sheet.id(), cell.address());
                if before.cell(id) != after.cell(id) {
                    assert!(
                        affected.contains(&id),
                        "sequence {sequence}: {id:?} changed from {:?} to {:?} but is not affected",
                        before.cell(id),
                        after.cell(id)
                    );
                }
            }
        }
    }
    assert_eq!(saved, 120);
}

fn data_sheet(draft: &WorkbookDraft) -> SheetId {
    draft.workbook().sheets()[0].id()
}

fn address(text: &str) -> CellAddress {
    CellAddress::from_a1(text).expect("address")
}

fn number(value: f64) -> CellValue {
    CellValue::number(value).expect("finite value")
}

fn cells(sheet_id: SheetId, addresses: &[&str]) -> BTreeSet<CalculationCellId> {
    addresses
        .iter()
        .map(|text| CalculationCellId::new(sheet_id, address(text)))
        .collect()
}

fn affected(formulas: &[CalculationCellId]) -> BTreeSet<CalculationCellId> {
    formulas.iter().copied().collect()
}

fn archive_parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("archive");
    let mut parts = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).expect("entry bytes");
        parts.insert(file.name().expect("entry name").into_owned(), contents);
    }
    parts
}

fn part_text(bytes: &[u8], name: &str) -> String {
    String::from_utf8(archive_parts(bytes).remove(name).expect("part")).expect("UTF-8 part")
}

fn cell_xml<'a>(sheet: &'a str, address: &str) -> &'a str {
    let start = sheet
        .find(&format!(r#"<c r="{address}""#))
        .expect("cell element");
    let end = sheet[start..]
        .find("</c>")
        .map_or(sheet.len(), |end| start + end);
    &sheet[start..end]
}
