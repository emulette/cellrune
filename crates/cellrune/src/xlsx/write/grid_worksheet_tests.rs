use super::patch_worksheet_grid;
use crate::grid::{GridAxis, GridEdit};
use crate::xlsx::package::PartPath;
use crate::{CellAddress, CellRange, WriteLimits, XlsxWriteErrorCode};

const MAIN: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main""#;

fn part() -> PartPath {
    PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part")
}

fn range(text: &str) -> CellRange {
    let (start, end) = text.split_once(':').expect("range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn patch(body: &str, edit: GridEdit, used: &str) -> Result<String, crate::XlsxWriteError> {
    let xml = format!(r#"<worksheet {MAIN}>{body}</worksheet>"#);
    patch_worksheet_grid(
        xml.as_bytes(),
        &part(),
        edit,
        "S1",
        Some(range(used)),
        WriteLimits::default(),
    )
    .map(|bytes| String::from_utf8(bytes).expect("UTF-8"))
}

const BODY: &str = concat!(
    r#"<dimension ref="A1:C4"/>"#,
    r#"<sheetViews><sheetView workbookViewId="0"><selection activeCell="B3" sqref="B3"/></sheetView></sheetViews>"#,
    r#"<sheetData>"#,
    r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
    r#"<row r="2" ht="30" customHeight="1" s="4" customFormat="1"><c r="A2" s="2"><v>2</v></c><c r="B2"/></row>"#,
    r#"<row r="3"><c r="A3"><v>3</v></c><c r="B3"><f>A3*2</f><v>6</v></c></row>"#,
    r#"<row r="4"><c r="C4"><v>4</v></c></row>"#,
    r#"</sheetData>"#,
    r#"<mergeCells count="1"><mergeCell ref="C2:C3"/></mergeCells>"#,
    r#"<conditionalFormatting sqref="A1:A4"><cfRule type="expression" priority="1"><formula>A1&gt;$C$4</formula></cfRule></conditionalFormatting>"#,
    r#"<conditionalFormatting sqref="B3"><cfRule type="expression" priority="2"><formula>B3&gt;0</formula></cfRule></conditionalFormatting>"#,
    r#"<dataValidations count="1"><dataValidation type="list" sqref="A1:A2"><formula1>$C$3:$C$4</formula1></dataValidation></dataValidations>"#,
    r#"<hyperlinks><hyperlink ref="C4" location="'S1'!A3"/></hyperlinks>"#,
    r#"<rowBreaks count="1" manualBreakCount="1"><brk id="2" max="16383" man="1"/></rowBreaks>"#,
);

#[test]
fn inserted_rows_move_cells_and_every_worksheet_coordinate() {
    let output = patch(
        BODY,
        GridEdit::Insert {
            axis: GridAxis::Rows,
            at: 3,
            count: 1,
        },
        "A1:C5",
    )
    .expect("patch");
    for expected in [
        r#"<dimension ref="A1:C5"/>"#,
        r#"<selection activeCell="B4" sqref="B4"/>"#,
        // The inserted row takes the row above's height, style, and cell styles.
        r#"<row r="3" ht="30" customHeight="1" s="4" customFormat="1"><c r="A3" s="2"/></row>"#,
        r#"<row r="4"><c r="A4"><v>3</v></c><c r="B4"><f>A3*2</f><v>6</v></c></row>"#,
        r#"<row r="5"><c r="C5"><v>4</v></c></row>"#,
        r#"<mergeCell ref="C2:C4"/>"#,
        r#"<conditionalFormatting sqref="A1:A5"><cfRule type="expression" priority="1"><formula>A1&gt;$C$5</formula>"#,
        r#"<conditionalFormatting sqref="B4"><cfRule type="expression" priority="2"><formula>B4&gt;0</formula>"#,
        // A validation covering the row above an insertion grows over the inserted row.
        r#"<dataValidation type="list" sqref="A1:A3"><formula1>$C$4:$C$5</formula1>"#,
        r#"<hyperlink ref="C5" location="'S1'!A3"/>"#,
        r#"<brk id="3" max="16383" man="1"/>"#,
    ] {
        assert!(output.contains(expected), "{expected}\n{output}");
    }
}

#[test]
fn deleted_rows_drop_their_cells_and_the_formats_left_without_cells() {
    let output = patch(
        BODY,
        GridEdit::Delete {
            axis: GridAxis::Rows,
            first: 3,
            last: 3,
        },
        "A1:C3",
    )
    .expect("patch");
    for expected in [
        r#"<row r="3"><c r="C3"><v>4</v></c></row>"#,
        r#"<conditionalFormatting sqref="A1:A3"><cfRule type="expression" priority="1"><formula>A1&gt;$C$3</formula>"#,
        r#"<dataValidation type="list" sqref="A1:A2"><formula1>$C$3:$C$3</formula1>"#,
        r#"<hyperlink ref="C3" location="'S1'!A3"/>"#,
    ] {
        assert!(output.contains(expected), "{expected}\n{output}");
    }
    for absent in ["B3&gt;0", "<mergeCells", "<rowBreaks", r#"<c r="B3">"#] {
        assert!(!output.contains(absent), "{absent}\n{output}");
    }
}

#[test]
fn inserted_columns_move_cells_and_column_spans() {
    let body = concat!(
        r#"<cols><col min="1" max="1" width="20" customWidth="1" style="3"/><col min="2" max="3" width="9" customWidth="1" hidden="1"/></cols>"#,
        r#"<sheetData><row r="1" spans="1:3"><c r="A1" s="3"><v>1</v></c><c r="B1"><v>2</v></c><c r="C1"><f>A1+B1</f></c></row></sheetData>"#,
        r#"<autoFilter ref="A1:C1"><filterColumn colId="2"><filters><filter val="3"/></filters></filterColumn></autoFilter>"#,
    );
    let output = patch(
        body,
        GridEdit::Insert {
            axis: GridAxis::Columns,
            at: 2,
            count: 1,
        },
        "A1:D1",
    )
    .expect("patch");
    for expected in [
        r#"<col min="1" max="2" width="20" customWidth="1" style="3"/><col min="3" max="4" width="9" customWidth="1" hidden="1"/>"#,
        r#"<row r="1"><c r="A1" s="3"><v>1</v></c><c r="B1" s="3"/><c r="C1"><v>2</v></c><c r="D1"><f>A1+B1</f></c></row>"#,
        r#"<autoFilter ref="A1:D1"><filterColumn colId="3">"#,
    ] {
        assert!(output.contains(expected), "{expected}\n{output}");
    }
}

#[test]
fn coordinates_no_pass_rewrites_are_refused_with_their_element() {
    for (body, detail) in [
        (
            r#"<sheetData/><controls><control shapeId="1025" r:id="rId1"/></controls>"#,
            "controls",
        ),
        (
            r#"<sheetData/><extLst><ext uri="{UNKNOWN}"><x14:unknown/></ext></extLst>"#,
            "{UNKNOWN}",
        ),
    ] {
        let error = patch(
            body,
            GridEdit::Insert {
                axis: GridAxis::Rows,
                at: 1,
                count: 1,
            },
            "A1:A1",
        )
        .expect_err("refused element");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
        assert!(
            error.detail().is_some_and(|text| text.contains(detail)),
            "{error:?}"
        );
    }
}

#[test]
fn extension_formats_and_sparklines_move_with_their_cells() {
    let body = concat!(
        r#"<sheetData/><extLst>"#,
        r#"<ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{1}"><x14:dataBar><x14:cfvo type="formula"><xm:f>$B$1</xm:f></x14:cfvo></x14:dataBar></x14:cfRule><xm:sqref>A2:A4</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext>"#,
        r#"<ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}"><x14:sparklineGroups><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>S1!A2:C2</xm:f><xm:sqref>D2</xm:sqref></x14:sparkline><x14:sparkline><xm:f>S1!A1:C1</xm:f><xm:sqref>D1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext>"#,
        r#"</extLst>"#,
    );
    let output = patch(
        body,
        GridEdit::Delete {
            axis: GridAxis::Rows,
            first: 1,
            last: 1,
        },
        "A1:A1",
    )
    .expect("patch");
    for expected in [
        "<xm:f>#REF!</xm:f></x14:cfvo>",
        "<xm:sqref>A1:A3</xm:sqref>",
        "<xm:f>S1!A1:C1</xm:f><xm:sqref>D1</xm:sqref>",
    ] {
        assert!(output.contains(expected), "{expected}\n{output}");
    }
    assert_eq!(output.matches("<x14:sparkline>").count(), 1, "{output}");
}
