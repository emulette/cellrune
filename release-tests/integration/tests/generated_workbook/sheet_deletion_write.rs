use std::collections::BTreeMap;

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellValue,
    DefinedName, DefinedNameScope, FormulaText, OpenOptions, RecalculationWriteOptions,
    RecalculationWritePolicy, SavedResult, SheetId, SheetName, WorkbookDraft, WriteOptions,
    XlsxWriteError, XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_bytes, write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support as support;
use crate::support::sheet_deletion_xlsx::generated_sheet_deletion_fixture;

const S1_PART: &str = "xl/worksheets/sheet1.xml";
const WORKBOOK_PART: &str = "xl/workbook.xml";

fn delete(source: &[u8], sheets: &[&str]) -> WorkbookDraft {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    for name in sheets {
        let sheet_id = draft
            .workbook()
            .sheet_by_name(name)
            .expect("deleted sheet")
            .id();
        draft.delete_sheet(sheet_id).expect("delete sheet");
    }
    draft
}

/// `'S3'!LocalS3`, a sheet-qualified local name, has no calculated value, so its result is
/// invalidated rather than required.
fn invalidating() -> RecalculationWriteOptions {
    RecalculationWriteOptions::default()
        .with_policy(RecalculationWritePolicy::InvalidateUnavailable)
}

fn write(draft: &WorkbookDraft) -> Result<Vec<u8>, XlsxWriteError> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, invalidating())
        .map(|output| output.bytes().to_vec())
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

fn result(document: &cellrune::XlsxDocument, sheet: &str, address: &str) -> (String, SavedResult) {
    let formula = support::formula_at(document, sheet, address);
    (
        formula.text().expect("formula text").as_str().to_owned(),
        formula.saved_result().clone(),
    )
}

fn number(value: f64) -> SavedResult {
    SavedResult::Present(CellValue::number(value).expect("number"))
}

