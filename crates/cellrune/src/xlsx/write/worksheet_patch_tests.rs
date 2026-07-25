use std::collections::BTreeMap;

use super::{WorksheetCacheAction, WorksheetCellUpdate, patch_worksheet};
use crate::xlsx::package::PartPath;
use crate::{CellAddress, CellValue, ExcelError, WriteLimits, XlsxWriteErrorCode};

fn part() -> PartPath {
    PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("valid part")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid address")
}

fn update(value: CellValue, requires_formula: bool) -> WorksheetCellUpdate {
    WorksheetCellUpdate {
        action: WorksheetCacheAction::Set(value),
        requires_formula,
    }
}

#[test]
fn typed_caches_replace_only_type_and_value_content() {
    let source = br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="B2" s="7" t="n" custom="keep"><f customFormula="yes">OLD&amp;FORMULA</f><v>stale</v><ext:marker xmlns:ext="urn:test"/></c></row><row r="3"><c r="B3"><f>TRUE()</f><v>stale</v></c></row><row r="4"><c r="B4"><f>1/0</f><v>stale</v></c></row><row r="5"><c r="B5"><f>1/10</f><v>stale</v></c></row><row r="6"><c r="B6"><f>""</f><v>stale</v></c></row><row r="7"><c r="B7" t="str"><f>UNKNOWN()</f><v>stale</v></c></row></sheetData><extLst><ext uri="preserve"/></extLst></worksheet>"#;
    let mut updates = BTreeMap::new();
    updates.insert(
        address("B2"),
        update(CellValue::Text("<한글 & Ω>".to_owned()), true),
    );
    updates.insert(address("B3"), update(CellValue::Logical(true), true));
    updates.insert(
        address("B4"),
        update(CellValue::Error(ExcelError::DivisionByZero), true),
    );
    updates.insert(
        address("B5"),
        update(
            CellValue::number(1.234_567_890_123_456).expect("finite"),
            true,
        ),
    );
    updates.insert(address("B6"), update(CellValue::Text(String::new()), true));
    updates.insert(
        address("B7"),
        WorksheetCellUpdate {
            action: WorksheetCacheAction::Invalidate,
            requires_formula: true,
        },
    );

    let output = patch_worksheet(source, &part(), &updates, WriteLimits::default()).expect("patch");
    let output = String::from_utf8(output).expect("UTF-8 XML");
    assert!(output.contains(r#"s="7""#));
    assert!(output.contains(r#"custom="keep""#));
    assert!(output.contains(r#"customFormula="yes""#));
    assert!(output.contains("OLD&amp;FORMULA"));
    assert!(output.contains("ext:marker"));
    assert!(output.contains(r#"<ext uri="preserve"/>"#));
    assert!(output.contains("&lt;한글 &amp; Ω&gt;"));
    assert!(output.contains(r#"<v>1</v>"#));
    assert!(output.contains(r#"<v>#DIV/0!</v>"#));
    assert!(output.contains(r#"<v>1.234567890123456</v>"#));
    assert!(output.contains(r#"<v></v>"#));
    let invalidated = output
        .split(r#"<c r="B7""#)
        .nth(1)
        .expect("B7")
        .split("</c>")
        .next()
        .expect("B7 content");
    assert!(!invalidated.contains("<v"));
}

#[test]
fn missing_array_followers_are_inserted_in_row_major_order() {
    let source = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="A2"><f>1</f></c><c r="C2"><v>9</v></c></row></sheetData></worksheet>"#;
    let mut updates = BTreeMap::new();
    updates.insert(
        address("A2"),
        update(CellValue::number(1.0).expect("finite"), true),
    );
    updates.insert(
        address("B2"),
        update(CellValue::number(2.0).expect("finite"), false),
    );
    updates.insert(
        address("A3"),
        update(CellValue::number(3.0).expect("finite"), false),
    );
    let output = patch_worksheet(source, &part(), &updates, WriteLimits::default()).expect("patch");
    let output = String::from_utf8(output).expect("UTF-8 XML");
    assert!(output.find(r#"r="A2""#) < output.find(r#"r="B2""#));
    assert!(output.find(r#"r="B2""#) < output.find(r#"r="C2""#));
    assert!(output.find(r#"<row r="3""#) > output.find(r#"<row r="2""#));
    assert!(output.contains(r#"<c r="A3" t="n"><v>3</v></c>"#));
}

#[test]
fn self_closing_follower_cells_and_rows_are_materialized_in_place() {
    let source = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="A2"><f>1</f></c><c r="B2"/></row><row r="3"/></sheetData></worksheet>"#;
    let mut updates = BTreeMap::new();
    updates.insert(
        address("A2"),
        update(CellValue::number(1.0).expect("finite"), true),
    );
    updates.insert(
        address("B2"),
        update(CellValue::number(2.0).expect("finite"), false),
    );
    updates.insert(
        address("A3"),
        update(CellValue::number(3.0).expect("finite"), false),
    );

    let output = patch_worksheet(source, &part(), &updates, WriteLimits::default()).expect("patch");
    let output = String::from_utf8(output).expect("UTF-8 XML");
    assert!(output.contains(r#"<c r="B2" t="n"><v>2</v></c>"#));
    assert!(output.contains(r#"<row r="3"><c r="A3" t="n"><v>3</v></c></row>"#));
    assert_eq!(output.matches(r#"<row r="3""#).count(), 1);
}

#[test]
fn invalid_targets_and_unrepresentable_blanks_fail_closed() {
    let source = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#;
    let mut missing_formula = BTreeMap::new();
    missing_formula.insert(
        address("A1"),
        update(CellValue::number(1.0).expect("finite"), true),
    );
    let missing = patch_worksheet(source, &part(), &missing_formula, WriteLimits::default())
        .expect_err("missing formula target");
    assert_eq!(missing.code(), XlsxWriteErrorCode::InvalidGeneratedXml);

    let mut blank = BTreeMap::new();
    blank.insert(address("A1"), update(CellValue::Blank, false));
    let blank_error =
        patch_worksheet(source, &part(), &blank, WriteLimits::default()).expect_err("blank cache");
    assert_eq!(
        blank_error.code(),
        XlsxWriteErrorCode::UnsupportedResultMaterialization
    );

    let invalid_row = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1048577"/></sheetData></worksheet>"#;
    let row_error = patch_worksheet(
        invalid_row,
        &part(),
        &BTreeMap::new(),
        WriteLimits::default(),
    )
    .expect_err("row outside Excel bounds");
    assert_eq!(row_error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);
}

#[test]
fn rejected_worksheets_report_the_cause_that_actually_applies() {
    let out_of_range = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1048577"/></sheetData></worksheet>"#;
    let error = patch_worksheet(
        out_of_range,
        &part(),
        &BTreeMap::new(),
        WriteLimits::default(),
    )
    .expect_err("row outside Excel bounds");
    let detail = error.detail().expect("detail must state the cause");
    assert!(
        detail.contains("outside the supported range") && detail.contains("1048577"),
        "row-range failure must name the range and the value, got {detail:?}"
    );

    // A row without an "r" attribute is not supported by the patch writer, but
    // the reason reported must be the missing attribute rather than an ordering
    // problem that does not exist in this document.
    let implicit_row = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#;
    let error = patch_worksheet(
        implicit_row,
        &part(),
        &BTreeMap::new(),
        WriteLimits::default(),
    )
    .expect_err("row without an explicit number");
    let detail = error.detail().expect("detail must state the cause");
    assert!(
        detail.contains("does not declare a required attribute") && detail.contains("r on <row>"),
        "missing-attribute failure must name the attribute and element, got {detail:?}"
    );
    assert!(
        !detail.contains("ascending"),
        "an ordering cause must not be reported for a missing attribute, got {detail:?}"
    );
}
