use std::collections::BTreeMap;

use super::{marked, split_mark};
use crate::WriteLimits;
use crate::xlsx::package::PartPath;

fn mark(workbook: &str) -> String {
    let part = PartPath::from_archive_name(b"xl/workbook.xml").expect("part");
    let mut parts = BTreeMap::new();
    parts.insert(
        PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part"),
        b"<worksheet/>".to_vec(),
    );
    let marked = marked(workbook.as_bytes(), &part, &parts, WriteLimits::default()).expect("mark");
    String::from_utf8(marked).expect("UTF-8")
}

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

#[test]
fn the_mark_joins_the_one_extension_list_whatever_its_form() {
    for (source, list) in [
        (
            format!(r#"<workbook xmlns="{MAIN}"><sheets/></workbook>"#),
            "<extLst>",
        ),
        (
            format!(r#"<workbook xmlns="{MAIN}"><sheets/><extLst/></workbook>"#),
            "<extLst>",
        ),
        (
            format!(
                r#"<workbook xmlns="{MAIN}"><sheets/><extLst><ext uri="{{other}}"/></extLst><!-- note --></workbook>"#
            ),
            "<extLst>",
        ),
        (
            format!(r#"<x:workbook xmlns:x="{MAIN}"><x:sheets/></x:workbook>"#),
            "<x:extLst>",
        ),
    ] {
        let marked = mark(&source);
        assert_eq!(marked.matches(list).count(), 1, "{marked}");
        let (unmarked, digest) = split_mark(marked.as_bytes());
        assert_eq!(digest.map(|digest| digest.len()), Some(64), "{marked}");
        assert!(
            !String::from_utf8(unmarked)
                .expect("UTF-8")
                .contains("savedResults")
        );
        assert_eq!(mark(&marked), marked, "marking again replaces the mark");
    }
}
