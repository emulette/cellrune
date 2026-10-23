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

#[test]
fn a_range_move_carries_its_cells_filter_and_links_in_the_saved_package() {
    let range = |start: &str, end: &str| {
        cellrune::CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    };
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![
            WorkbookChange::move_range(
                sheet(1),
                range("L1", "M4"),
                CellAddress::from_a1("O1").expect("O1"),
            )
            .expect("move"),
            WorkbookChange::move_range(
                sheet(1),
                range("G5", "G5"),
                CellAddress::from_a1("J20").expect("J20"),
            )
            .expect("move"),
        ],
    );
    let (output, reopened) = write_and_reopen(&draft);
    assert_eq!(
        literal(&reopened, "Data", "O2"),
        Some(CellValue::Text("x".to_owned()))
    );
    assert_eq!(literal(&reopened, "Data", "L2"), None);
    assert_eq!(
        names(&reopened)[0],
        (
            "_xlnm._FilterDatabase".to_owned(),
            "Data!$O$1:$P$4".to_owned()
        )
    );
    let data = reopened.workbook().sheet_by_name("Data").expect("Data");
    assert_eq!(data.hyperlinks()[0].range().start().to_string(), "J20");
    let parts = parts(&output);
    assert_contains(
        &parts[DATA],
        &[
            r#"<autoFilter ref="O1:P4"><filterColumn colId="1">"#,
            "<formula1>$O$2:$O$4</formula1>",
            r#"<hyperlink ref="J20" location="'Data'!A1" display="top"/>"#,
        ],
    );
    assert!(!parts[DATA].contains(r#"<c r="L1""#), "{}", parts[DATA]);
}

#[test]
fn copies_save_shifted_formulas_with_their_values_and_formats() {
    let range = |text: &str| {
        let (start, end) = text.split_once(':').unwrap_or((text, text));
        cellrune::CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    };
    let source = generated_grid_edit_fixture();
    let draft = edited(
        &source,
        vec![
            WorkbookChange::copy_range(sheet(1), range("B2"), sheet(1), range("F10"))
                .expect("copy"),
            WorkbookChange::copy_range(sheet(1), range("D2"), sheet(1), range("D10:D11"))
                .expect("fill"),
            WorkbookChange::copy_range(sheet(1), range("A7"), sheet(2), range("A7")).expect("copy"),
        ],
    );
    let (_, reopened) = write_and_reopen(&draft);
    assert_eq!(formula(&reopened, "Data", "D10"), "B11*2");
    assert_eq!(formula(&reopened, "Data", "D11"), "B12*2");
    assert_eq!(formula(&reopened, "Report", "A7"), "SUM(Things[val])");
    assert_eq!(
        literal(&reopened, "Data", "F10"),
        Some(CellValue::number(1.0).expect("number"))
    );
    let saved = WorkbookDraft::from_document(&reopened);
    assert_eq!(
        saved
            .cell_format(sheet(1), CellAddress::from_a1("F10").expect("F10"))
            .expect("format"),
        saved
            .cell_format(sheet(1), CellAddress::from_a1("B2").expect("B2"))
            .expect("format")
    );
}

#[test]
fn the_inverse_of_a_deletion_saves_the_source_cells_again() {
    let source = generated_grid_edit_fixture();
    let document = open(&source);
    let mut draft = WorkbookDraft::from_document(&document);
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::delete_rows(
            sheet(1),
            row(3),
            row(3),
        )
        .expect("deletion")]))
        .expect("deletion");
    let inverse = receipt.inverse().expect("inverse").clone();
    assert!(matches!(
        inverse.changes().first(),
        Some(WorkbookChange::InsertRows { count: 1, .. })
    ));
    assert!(inverse.unrestored().iter().any(|content| {
        content.kind() == cellrune::UnrestoredContentKind::PreservedContent
            && content.sheet_id() == Some(sheet(1))
    }));
    draft
        .apply_changes(EditBatch::new(inverse.changes().to_vec()))
        .expect("undo");
    let (_, reopened) = write_and_reopen(&draft);
    let texts = |document: &XlsxDocument, name: &str| {
        document
            .workbook()
            .sheet_by_name(name)
            .expect("sheet")
            .cells()
            .map(|cell| {
                let text = match cell.content() {
                    CellContent::Formula(formula) => {
                        formula.text().map(|text| text.as_str().to_owned())
                    }
                    CellContent::Literal(value) => Some(format!("{value:?}")),
                };
                (cell.address().to_string(), text)
            })
            .collect::<Vec<_>>()
    };
    for name in ["Data", "Report"] {
        assert_eq!(texts(&reopened, name), texts(&document, name), "{name}");
    }
    assert_eq!(names(&reopened), names(&document));
}

