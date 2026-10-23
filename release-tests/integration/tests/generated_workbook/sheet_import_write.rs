use cellrune::{
    CalculationOptions, CellAddress, CellRange, EditBatch, ImportedSheet, OpenOptions,
    RecalculationWriteOptions, ValidationError, WorkbookChange, WorkbookDraft, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use crate::support::generated_xlsx::{generated_format_workbook, generated_package};
use crate::support::sheet_deletion_xlsx::generated_sheet_deletion_fixture;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;
const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

/// A two-sheet workbook whose custom number format 164 is `format` and whose cell style 1 shows
/// it in bold.
fn workbook(names: [&str; 2], format: &str, first_sheet: &str) -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{}" sheetId="1" r:id="rId1"/><sheet name="{}" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
        names[0], names[1]
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="{format}"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="1" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyFont="1"/></cellXfs></styleSheet>"#
    );
    let other = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>10</v></c></row></sheetData></worksheet>"#;
    generated_package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/styles.xml", &styles),
        ("xl/worksheets/sheet1.xml", first_sheet),
        ("xl/worksheets/sheet2.xml", other),
    ])
}

/// `Data` holds a bold 0.000 number, a formula reading itself, a merged range, a hidden row, a
/// column width, and page margins, which an import leaves behind.
const DATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="2" max="2" width="18" customWidth="1"/></cols><sheetData><row r="1"><c r="A1" s="1"><v>1.25</v></c><c r="B1"><f>Data!A1*2</f><v>2.5</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>merged</t></is></c></row><row r="4" hidden="1"><c r="A4"><v>4</v></c></row></sheetData><mergeCells count="1"><mergeCell ref="A3:B3"/></mergeCells><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#;

fn open(bytes: &[u8]) -> WorkbookDraft {
    WorkbookDraft::from_document(
        &open_xlsx_document_bytes(bytes, OpenOptions::default()).expect("open"),
    )
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("address")
}

#[test]
fn an_imported_sheet_saves_with_its_formats_merges_and_hidden_rows() {
    let source = open(&workbook(["Data", "Other"], "0.000", DATA));
    // The target's own custom format 164 is a percentage, so the import's takes another ID.
    let mut target = open(&workbook(
        ["Sheet1", "Data"],
        "0.0%",
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1"><v>0.5</v></c></row></sheetData></worksheet>"#,
    ));
    let data = source.workbook().sheet_by_name("Data").expect("Data").id();
    let imported = ImportedSheet::from_draft(&source, data).expect("readable sheet");
    let receipt = target
        .apply_changes(EditBatch::new([WorkbookChange::import_sheet(imported, 2)]))
        .expect("import");
    let sheet_id = receipt.created_sheet_ids()[0];
    let calculation = calculate_workbook(target.workbook(), CalculationOptions::default());
    let output =
        write_xlsx_draft_bytes(&target, &calculation, RecalculationWriteOptions::default())
            .expect("write");
    let reopened = open(output.bytes());
    let sheet = reopened.workbook().sheet_by_id(sheet_id).expect("imported");
    assert_eq!(sheet.name().as_str(), "Data (2)");
    assert_eq!(
        sheet.merged_ranges(),
        [CellRange::new(address("A3"), address("B3")).expect("range")]
    );
    assert!(sheet.is_row_hidden(cellrune::Row::new(4).expect("row")));
    let a1 = sheet.cell(address("A1")).expect("A1");
    assert_eq!(a1.number_format().code(), Some("0.000"));
    assert!(
        reopened
            .cell_format(sheet_id, address("A1"))
            .expect("format")
            .font()
            .bold()
    );
    assert_eq!(
        reopened
            .presentation()
            .column_width(sheet_id, cellrune::Column::new(2).expect("column")),
        Some(18.0)
    );
    let cellrune::CellContent::Formula(formula) = sheet.cell(address("B1")).expect("B1").content()
    else {
        panic!("B1 holds a formula");
    };
    assert_eq!(
        formula.text().map(cellrune::FormulaText::as_str),
        Some("'Data (2)'!A1*2")
    );
    // The target's own cell keeps its percentage.
    let own = reopened.workbook().sheet_by_name("Sheet1").expect("Sheet1");
    assert_eq!(
        own.cell(address("A1")).expect("A1").number_format().code(),
        Some("0.0%")
    );
}

#[test]
fn content_the_target_cannot_hold_refuses_the_import() {
    let unsupported = |content: &str| ValidationError::SheetImportUnsupported {
        content: content.to_owned(),
    };
    let formats = open(&generated_format_workbook());
    let styled = formats.workbook().sheets()[0].id();
    assert_eq!(
        ImportedSheet::from_draft(&formats, styled),
        Err(unsupported("row or column styles"))
    );
    // S2 holds a drawing and comments; S1 a conditional format beside its other content.
    let deletion = open(&generated_sheet_deletion_fixture());
    let s2 = deletion.workbook().sheet_by_name("S2").expect("S2").id();
    assert_eq!(
        ImportedSheet::from_draft(&deletion, s2),
        Err(unsupported("comments"))
    );
    let other_reference = open(&workbook(
        ["Data", "Other"],
        "0.000",
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>Other!A1</f></c></row></sheetData><conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"><formula>TRUE</formula></cfRule></conditionalFormatting></worksheet>"#,
    ));
    let data = other_reference.workbook().sheets()[0].id();
    assert_eq!(
        ImportedSheet::from_draft(&other_reference, data),
        Err(unsupported("conditionalFormatting"))
    );
}