#[test]
fn deleting_a_sheet_removes_the_parts_only_it_reached_and_rewrites_the_rest() {
    let source = generated_sheet_deletion_fixture();
    let output = write(&delete(&source, &["S2"])).expect("output without S2");
    let source_parts = support::archive_parts(&source);
    let parts = support::archive_parts(&output);

    let removed = source_parts
        .keys()
        .filter(|part| !parts.contains_key(*part))
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        removed,
        [
            "xl/calcChain.xml",
            "xl/charts/chart2.xml",
            "xl/comments1.xml",
            "xl/drawings/_rels/drawing2.xml.rels",
            "xl/drawings/drawing2.xml",
            "xl/drawings/vmlDrawing1.vml",
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            "xl/pivotCache/pivotCacheDefinition1.xml",
            "xl/pivotCache/pivotCacheRecords1.xml",
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            "xl/pivotTables/pivotTable1.xml",
            "xl/tables/table1.xml",
            "xl/worksheets/_rels/sheet2.xml.rels",
            "xl/worksheets/sheet2.xml",
        ]
    );
    // A part another sheet still reaches stays, and so does `docProps/app.xml`, as for renames.
    for part in [
        "docProps/app.xml",
        "xl/media/image1.png",
        "xl/drawings/drawing1.xml",
        "xl/pivotTables/pivotTable2.xml",
        "xl/pivotCache/pivotCacheDefinition2.xml",
    ] {
        assert_eq!(source_parts[part], parts[part], "{part} is untouched");
    }

    let content_types = text(&parts, "[Content_Types].xml");
    for removed in &removed {
        assert!(
            !content_types.contains(&format!("\"/{removed}\"")),
            "{removed}: {content_types}"
        );
    }
    assert!(content_types.contains(r#"PartName="/xl/pivotCache/pivotCacheDefinition2.xml""#));
    let relationships = text(&parts, "xl/_rels/workbook.xml.rels");
    for (id, kept) in [
        ("rId1", true),
        ("rId2", false),
        ("rId3", true),
        ("rId4", true),
        ("rId5", false),
        ("rId6", true),
        ("rId7", false),
    ] {
        assert_eq!(
            relationships.contains(&format!(r#"Id="{id}""#)),
            kept,
            "{id}: {relationships}"
        );
    }

    let workbook = text(&parts, WORKBOOK_PART);
    assert!(!workbook.contains(r#"name="S2""#), "{workbook}");
    for expected in [
        r#"<workbookView xWindow="0" yWindow="0" firstSheet="1" activeTab="1"/>"#,
        r#"<pivotCaches><pivotCache cacheId="2" r:id="rId6"/></pivotCaches>"#,
        r#"<definedName name="LocalS1" localSheetId="0">#REF!</definedName>"#,
        r#"<definedName name="LocalS3" localSheetId="1">'S3'!$A$1</definedName>"#,
        r#"<definedName name="LocalS4" localSheetId="2">'S4'!$A$1</definedName>"#,
        r#"<definedName name="Wb_S2">#REF!</definedName>"#,
        r#"<definedName name="Wb_S2S4">'S3:S4'!$A$1</definedName>"#,
        r#"<definedName name="Wb_Used">#REF!</definedName>"#,
    ] {
        assert!(workbook.contains(expected), "{expected}: {workbook}");
    }
    for removed in ["LocalS2", "Unused2"] {
        assert!(!workbook.contains(removed), "{removed}: {workbook}");
    }

    let s1 = text(&parts, S1_PART);
    assert_eq!(
        after_cells(&s1),
        after_cells(&text(&source_parts, S1_PART))
            .replace(
                "<formula>'S2'!$A$1&gt;0</formula>",
                "<formula>#REF!&gt;0</formula>"
            )
            .replace(r#"val="'S2'!$A$1""#, r##"val="#REF!""##)
            .replace(
                "<formula1>'S2'!$A$1:$A$3</formula1>",
                "<formula1>#REF!</formula1>"
            )
            .replace("<xm:f>'S2'!$A$2&gt;1</xm:f>", "<xm:f>#REF!&gt;1</xm:f>")
            .replace("<xm:f>'S2'!$A$1:$A$3</xm:f>", "<xm:f>#REF!</xm:f>")
            .replace("<xm:f>'S2'!A1:A3</xm:f>", "")
    );
    assert_eq!(
        text(&parts, "xl/charts/chart1.xml"),
        text(&source_parts, "xl/charts/chart1.xml")
            .replace("<c:f>'S2'!$A$1:$A$3</c:f>", "<c:f>#REF!</c:f>")
    );
    assert!(
        text(&parts, "xl/tables/table2.xml")
            .contains("<calculatedColumnFormula>#REF!+1</calculatedColumnFormula>")
    );

    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    let names = reopened
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["S1", "S3", "S4"]);
    assert!(reopened.workbook().table("Things").is_none());
    for (address, formula, saved) in [
        (
            "B1",
            "#REF!",
            SavedResult::Present(CellValue::Error(cellrune::ExcelError::Reference)),
        ),
        ("B2", "SUM('S1:S3'!A1)", number(4.0)),
        ("B3", "SUM('S3:S4'!A1)", number(7.0)),
        ("B4", "SUM('S3'!A1)", number(3.0)),
        (
            "B6",
            "SUM(#REF!)",
            SavedResult::Present(CellValue::Error(cellrune::ExcelError::Reference)),
        ),
        (
            "B8",
            "#REF!",
            SavedResult::Present(CellValue::Error(cellrune::ExcelError::Reference)),
        ),
        (
            "B13",
            "#REF!+'S3'!A1",
            SavedResult::Present(CellValue::Error(cellrune::ExcelError::Reference)),
        ),
        ("B15", "SHEETS()", number(3.0)),
    ] {
        assert_eq!(
            result(&reopened, "S1", address),
            (formula.to_owned(), saved),
            "S1!{address}"
        );
    }
}

/// Deleting sheets and recalculating gives the results that opening the saved file and
/// calculating it gives.
#[test]
fn delete_then_full_recalculation_equals_the_saved_file_calculated_fresh() {
    let source = generated_sheet_deletion_fixture();
    for deleted in [&["S1"][..], &["S2"], &["S4"], &["S1", "S4"], &["S2", "S1"]] {
        let draft = delete(&source, deleted);
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output = write_xlsx_draft_bytes(&draft, &calculation, invalidating())
            .unwrap_or_else(|error| panic!("{deleted:?}: {error}"));
        let reopened =
            open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
        let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
        let results = |calculation: &cellrune::CalculationSnapshot| {
            calculation
                .cells()
                .map(|(cell, result)| (cell, result.clone()))
                .collect::<Vec<(CalculationCellId, CalculationCellResult)>>()
        };
        assert_eq!(results(&calculation), results(&fresh), "{deleted:?}");
        assert!(!results(&fresh).is_empty(), "{deleted:?}");
    }
}

#[test]
fn deleting_a_sheet_a_preserved_pivot_cache_or_chart_reads_is_refused_with_the_part() {
    let source = generated_sheet_deletion_fixture();
    // The cache whose source is `S3` serves the pivot table on `S1`, which stays.
    let error = write(&delete(&source, &["S3"])).expect_err("S3 is a pivot cache source");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    assert_eq!(
        error.detail(),
        Some(
            "a preserved pivot cache reads a deleted sheet or a table on one: xl/pivotCache/pivotCacheDefinition2.xml"
        )
    );
    assert_eq!(
        error.source_id().map(|source| source.as_str()),
        Some("xl/pivotCache/pivotCacheDefinition2.xml")
    );
    // Once that pivot table goes too, so does the cache, and nothing is left to refuse.
    write(&delete(&source, &["S1", "S3"])).expect("the cache leaves with its pivot table");

    let pivot_chart = support::replace_part_text(
        &source,
        "xl/charts/chart1.xml",
        "<c:chart>",
        "<c:pivotSource><c:name>[Book1.xlsx]S2!PivotTable1</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart>",
    );
    let error = write(&delete(&pivot_chart, &["S2"])).expect_err("a pivot chart of S2 stays");
    assert_eq!(
        error.detail(),
        Some(
            "a preserved pivot chart reads a pivot table on a deleted sheet: xl/charts/chart1.xml"
        )
    );
    write(&delete(&pivot_chart, &["S1"])).expect("the pivot chart leaves with S1");

    let slicer = support::rewrite_archive(
        &support::replace_part_text(
            &source,
            "xl/_rels/workbook.xml.rels",
            "</Relationships>",
            r#"<Relationship Id="rId8" Type="http://schemas.microsoft.com/office/2007/relationships/slicerCache" Target="slicerCaches/slicerCache1.xml"/></Relationships>"#,
        ),
        &BTreeMap::new(),
        &[(
            "xl/slicerCaches/slicerCache1.xml",
            br#"<slicerCacheDefinition xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" name="Slicer_k" sourceName="k"/>"#,
        )],
    );
    let error = write(&delete(&slicer, &["S2"])).expect_err("S2 holds a table and a pivot table");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    assert_eq!(
        error.detail(),
        Some(
            "a preserved slicer or timeline cache may select a pivot table or table of a deleted sheet: xl/slicerCaches/slicerCache1.xml"
        )
    );
    write(&delete(&slicer, &["S4"])).expect("S4 holds no table or pivot table");
}

#[test]
fn deleting_a_sheet_skips_relationship_targets_that_cannot_be_parts() {
    let hyperlink = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    // Producers write cell and mail links in drawing, chart, and VML relationships without
    // `TargetMode="External"`, empty targets, and repeated IDs; none of them names a part.
    let unusual = format!(
        r##"<Relationship Id="rId3" Type="{hyperlink}" Target="#S3!A1"/><Relationship Id="rId4" Type="{hyperlink}" Target="mailto:someone@example.com"/><Relationship Id="rId5" Type="{hyperlink}" Target=""/><Relationship Id="rId5" Type="{hyperlink}" Target="#'S4'!A1"/></Relationships>"##
    );
    let source = support::replace_part_text(
        &generated_sheet_deletion_fixture(),
        "xl/drawings/_rels/drawing1.xml.rels",
        "</Relationships>",
        &unusual,
    );
    let output = write(&delete(&source, &["S2"])).expect("deletion despite unusual targets");
    let parts = support::archive_parts(&output);
    assert!(!parts.contains_key("xl/worksheets/sheet2.xml"));
    assert_eq!(
        support::archive_parts(&source)["xl/drawings/_rels/drawing1.xml.rels"],
        parts["xl/drawings/_rels/drawing1.xml.rels"]
    );

    let unreadable = support::replace_part_text(
        &generated_sheet_deletion_fixture(),
        "xl/drawings/_rels/drawing1.xml.rels",
        "</Relationships>",
        "<Relationship></Relationships>",
    );
    let error = write(&delete(&unreadable, &["S2"])).expect_err("unreadable relationships");
    assert_eq!(error.code(), XlsxWriteErrorCode::InvalidPackagePlan);
    assert_eq!(
        error.source_id().map(|source| source.as_str()),
        Some("xl/drawings/_rels/drawing1.xml.rels")
    );
}

#[test]
fn deleting_a_sheet_with_a_dangling_relationship_removes_only_parts_the_package_holds() {
    let source = support::replace_part_text(
        &generated_sheet_deletion_fixture(),
        "xl/worksheets/_rels/sheet2.xml.rels",
        "</Relationships>",
        r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings" Target="../printerSettings/printerSettings1.bin"/></Relationships>"#,
    );
    let output = write(&delete(&source, &["S2"])).expect("deletion with a dangling relationship");
    let parts = support::archive_parts(&output);
    assert!(!parts.contains_key("xl/worksheets/sheet2.xml"));
    assert!(!parts.contains_key("xl/worksheets/_rels/sheet2.xml.rels"));
}

#[test]
fn edit_only_write_clears_the_results_a_deleted_sheet_changes() {
    let source = generated_sheet_deletion_fixture();
    let draft = delete(&source, &["S2"]);
    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("edits");
    assert!(output.report().requests_host_recalculation());
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open");
    // `S2` sat inside the 3-D range and the sheet list shrank, so these lose their results even
    // though their text is unchanged.
    for address in ["B2", "B15", "B1", "B5"] {
        assert_eq!(
            result(&reopened, "S1", address).1,
            SavedResult::Missing,
            "S1!{address}"
        );
    }
    assert_eq!(result(&reopened, "S4", "B2").1, number(1.0));
}

#[test]
fn canonical_draft_deletes_a_sheet_and_keeps_the_ids_of_the_rest() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();
    let middle = draft
        .add_sheet(SheetName::new("Middle").expect("sheet name"))
        .expect("middle");
    let last = draft
        .add_sheet(SheetName::new("Last").expect("sheet name"))
        .expect("last");
    let a1 = CellAddress::from_a1("A1").expect("A1");
    draft
        .set_cell_value(last, a1, CellValue::number(5.0).expect("number"))
        .expect("value");
    draft
        .set_cell_formula(
            first,
            a1,
            FormulaText::from_xlsx("Middle!A1+Last!A1").expect("formula"),
        )
        .expect("formula");
    draft
        .set_defined_name(
            DefinedName::new(
                "Local",
                DefinedNameScope::Sheet(last),
                FormulaText::from_xlsx("Last!$A$1").expect("name formula"),
                false,
            )
            .expect("name"),
        )
        .expect("scoped name");
    draft.delete_sheet(middle).expect("delete middle");

    let output = write(&draft).expect("canonical output");
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    let ids = reopened
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.id())
        .collect::<Vec<_>>();
    assert_eq!(ids, [first, last]);
    assert_eq!(
        reopened.workbook().defined_names()[0].scope(),
        DefinedNameScope::Sheet(last)
    );
    assert_eq!(
        result(&reopened, "Sheet1", "A1").0,
        "#REF!+Last!A1".to_owned()
    );
    assert_eq!(
        draft
            .add_sheet(SheetName::new("Added").expect("sheet name"))
            .expect("added"),
        SheetId::new(4).expect("next unused ID")
    );
}
