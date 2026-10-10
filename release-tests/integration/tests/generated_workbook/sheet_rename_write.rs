use std::collections::BTreeMap;

use cellrune::{
    CalculationOptions, OpenOptions, RecalculationWriteOptions, SheetName, WorkbookDraft,
    XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_sheet_reference_fixture;

const DATA_PART: &str = "xl/worksheets/sheet1.xml";
const SUMMARY_PART: &str = "xl/worksheets/sheet2.xml";
const CHART_PART: &str = "xl/charts/chart1.xml";
const PIVOT_CACHE_PART: &str = "xl/pivotCache/pivotCacheDefinition1.xml";

#[test]
fn sheet_rename_rewrites_references_outside_cell_formulas() {
    let source = generated_sheet_reference_fixture();
    let output = write_renamed(&source, &[("Data", "New Data")]).expect("renamed output");

    let source_parts = support::archive_parts(&source);
    let output_parts = support::archive_parts(&output);
    for part in [
        DATA_PART,
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/worksheets/_rels/sheet2.xml.rels",
    ] {
        assert_eq!(
            source_parts[part], output_parts[part],
            "{part} is untouched"
        );
    }

    // The cell formula is rewritten by the draft; everything after the cells only by the
    // reference patch, which leaves every byte but the renamed qualifiers in place. The external
    // hyperlink's location points into the linked file and keeps its text.
    let summary = text(&output_parts, SUMMARY_PART);
    assert!(
        summary.contains("<f>SUM('New Data'!A1:A3)</f>"),
        "{summary}"
    );
    assert_eq!(
        after_cells(&summary),
        after_cells(&text(&source_parts, SUMMARY_PART))
            .replace(
                "<formula>Data!$A$1&gt;0</formula>",
                "<formula>'New Data'!$A$1&gt;0</formula>",
            )
            .replace(
                r#"<cfvo type="formula" val="Data!$A$1"/>"#,
                r#"<cfvo type="formula" val="&apos;New Data&apos;!$A$1"/>"#,
            )
            .replace(
                "<formula1>Data!$A$1:$A$3</formula1>",
                "<formula1>'New Data'!$A$1:$A$3</formula1>",
            )
            .replace(
                r#"<hyperlink ref="C1" location="Data!A1" display="Data"/>"#,
                r#"<hyperlink ref="C1" location="&apos;New Data&apos;!A1" display="Data"/>"#,
            )
            .replace(
                "<xm:f>Data!$A$2&gt;1</xm:f>",
                "<xm:f>'New Data'!$A$2&gt;1</xm:f>",
            )
            .replace(
                "<xm:f>Data!$B$1:$B$3</xm:f>",
                "<xm:f>'New Data'!$B$1:$B$3</xm:f>",
            )
            .replace("<xm:f>Data!A1:C1</xm:f>", "<xm:f>'New Data'!A1:C1</xm:f>")
    );
    assert!(summary.contains(r#"<hyperlink ref="C2" r:id="rId2" location="Data!A1"/>"#));
    assert_eq!(
        text(&output_parts, CHART_PART),
        text(&source_parts, CHART_PART).replace(
            "<c:f>Data!$A$1:$A$3</c:f>",
            "<c:f>'New Data'!$A$1:$A$3</c:f>"
        )
    );
    assert_eq!(
        text(&output_parts, PIVOT_CACHE_PART),
        text(&source_parts, PIVOT_CACHE_PART).replace(r#"sheet="Data""#, r#"sheet="New Data""#)
    );

    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    assert!(reopened.workbook().sheet_by_name("New Data").is_some());
    assert!(reopened.workbook().sheet_by_name("Data").is_none());
}

#[test]
fn sheet_name_swap_rewrites_each_reference_to_its_own_sheet() {
    let source = generated_sheet_reference_fixture();
    let output = write_renamed(
        &source,
        &[("Data", "Swap"), ("Summary", "Data"), ("Swap", "Summary")],
    )
    .expect("swapped output");

    let output_parts = support::archive_parts(&output);
    let data = text(&output_parts, DATA_PART);
    assert!(data.contains(r#"location="Data!A1""#), "{data}");
    let summary = text(&output_parts, SUMMARY_PART);
    assert!(
        summary.contains("<formula>Summary!$A$1&gt;0</formula>"),
        "{summary}"
    );
    assert!(summary.contains("<xm:f>Summary!A1:C1</xm:f>"), "{summary}");
    let chart = text(&output_parts, CHART_PART);
    assert!(chart.contains("<c:f>Data!$A$1</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>Summary!$A$1:$A$3</c:f>"), "{chart}");
    assert!(text(&output_parts, PIVOT_CACHE_PART).contains(r#"sheet="Summary""#));
}

#[test]
fn sheet_rename_rejects_chart_references_it_cannot_rewrite() {
    let source = generated_sheet_reference_fixture();
    for (from, to) in [
        ("<c:f>Data!$A$1:$A$3</c:f>", "<c:f>Data!$A$1:$A$3+</c:f>"),
        (
            "<c:chart>",
            "<c:pivotSource><c:name>[Book1.xlsx]Data!PivotTable1</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart>",
        ),
    ] {
        let mutated = support::replace_part_text(&source, CHART_PART, from, to);
        let Err(error) = write_renamed(&mutated, &[("Data", "New Data")]) else {
            panic!("an unrewritable chart reference to the renamed sheet must fail closed: {to}");
        };
        assert_eq!(error, XlsxWriteErrorCode::UnsupportedPreservation, "{to}");

        let output = write_renamed(&mutated, &[("Summary", "Report")])
            .expect("a rename of another sheet leaves the chart reference alone");
        assert!(text(&support::archive_parts(&output), CHART_PART).contains(to));
    }
}

fn write_renamed(source: &[u8], renames: &[(&str, &str)]) -> Result<Vec<u8>, XlsxWriteErrorCode> {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    for (from, to) in renames {
        let sheet = draft
            .workbook()
            .sheet_by_name(from)
            .expect("renamed sheet")
            .id();
        draft
            .rename_sheet(sheet, SheetName::new(*to).expect("sheet name"))
            .expect("rename");
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .map(|output| output.bytes().to_vec())
        .map_err(|error| error.code())
}

fn text(parts: &BTreeMap<String, Vec<u8>>, part: &str) -> String {
    String::from_utf8(parts[part].clone()).expect("UTF-8 part")
}

fn after_cells(worksheet: &str) -> &str {
    worksheet
        .split_once("</sheetData>")
        .expect("worksheet cells")
        .1
}
