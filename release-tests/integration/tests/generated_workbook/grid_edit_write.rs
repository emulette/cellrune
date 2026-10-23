use std::collections::BTreeMap;

use cellrune::{
    CalculationOptions, CellAddress, CellContent, CellValue, Column, EditBatch, OpenOptions,
    RecalculationWriteOptions, RecalculationWritePolicy, Row, SavedResult, SheetId, WorkbookChange,
    WorkbookDraft, WriteOptions, XlsxDocument, XlsxWriteErrorCode, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes, write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support as support;
use crate::support::grid_edit_xlsx::{
    generated_grid_edit_fixture, generated_grid_edit_fixture_with,
};

const DATA: &str = "xl/worksheets/sheet1.xml";
const REPORT: &str = "xl/worksheets/sheet2.xml";

fn sheet(value: u32) -> SheetId {
    SheetId::new(value).expect("sheet ID")
}

fn row(value: u32) -> Row {
    Row::new(value).expect("row")
}

fn column(value: u32) -> Column {
    Column::new(value).expect("column")
}

fn open(bytes: &[u8]) -> XlsxDocument {
    open_xlsx_document_bytes(bytes, OpenOptions::default()).expect("open")
}

fn edited(source: &[u8], changes: Vec<WorkbookChange>) -> WorkbookDraft {
    let mut draft = WorkbookDraft::from_document(&open(source));
    draft
        .apply_changes(EditBatch::new(changes))
        .expect("structural edit");
    draft
}

/// Writes the calculated draft, reopens the output, and checks that calculating it afresh gives
/// every result the draft's own calculation gave.
fn write_and_reopen(draft: &WorkbookDraft) -> (Vec<u8>, XlsxDocument) {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        draft,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("write structural edit")
    .bytes()
    .to_vec();
    let reopened = open(&output);
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    assert_eq!(
        fresh.cells().collect::<Vec<_>>(),
        calculation.cells().collect::<Vec<_>>()
    );
    (output, reopened)
}

fn formula(document: &XlsxDocument, sheet: &str, address: &str) -> String {
    support::formula_at(document, sheet, address)
        .text()
        .expect("formula text")
        .as_str()
        .to_owned()
}

fn literal(document: &XlsxDocument, sheet: &str, address: &str) -> Option<CellValue> {
    match document
        .workbook()
        .sheet_by_name(sheet)
        .expect("sheet")
        .cell(CellAddress::from_a1(address).expect("address"))
        .map(cellrune::Cell::content)
    {
        Some(CellContent::Literal(value)) => Some(value.clone()),
        _ => None,
    }
}

fn parts(bytes: &[u8]) -> BTreeMap<String, String> {
    support::archive_parts(bytes)
        .into_iter()
        .map(|(name, bytes)| (name, String::from_utf8_lossy(&bytes).into_owned()))
        .collect()
}

fn assert_contains(text: &str, expected: &[&str]) {
    for expected in expected {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

fn names(document: &XlsxDocument) -> Vec<(String, String)> {
    document
        .workbook()
        .defined_names()
        .iter()
        .map(|name| (name.name().to_owned(), name.formula().as_str().to_owned()))
        .collect()
}

#[test]
fn inserted_rows_move_every_coordinate_of_the_saved_package() {
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![WorkbookChange::insert_rows(sheet(1), row(3), 2).expect("insertion")],
    );
    let (output, reopened) = write_and_reopen(&draft);

    for (cell, expected) in [
        ("D1", "SUM(B2:B7)"),
        ("D2", "B5*2"),
        ("D5", "B2+1"),
        ("D6", "B5+1"),
        ("D7", "B6+1"),
        ("A9", "SUM(Things[val])"),
        ("B9", "ROW()"),
    ] {
        assert_eq!(formula(&reopened, "Data", cell), expected, "Data!{cell}");
    }
    assert_eq!(
        literal(&reopened, "Data", "A7"),
        Some(CellValue::Text("d".to_owned()))
    );
    for (cell, expected) in [
        ("A1", "SUM(Data!B2:B7)"),
        ("A2", "Data!B5"),
        ("A3", "SUM(Data!B:B)"),
        ("A4", "Data!D1+Total"),
    ] {
        assert_eq!(
            formula(&reopened, "Report", cell),
            expected,
            "Report!{cell}"
        );
    }
    assert_eq!(
        names(&reopened),
        [
            (
                "_xlnm._FilterDatabase".to_owned(),
                "Data!$L$1:$M$6".to_owned()
            ),
            ("_xlnm.Print_Area".to_owned(), "Data!$A$1:$H$12".to_owned()),
            ("Total".to_owned(), "Data!$B$2:$B$7".to_owned()),
        ]
    );
    let data = reopened.workbook().sheet_by_name("Data").expect("Data");
    assert_eq!(data.tables()[0].range().end().to_string(), "B7");
    assert_eq!(data.merged_ranges()[0].end().to_string(), "H5");
    assert_eq!(data.comments()[0].address().to_string(), "B6");
    assert_eq!(data.hyperlinks()[0].range().start().to_string(), "G7");
    for (line, height) in [(2, Some(25.0)), (3, Some(25.0)), (4, Some(25.0)), (5, None)] {
        assert_eq!(
            reopened.presentation().row_height(sheet(1), row(line)),
            height,
            "{line}"
        );
    }

    let parts = parts(&output);
    assert!(!parts.contains_key("xl/calcChain.xml"));
    assert_contains(
        &parts[DATA],
        &[
            r#"<row r="3" ht="25" customHeight="1"><c r="B3" s="2"/></row>"#,
            r#"<row r="5" s="1" customFormat="1">"#,
            r#"<selection pane="bottomLeft" activeCell="B5" sqref="B5"/>"#,
            r#"<autoFilter ref="L1:M6"><filterColumn colId="1">"#,
            r#"<mergeCell ref="G2:H5"/>"#,
            r#"<conditionalFormatting sqref="B2:B7"><cfRule type="expression" dxfId="0" priority="1"><formula>B2&gt;2</formula>"#,
            r#"<dataValidation type="list" allowBlank="1" sqref="A2:A7"><formula1>$L$2:$L$6</formula1>"#,
            r#"<hyperlink ref="G7" location="'Data'!A1" display="top"/>"#,
            r#"<brk id="6" max="16383" man="1"/>"#,
            "<xm:f>$B$7</xm:f>",
            "<xm:sqref>B2:B7</xm:sqref>",
            "<xm:f>Report!$C$1:$C$2</xm:f></x14:formula1><xm:sqref>C2:C5</xm:sqref>",
            "<xm:f>Data!B2:B7</xm:f><xm:sqref>N2</xm:sqref>",
        ],
    );
    assert_contains(
        &parts["xl/drawings/drawing1.xml"],
        &[
            "<xdr:from><xdr:col>9</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row>",
            "<xdr:to><xdr:col>14</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>11</xdr:row>",
        ],
    );
    assert_contains(
        &parts["xl/drawings/vmlDrawing1.vml"],
        &[
            "<x:Anchor>2, 15, 4, 10, 4, 15, 8, 4</x:Anchor>",
            "<x:Row>5</x:Row><x:Column>1</x:Column>",
        ],
    );
    assert_contains(&parts["xl/comments1.xml"], &[r#"<comment ref="B6""#]);
    assert_contains(
        &parts["xl/threadedComments/threadedComment1.xml"],
        &[r#"<threadedComment ref="B6""#],
    );
    assert_contains(
        &parts["xl/charts/chart1.xml"],
        &[
            "<c:f>Data!$B$1</c:f>",
            "<c:f>Data!$A$2:$A$7</c:f>",
            "<c:f>Data!$B$2:$B$7</c:f>",
        ],
    );
    assert_contains(
        &parts[REPORT],
        &[
            "<formula>Data!$B$5&gt;0</formula>",
            "<formula1>Data!$A$2:$A$7</formula1>",
            r#"location="Data!B3""#,
        ],
    );
    assert_contains(&parts["xl/tables/table1.xml"], &[r#"ref="A1:B7""#]);
}

#[test]
fn deleted_rows_turn_saved_references_into_ref_errors() {
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![WorkbookChange::delete_rows(sheet(1), row(3), row(3)).expect("deletion")],
    );
    let (output, reopened) = write_and_reopen(&draft);
    for (cell, expected) in [
        ("D1", "SUM(B2:B4)"),
        ("D2", "#REF!*2"),
        ("D3", "#REF!+1"),
        ("D4", "B3+1"),
    ] {
        assert_eq!(formula(&reopened, "Data", cell), expected, "Data!{cell}");
    }
    assert_eq!(formula(&reopened, "Report", "A2"), "Data!#REF!");
    assert_eq!(formula(&reopened, "Report", "A1"), "SUM(Data!B2:B4)");
    let data = reopened.workbook().sheet_by_name("Data").expect("Data");
    assert_eq!(data.tables()[0].range().end().to_string(), "B4");
    assert_eq!(data.comments()[0].address().to_string(), "B3");
    let parts = parts(&output);
    assert_contains(
        &parts[DATA],
        &[
            r#"<conditionalFormatting sqref="B2:B4">"#,
            r#"<formula1>$L$2:$L$3</formula1>"#,
            r#"<autoFilter ref="L1:M3">"#,
            r#"<mergeCell ref="G2:H2"/>"#,
            "<xm:f>$B$4</xm:f>",
            "<xm:f>Data!B2:B4</xm:f>",
        ],
    );
    assert_contains(&parts["xl/drawings/vmlDrawing1.vml"], &["<x:Row>2</x:Row>"]);
    assert_contains(&parts[REPORT], &["<formula>Data!#REF!&gt;0</formula>"]);
}

#[test]
fn inserted_columns_grow_the_table_with_a_named_column() {
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![WorkbookChange::insert_columns(sheet(1), column(2), 1).expect("insertion")],
    );
    let (output, reopened) = write_and_reopen(&draft);
    assert_eq!(formula(&reopened, "Data", "E1"), "SUM(C2:C5)");
    assert_eq!(formula(&reopened, "Report", "A1"), "SUM(Data!C2:C5)");
    assert_eq!(formula(&reopened, "Report", "A3"), "SUM(Data!C:C)");
    let data = reopened.workbook().sheet_by_name("Data").expect("Data");
    let table = &data.tables()[0];
    assert_eq!(table.range().end().to_string(), "C5");
    assert_eq!(
        table
            .columns()
            .iter()
            .map(cellrune::TableColumn::name)
            .collect::<Vec<_>>(),
        ["key", "Column1", "val"]
    );
    assert_eq!(
        literal(&reopened, "Data", "B1"),
        Some(CellValue::Text("Column1".to_owned()))
    );
    for (line, width) in [(1, Some(15.0)), (2, Some(15.0)), (3, None), (4, Some(12.0))] {
        assert_eq!(
            reopened.presentation().column_width(sheet(1), column(line)),
            width,
            "{line}"
        );
    }
    let parts = parts(&output);
    assert_contains(
        &parts["xl/charts/chart1.xml"],
        &["<c:f>Data!$C$2:$C$5</c:f>", "<c:f>Data!$A$2:$A$5</c:f>"],
    );
    assert_contains(
        &parts["xl/drawings/drawing1.xml"],
        &["<xdr:from><xdr:col>10</xdr:col>"],
    );
    assert_contains(&parts[DATA], &[r#"<autoFilter ref="M1:N4">"#]);
}

#[test]
fn successive_edits_replay_in_order_on_every_part() {
    let source = generated_grid_edit_fixture();
    let mut draft = edited(
        &source,
        vec![
            WorkbookChange::insert_rows(sheet(1), row(3), 1).expect("insertion"),
            WorkbookChange::delete_columns(sheet(1), column(6), column(6)).expect("deletion"),
        ],
    );
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_value(
                sheet(1),
                CellAddress::from_a1("B4").expect("B4"),
                CellValue::number(50.0).expect("number"),
            ),
            WorkbookChange::insert_rows(sheet(1), row(1), 1).expect("insertion"),
        ]))
        .expect("later edits");
    let (output, reopened) = write_and_reopen(&draft);
    assert_eq!(formula(&reopened, "Report", "A2"), "Data!B5");
    assert_eq!(
        literal(&reopened, "Data", "B5"),
        Some(CellValue::number(50.0).expect("number"))
    );
    assert_contains(&parts(&output)[DATA], &[r#"<mergeCell ref="F3:G5"/>"#]);
}

#[test]
fn edit_only_writes_clear_the_results_the_structural_edit_can_change() {
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![WorkbookChange::insert_rows(sheet(1), row(3), 1).expect("insertion")],
    );
    let output =
        write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("edit-only write");
    assert!(output.report().requests_host_recalculation());
    let reopened = open(output.bytes());
    for (sheet, cell) in [("Data", "D1"), ("Report", "A1"), ("Report", "A3")] {
        assert_eq!(
            support::formula_at(&reopened, sheet, cell).saved_result(),
            &SavedResult::Missing,
            "{sheet}!{cell}"
        );
    }
    assert_eq!(formula(&reopened, "Report", "A2"), "Data!B4");
}

#[test]
fn parts_holding_coordinates_no_pass_rewrites_are_refused() {
    let pivot_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/pivotTable1.xml"/></Relationships>"#;
    let pivot_table = r#"<?xml version="1.0" encoding="UTF-8"?>
<pivotTableDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="PivotTable1" cacheId="1" dataCaption="Values"><location ref="J20:K22" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/></pivotTableDefinition>"#;
    let cases: [(Vec<(&str, &str)>, &str); 3] = [
        (
            vec![
                ("xl/worksheets/_rels/sheet1.xml.rels", pivot_relationships),
                ("xl/pivotTables/pivotTable1.xml", pivot_table),
            ],
            "xl/pivotTables/pivotTable1.xml",
        ),
        (
            vec![(
                "Data",
                r#"<controls><control shapeId="1026" r:id="rId9"/></controls>"#,
            )],
            "controls",
        ),
        (
            vec![(
                "Data",
                r#"<extLst><ext uri="{0000-UNKNOWN}"><unknown/></ext></extLst>"#,
            )],
            "{0000-UNKNOWN}",
        ),
    ];
    for (extra, detail) in cases {
        let source = generated_grid_edit_fixture_with(&extra);
        let draft = edited(
            &source,
            vec![WorkbookChange::insert_rows(sheet(1), row(3), 1).expect("insertion")],
        );
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let error = write_xlsx_draft_bytes(
            &draft,
            &calculation,
            RecalculationWriteOptions::default()
                .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
        )
        .expect_err("refused part");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
        assert!(
            error.detail().is_some_and(|text| text.contains(detail)),
            "{detail}: {error:?}"
        );
    }
    // Rows inserted below the pivot table leave it, and the write, as they are.
    let source = generated_grid_edit_fixture_with(&[
        ("xl/worksheets/_rels/sheet1.xml.rels", pivot_relationships),
        ("xl/pivotTables/pivotTable1.xml", pivot_table),
    ]);
    let draft = edited(
        &source,
        vec![WorkbookChange::insert_rows(sheet(1), row(30), 1).expect("insertion")],
    );
    write_and_reopen(&draft);
}

#[test]
fn moved_rows_reorder_the_saved_rows_and_carry_their_coordinates() {
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![WorkbookChange::move_rows(sheet(1), row(7), row(7), row(1)).expect("move")],
    );
    let (output, reopened) = write_and_reopen(&draft);
    // Row 7 lands on row 1, so rows 1 through 6 move down one.
    for (cell, expected) in [
        ("A1", "SUM(Things[val])"),
        ("B1", "ROW()"),
        ("D2", "SUM(B3:B6)"),
        ("D3", "B4*2"),
    ] {
        assert_eq!(formula(&reopened, "Data", cell), expected, "Data!{cell}");
    }
    assert_eq!(formula(&reopened, "Report", "A2"), "Data!B4");
    assert_eq!(formula(&reopened, "Report", "A1"), "SUM(Data!B3:B6)");
    let data = reopened.workbook().sheet_by_name("Data").expect("Data");
    assert_eq!(data.tables()[0].range().start().to_string(), "A2");
    assert_eq!(data.comments()[0].address().to_string(), "B5");
    assert_eq!(
        reopened.presentation().row_height(sheet(1), row(3)),
        Some(25.0)
    );
    let parts = parts(&output);
    let data_part = &parts[DATA];
    assert!(
        data_part.find(r#"<row r="1">"#) < data_part.find(r#"<row r="2">"#),
        "{data_part}"
    );
    assert_contains(
        data_part,
        &[
            r#"<mergeCell ref="G3:H4"/>"#,
            r#"<conditionalFormatting sqref="B3:B6">"#,
            r#"<autoFilter ref="L2:M5">"#,
            r#"<brk id="5" max="16383" man="1"/>"#,
        ],
    );
    assert_contains(
        &parts["xl/charts/chart1.xml"],
        &["<c:f>Data!$B$3:$B$6</c:f>"],
    );
    assert_contains(&parts["xl/drawings/vmlDrawing1.vml"], &["<x:Row>4</x:Row>"]);
}