#[test]
fn deleting_a_shared_formula_master_saves_its_followers_as_formulas_of_their_own() {
    // `D3` is the master of the shared group `D3:D5`, whose followers' text deleting row 3 does
    // not change.
    let source = support::replace_part_text(
        &generated_grid_edit_fixture(),
        DATA,
        r#"<f t="shared" ref="D3:D5" si="0">B2+1</f>"#,
        r#"<f t="shared" ref="D3:D5" si="0">$B$2+1</f>"#,
    );
    let draft = edited(
        &source,
        vec![WorkbookChange::delete_rows(sheet(1), row(3), row(3)).expect("delete")],
    );
    let (_, reopened) = write_and_reopen(&draft);
    assert_eq!(formula(&reopened, "Data", "D3"), "$B$2+1");
    assert_eq!(formula(&reopened, "Data", "D4"), "$B$2+1");
}

#[test]
fn a_row_move_that_crosses_a_drawing_anchor_shrinks_it_instead_of_inverting_it() {
    // The chart's anchor moved below the table to zero-based rows 11 to 19; moving its last row,
    // 20, to row 10 would put that end before the first. The anchor shrinks as a range does: its
    // other rows move down past the inserted row.
    let source = support::replace_part_text(
        &support::replace_part_text(
            &generated_grid_edit_fixture(),
            "xl/drawings/drawing1.xml",
            "<xdr:row>1</xdr:row>",
            "<xdr:row>11</xdr:row>",
        ),
        "xl/drawings/drawing1.xml",
        "<xdr:row>9</xdr:row>",
        "<xdr:row>19</xdr:row>",
    );
    let draft = edited(
        &source,
        vec![WorkbookChange::move_rows(sheet(1), row(20), row(20), row(10)).expect("move")],
    );
    let (output, _) = write_and_reopen(&draft);
    let drawing = parts(&output)["xl/drawings/drawing1.xml"].clone();
    let rows = drawing
        .match_indices("<xdr:row>")
        .map(|(index, tag)| {
            let start = index + tag.len();
            let end = start + drawing[start..].find('<').expect("row end");
            drawing[start..end].parse::<u32>().expect("row")
        })
        .collect::<Vec<_>>();
    assert_eq!(rows, [12, 19], "{drawing}");
}

#[test]
fn cutting_part_of_a_relative_conditional_format_splits_it_in_the_saved_part() {
    let source = support::replace_part_text(
        &generated_grid_edit_fixture(),
        DATA,
        r#"<conditionalFormatting sqref="B2:B5"><cfRule type="expression" dxfId="0" priority="1"><formula>B2&gt;2</formula>"#,
        r#"<conditionalFormatting sqref="H2:H5"><cfRule type="expression" dxfId="0" priority="1"><formula>H2&gt;2</formula>"#,
    );
    let range = |start: &str, end: &str| {
        cellrune::CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    };
    let draft = edited(
        &source,
        vec![
            WorkbookChange::move_range(
                sheet(1),
                range("H4", "H5"),
                CellAddress::from_a1("P20").expect("destination"),
            )
            .expect("move"),
        ],
    );
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        &draft,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("split rule");
    let worksheet = support::part_text(output.bytes(), DATA);
    assert!(
        worksheet.contains(r#"<conditionalFormatting sqref="H2:H3"><cfRule type="expression" dxfId="0" priority="1"><formula>H2&gt;2</formula>"#),
        "{worksheet}"
    );
    assert!(
        worksheet.contains(r#"<conditionalFormatting sqref="P20:P21"><cfRule type="expression" dxfId="0" priority="#),
        "{worksheet}"
    );
    assert!(
        worksheet.contains("<formula>P20&gt;2</formula>"),
        "{worksheet}"
    );
}

#[test]
fn the_inverse_of_a_row_move_lists_the_preserved_content_it_cannot_restore() {
    let mut draft = WorkbookDraft::from_document(&open(&generated_grid_edit_fixture()));
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::move_rows(
            sheet(1),
            row(7),
            row(7),
            row(12),
        )
        .expect("move")]))
        .expect("move");
    let inverse = receipt.inverse().expect("inverse");
    assert!(inverse.unrestored().iter().any(|content| {
        content.kind() == cellrune::UnrestoredContentKind::PreservedContent
            && content.sheet_id() == Some(sheet(1))
    }));
}

