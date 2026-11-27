use super::patch_data_validations;
use crate::xlsx::package::PartPath;
use crate::{
    CellAddress, CellRange, DataValidation, DataValidationType, Sheet, SheetId, SheetName,
    SheetVisibility, WriteLimits,
};

const MAIN: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;

fn cell(text: &str) -> CellRange {
    let address = CellAddress::from_a1(text).expect("address");
    CellRange::new(address, address).expect("range")
}

fn list(at: &str, source: &str) -> DataValidation {
    DataValidation::new(vec![cell(at)], DataValidationType::List).with_formula1(source)
}

fn sheet(validations: Vec<DataValidation>) -> Sheet {
    let mut sheet = Sheet::new(
        SheetId::new(1).expect("sheet ID"),
        SheetName::new("Script").expect("sheet name"),
        SheetVisibility::Visible,
    );
    sheet.set_data_validations(validations);
    sheet
}

fn patch(body: &str, sheet: &Sheet) -> Option<String> {
    let xml = format!(r#"<worksheet {MAIN}>{body}</worksheet>"#);
    patch_data_validations(
        xml.as_bytes(),
        &PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part"),
        sheet,
        WriteLimits::default(),
    )
    .expect("patch")
    .map(|bytes| String::from_utf8(bytes).expect("UTF-8"))
}

const EXTENSION_RULE: &str = r#"<x14:dataValidation type="list"><x14:formula1><xm:f>Lists!$A$1</xm:f></x14:formula1><xm:sqref>E1</xm:sqref></x14:dataValidation>"#;

#[test]
fn an_extension_list_before_the_main_rules_is_emptied_without_losing_them() {
    let body = format!(
        r#"<sheetData/><extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}"><x14:dataValidations count="1">{EXTENSION_RULE}</x14:dataValidations></ext></extLst><dataValidations count="2"><dataValidation type="list" sqref="A1"><formula1>"a"</formula1></dataValidation><dataValidation type="list" sqref="B1"><formula1>"b"</formula1></dataValidation></dataValidations>"#
    );
    let patched = patch(&body, &sheet(vec![list("A1", "\"a\"")])).expect("changed");
    assert_eq!(
        patched,
        format!(
            r#"<worksheet {MAIN}><sheetData/><dataValidations count="1"><dataValidation type="list" sqref="A1"><formula1>"a"</formula1></dataValidation></dataValidations></worksheet>"#
        )
    );
}

#[test]
fn an_extension_rule_inside_a_compatibility_wrapper_is_matched_where_it_is() {
    let body = format!(
        r#"<sheetData/><extLst><mc:AlternateContent><mc:Choice Requires="x14"><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}"><x14:dataValidations count="1">{EXTENSION_RULE}</x14:dataValidations></ext></mc:Choice></mc:AlternateContent></extLst>"#
    );
    let held = list("E1", "Lists!$A$1");
    assert_eq!(
        patch(&body, &sheet(vec![held.clone()])),
        None,
        "the part holds the rule"
    );
    let patched = patch(&body, &sheet(vec![held, list("A1", "\"a\"")])).expect("changed");
    assert_eq!(
        patched.matches("<x14:dataValidation ").count(),
        1,
        "the wrapped rule is not written again: {patched}"
    );
    assert!(
        patched.contains(r#"<dataValidations count="1"><dataValidation type="list" sqref="A1">"#),
        "{patched}"
    );
}

#[test]
fn rules_are_read_as_the_reader_reads_them() {
    let whole = DataValidation::new(vec![cell("B2")], DataValidationType::Whole)
        .with_formula1("1")
        .with_formula2("9");
    let commented = r#"<dataValidation type="whole" sqref="B2"><formula1><!--c-->1</formula1><formula2>9</formula2></dataValidation>"#;
    let forbidden =
        r#"<dataValidation type="custom" sqref="E1"><formula1>A1&#x1;</formula1></dataValidation>"#;
    let unknown =
        r#"<dataValidation type="custom" sqref="E2"><formula1>&bogus;</formula1></dataValidation>"#;
    let body = format!(
        r#"<sheetData/><dataValidations count="3">{commented}{forbidden}{unknown}</dataValidations>"#
    );
    // The reader models the commented rule and leaves the other two, which it cannot decode,
    // unmodeled.
    let patched = patch(&body, &sheet(vec![whole, list("A1", "\"a\"")])).expect("changed");
    assert_eq!(patched.matches(r#"sqref="B2""#).count(), 1, "{patched}");
    assert!(patched.contains(forbidden), "{patched}");
    assert!(patched.contains(unknown), "{patched}");
    assert!(
        patched.contains(r#"<dataValidations count="4">"#),
        "{patched}"
    );
}
