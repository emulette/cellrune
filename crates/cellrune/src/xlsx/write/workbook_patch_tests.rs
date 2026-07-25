use super::patch_calculation_properties;
use crate::WriteLimits;
use crate::xlsx::package::PartPath;

fn part() -> PartPath {
    PartPath::from_archive_name(b"xl/workbook.xml").expect("valid part")
}

#[test]
fn existing_calculation_properties_preserve_producer_fields() {
    let source = br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/><calcPr calcId="191029" calcMode="manual" fullCalcOnLoad="0" forceFullCalc="0" concurrentCalc="1"/></workbook>"#;
    let output =
        patch_calculation_properties(source, &part(), true, WriteLimits::default()).expect("patch");
    let output = String::from_utf8(output).expect("UTF-8 XML");
    assert!(output.contains(r#"calcId="191029""#));
    assert!(output.contains(r#"calcMode="manual""#));
    assert!(output.contains(r#"concurrentCalc="1""#));
    assert!(output.contains(r#"fullCalcOnLoad="1""#));
    assert!(output.contains(r#"forceFullCalc="1""#));
    assert_eq!(output.matches("fullCalcOnLoad=").count(), 1);
    assert_eq!(output.matches("forceFullCalc=").count(), 1);
}

#[test]
fn missing_calculation_properties_are_inserted_before_extensions() {
    let source = br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/><extLst><ext uri="preserve"/></extLst></workbook>"#;
    let output = patch_calculation_properties(source, &part(), false, WriteLimits::default())
        .expect("patch");
    let output = String::from_utf8(output).expect("UTF-8 XML");
    let calc = output.find("<calcPr").expect("generated calcPr");
    let extensions = output.find("<extLst").expect("preserved extLst");
    assert!(
        calc < extensions,
        "calcPr must precede extLst in workbook order"
    );
    assert!(output.contains(r#"fullCalcOnLoad="0""#));
    assert!(output.contains(r#"forceFullCalc="0""#));
    assert!(output.contains(r#"<ext uri="preserve"/>"#));
}