#[test]
fn a_shape_linked_to_moved_cells_is_refused_with_its_part() {
    let shape = r#"<xdr:twoCellAnchor><xdr:from><xdr:col>16</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>18</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro="" textlink="$B$4"><xdr:nvSpPr><xdr:cNvPr id="3" name="Box"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#;
    let source = support::replace_part_text(
        &generated_grid_edit_fixture(),
        "xl/drawings/drawing1.xml",
        "</xdr:wsDr>",
        shape,
    );
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
    .expect_err("the shape's link would name other cells");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    assert_eq!(
        error.source_id().map(|source| source.as_str()),
        Some("xl/drawings/drawing1.xml")
    );
}

#[test]
fn a_drawing_anchor_outside_the_grid_is_refused_with_its_part() {
    let picture = r#"<xdr:oneCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4294967295</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="9525" cy="9525"/><xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="4" name="Far"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr/></xdr:sp><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>"#;
    let source = support::replace_part_text(
        &generated_grid_edit_fixture(),
        "xl/drawings/drawing1.xml",
        "</xdr:wsDr>",
        picture,
    );
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
    .expect_err("the anchor cannot follow the insertion");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    assert_eq!(
        error.source_id().map(|source| source.as_str()),
        Some("xl/drawings/drawing1.xml")
    );
}

#[test]
fn a_range_move_splits_relative_rules_as_excel_does() {
    use super::style_record_write::{address, edit_and_save, one_sheet_package, sheet1};

    // Excel 16.114 (excel-measure I4): cutting A2:A3 of a rule on A1:A6 to D2 leaves the rule on
    // A1 and A4:A6 and adds one on D2:D3 that reads as the rule did at A2, following the move.
    const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>3</v></c></row><row r="2"><c r="A2"><v>2</v></c></row></sheetData><conditionalFormatting sqref="A1:A6"><cfRule type="expression" dxfId="0" priority="3"><formula>A1&gt;B1</formula></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="custom" sqref="A1:A6"><formula1>A1&lt;B1+10</formula1></dataValidation></dataValidations></worksheet>"#;
    let output = edit_and_save(
        &one_sheet_package(SHEET, None),
        vec![
            WorkbookChange::move_range(
                sheet1(),
                cellrune::CellRange::new(address("A2"), address("A3")).expect("range"),
                address("D2"),
            )
            .expect("move"),
        ],
    );
    let worksheet = support::part_text(&output, "xl/worksheets/sheet1.xml");
    for expected in [
        r#"<conditionalFormatting sqref="A1 A4:A6"><cfRule type="expression" dxfId="0" priority="3"><formula>A1&gt;B1</formula></cfRule></conditionalFormatting>"#,
        r#"<conditionalFormatting sqref="D2:D3"><cfRule type="expression" dxfId="0" priority="4"><formula>D2&gt;B2</formula></cfRule></conditionalFormatting>"#,
        r#"<dataValidation type="custom" sqref="A1 A4:A6"><formula1>A1&lt;B1+10</formula1></dataValidation>"#,
        r#"<dataValidation type="custom" sqref="D2:D3"><formula1>D2&lt;B2+10</formula1></dataValidation>"#,
        r#"<dataValidations count="2">"#,
    ] {
        assert!(worksheet.contains(expected), "{expected}\n{worksheet}");
    }
}

#[test]
fn deleting_table_columns_saves_the_narrowed_or_removed_table() {
    let column = |index| cellrune::Column::new(index).expect("column");
    for (first, last, expected) in [(2, 2, vec![("A1", "A5", vec!["key"])]), (1, 2, Vec::new())] {
        let draft = edited(
            &generated_grid_edit_fixture(),
            vec![
                WorkbookChange::delete_columns(sheet(1), column(first), column(last))
                    .expect("deletion"),
            ],
        );
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output = write_xlsx_draft_bytes(
            &draft,
            &calculation,
            RecalculationWriteOptions::default()
                .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
        )
        .expect("saved deletion");
        let reopened = open(output.bytes());
        let data = reopened.workbook().sheet_by_name("Data").expect("Data");
        let tables = data
            .tables()
            .iter()
            .map(|table| {
                (
                    table.range().start().to_string(),
                    table.range().end().to_string(),
                    table
                        .columns()
                        .iter()
                        .map(|column| column.name().to_owned())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let expected = expected
            .into_iter()
            .map(|(start, end, columns)| {
                (
                    start.to_owned(),
                    end.to_owned(),
                    columns.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(tables, expected, "{first}:{last}");
        // SUM(Things[val]) in A7 names a removed column, then a removed table.
        let reads_ref = data.cells().any(|cell| {
            matches!(
                cell.content(),
                CellContent::Formula(formula)
                    if formula.text().is_some_and(|text| text.as_str() == "SUM(#REF!)")
            )
        });
        assert!(reads_ref, "{first}:{last}");
    }
}
