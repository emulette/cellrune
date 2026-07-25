use std::collections::BTreeSet;

use super::{remove_calculation_chain_relationship, remove_content_type_overrides};
use crate::xlsx::package::PartPath;
use crate::{WriteLimits, XlsxWriteErrorCode};

fn part(value: &[u8]) -> PartPath {
    PartPath::from_archive_name(value).expect("valid package part")
}

#[test]
fn non_empty_calculation_chain_relationship_is_removed() {
    let relationships = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"><extension/></Relationship><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let relationship_part = part(b"xl/_rels/workbook.xml.rels");
    let workbook_part = part(b"xl/workbook.xml");
    let patch = remove_calculation_chain_relationship(
        relationships,
        &relationship_part,
        &workbook_part,
        WriteLimits::default(),
    )
    .expect("relationship patch");
    assert_eq!(
        patch.removed_parts,
        BTreeSet::from([part(b"xl/calcChain.xml")])
    );
    let output = String::from_utf8(patch.relationship_bytes.expect("rewritten relationships"))
        .expect("UTF-8 XML");
    assert!(!output.contains("calcChain"));
    assert!(output.contains("worksheets/sheet1.xml"));
}

#[test]
fn strict_calculation_chain_relationship_is_removed() {
    let relationships = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#;
    let relationship_part = part(b"xl/_rels/workbook.xml.rels");
    let workbook_part = part(b"xl/workbook.xml");

    let patch = remove_calculation_chain_relationship(
        relationships,
        &relationship_part,
        &workbook_part,
        WriteLimits::default(),
    )
    .expect("strict relationship patch");

    assert_eq!(
        patch.removed_parts,
        BTreeSet::from([part(b"xl/calcChain.xml")])
    );
}

#[test]
fn custom_external_and_nested_calc_chain_lookalikes_are_preserved() {
    let relationships = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="https://example.test/custom/calcChain" Target="custom.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="https://example.test/calcChain.xml" TargetMode="External"/><Extension><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="nested.xml"/></Extension></Relationships>"#;
    let relationship_part = part(b"xl/_rels/workbook.xml.rels");
    let workbook_part = part(b"xl/workbook.xml");

    let patch = remove_calculation_chain_relationship(
        relationships,
        &relationship_part,
        &workbook_part,
        WriteLimits::default(),
    )
    .expect("lookalike relationships");

    assert!(patch.removed_parts.is_empty());
    assert!(patch.relationship_bytes.is_none());
}

#[test]
fn content_type_count_accepts_the_exact_limit_and_rejects_one_less() {
    let content_types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="workbook"/><Override PartName="/xl/calcChain.xml" ContentType="calc-chain"/></Types>"#;
    let content_types_part = part(b"[Content_Types].xml");
    let removals = BTreeSet::from([part(b"xl/calcChain.xml")]);
    let exact = WriteLimits::default()
        .with_max_content_types(2)
        .expect("positive exact limit");
    let output =
        remove_content_type_overrides(content_types, &content_types_part, &removals, exact)
            .expect("exact declaration count is valid");
    assert!(
        !String::from_utf8(output)
            .expect("UTF-8 XML")
            .contains("calcChain")
    );

    let below = WriteLimits::default()
        .with_max_content_types(1)
        .expect("positive lower limit");
    let error = remove_content_type_overrides(content_types, &content_types_part, &removals, below)
        .expect_err("declarations exceed lower limit");
    assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
}
