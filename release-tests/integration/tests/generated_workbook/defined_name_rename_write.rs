use cellrune::{
    CalculationOptions, DefinedNameScope, OpenOptions, RecalculationWriteOptions, WorkbookDraft,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// `Data` and `Calc`, a workbook `Limit` on `Data!A1`, and a `Data` scoped `Limit` on `Data!A2`.
/// `Calc` uses `Limit` in a cell, a conditional format, a data validation, and an x14 data
/// validation, and a chart reads `[0]!Limit`; `Data` uses its own `Limit` in a conditional format.
fn fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>"#;
    let root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="{MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Calc" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="Limit">Data!$A$1</definedName><definedName name="Limit" localSheetId="0">Data!$A$2</definedName></definedNames></workbook>"#
    );
    let data = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1"><v>5</v></c></row><row r="2"><c r="A2"><v>7</v></c></row></sheetData><conditionalFormatting sqref="B1"><cfRule type="expression" priority="1"><formula>B1&gt;Limit</formula></cfRule></conditionalFormatting></worksheet>"#
    );
    let calc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><sheetData><row r="1"><c r="A1"><f>Limit*2</f><v>10</v></c></row></sheetData><conditionalFormatting sqref="B1"><cfRule type="expression" priority="1"><formula>B1&gt;Limit+Data!Limit</formula></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="whole" operator="lessThan" sqref="C1"><formula1>Limit</formula1></dataValidation></dataValidations><extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}"><x14:dataValidations count="1"><x14:dataValidation type="whole" operator="lessThan"><x14:formula1><xm:f>Limit</xm:f></x14:formula1><xm:sqref>C2</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst></worksheet>"#
    );
    let chart = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:f>[0]!Limit</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    generated_package(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", &data),
        ("xl/worksheets/sheet2.xml", &calc),
        ("xl/charts/chart1.xml", chart),
    ])
}

#[test]
fn renaming_a_name_rewrites_its_uses_in_preserved_parts_by_scope() {
    let source = fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(cellrune::EditBatch::new([
            cellrune::WorkbookChange::rename_defined_name(
                DefinedNameScope::Workbook,
                "Limit",
                "Cap",
            ),
        ]))
        .expect("rename Limit");
    let calc_id = draft.workbook().sheet_by_name("Calc").expect("Calc").id();
    let validations = draft
        .workbook()
        .sheet_by_id(calc_id)
        .expect("Calc")
        .data_validations()
        .iter()
        .map(|validation| validation.formula1().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        validations,
        [Some("Cap".to_owned()), Some("Cap".to_owned())]
    );

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let parts = support::archive_parts(output.bytes());
    let text = |part: &str| String::from_utf8(parts[part].clone()).expect("UTF-8");
    let calc = text("xl/worksheets/sheet2.xml");
    for expected in [
        "<f>Cap*2</f>",
        // `Data!Limit` names `Data`'s own `Limit`, which keeps its name.
        "<formula>B1&gt;Cap+Data!Limit</formula>",
        "<formula1>Cap</formula1>",
        "<xm:f>Cap</xm:f>",
    ] {
        assert!(calc.contains(expected), "{expected}: {calc}");
    }
    // On `Data`, `Limit` is the sheet's own name.
    assert!(text("xl/worksheets/sheet1.xml").contains("<formula>B1&gt;Limit</formula>"));
    assert!(text("xl/charts/chart1.xml").contains("<c:f>[0]!Cap</c:f>"));
    let workbook = text("xl/workbook.xml");
    assert!(
        workbook.contains(r#"<definedName name="Cap">Data!$A$1</definedName>"#),
        "{workbook}"
    );
    assert!(
        workbook.contains(r#"<definedName name="Limit" localSheetId="0">Data!$A$2</definedName>"#)
    );

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_id(calc_id)
            .expect("Calc")
            .data_validations(),
        draft
            .workbook()
            .sheet_by_id(calc_id)
            .expect("Calc")
            .data_validations()
    );
}

#[test]
fn renaming_a_workbook_name_renames_the_preserved_pivot_cache_source_that_reads_it() {
    let source = support::replace_part_text(
        &crate::support::sheet_deletion_xlsx::generated_sheet_deletion_fixture(),
        "xl/pivotCache/pivotCacheDefinition2.xml",
        r#"<worksheetSource ref="D1:D2" sheet="S3"/>"#,
        r#"<worksheetSource name="Wb_S2"/>"#,
    );
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(cellrune::EditBatch::new([
            cellrune::WorkbookChange::rename_defined_name(
                DefinedNameScope::Workbook,
                "Wb_S2",
                "Data2",
            ),
        ]))
        .expect("rename Wb_S2");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        &draft,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(cellrune::RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("write");
    let parts = support::archive_parts(output.bytes());
    let cache =
        String::from_utf8(parts["xl/pivotCache/pivotCacheDefinition2.xml"].clone()).expect("UTF-8");
    assert!(
        cache.contains(r#"<worksheetSource name="Data2"/>"#),
        "{cache}"
    );
}
