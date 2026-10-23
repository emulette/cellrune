//! How the preservation writer adds font, fill, border, and xf records to a source styles part.

use cellrune::{
    CellAddress, CellFormatEdit, Color, EditBatch, Fill, FillPattern, OpenOptions, PatternFill,
    SheetId, WorkbookChange, WorkbookDraft, WriteOptions, open_xlsx_document_bytes,
    write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::generated_package;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
const STYLES_PART: &str = "xl/styles.xml";

/// A package whose only sheet, `Sheet1`, is `worksheet`, with `styles` when given.
pub(super) fn one_sheet_package(worksheet: &str, styles: Option<&str>) -> Vec<u8> {
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
    );
    let mut relationships = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    );
    if styles.is_some() {
        content_types.push_str(r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#);
        relationships.push_str(r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    }
    content_types.push_str("</Types>");
    relationships.push_str("</Relationships>");
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let mut entries = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", worksheet),
    ];
    if let Some(styles) = styles {
        entries.push((STYLES_PART, styles));
    }
    generated_package(&entries)
}

pub(super) const ONE_VALUE_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row></sheetData></worksheet>"#;

pub(super) fn sheet1() -> SheetId {
    SheetId::new(1).expect("sheet")
}

pub(super) fn address(a1: &str) -> CellAddress {
    CellAddress::from_a1(a1).expect("address")
}

pub(super) fn edit_and_save(source: &[u8], changes: Vec<WorkbookChange>) -> Vec<u8> {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new(changes))
        .expect("changes");
    write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("save")
        .into_bytes()
}

/// Returns the `<fill>` records of a styles part in order.
fn fill_records(styles: &str) -> Vec<String> {
    let fills = styles
        .split_once("<fills")
        .and_then(|(_, rest)| rest.split_once("</fills>"))
        .map(|(fills, _)| fills)
        .expect("fills section");
    fills
        .split("<fill>")
        .skip(1)
        .map(|record| record.trim_end_matches("</fill>").to_owned())
        .collect()
}

#[test]
fn an_added_fill_never_takes_a_reserved_fill_index() {
    let red = PatternFill::solid(Color::argb(0xFFFF_0000));
    for fills in [
        r#"<fills count="1"><fill><patternFill patternType="none"/></fill></fills>"#,
        "",
    ] {
        let styles = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>{fills}<borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#
        );
        let output = edit_and_save(
            &one_sheet_package(ONE_VALUE_SHEET, Some(&styles)),
            vec![WorkbookChange::set_cell_format(
                sheet1(),
                address("A1"),
                CellFormatEdit::new().with_fill(red),
            )],
        );
        let written = support::part_text(&output, STYLES_PART);
        assert_eq!(
            fill_records(&written),
            [
                r#"<patternFill patternType="none"/>"#,
                r#"<patternFill patternType="gray125"/>"#,
                r#"<patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill>"#,
            ],
            "{written}"
        );
        assert!(written.contains(r#"fillId="2""#), "{written}");
        let reopened = WorkbookDraft::from_document(
            &open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen"),
        );
        assert_eq!(
            reopened
                .cell_format(sheet1(), address("A1"))
                .expect("A1")
                .fill(),
            Fill::Pattern(red)
        );
        assert_eq!(
            reopened
                .cell_format(sheet1(), address("B1"))
                .expect("B1")
                .fill(),
            Fill::Pattern(PatternFill::new(FillPattern::None))
        );
    }
}

/// Returns the `count` of a styles section.
fn section_count(styles: &str, section: &str) -> usize {
    let start = format!("<{section} count=\"");
    let (_, rest) = styles.split_once(&start).expect("section");
    rest.split_once('"')
        .and_then(|(count, _)| count.parse().ok())
        .expect("count")
}

#[test]
fn edit_sessions_reuse_records_equal_to_the_ones_they_need() {
    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF0000FF"/></patternFill></fill><fill>
  <patternFill patternType="solid"><fgColor rgb="FF00FF00"/></patternFill>
</fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#;
    let red = CellFormatEdit::new().with_fill(PatternFill::solid(Color::argb(0xFFFF_0000)));
    let mut bytes = one_sheet_package(ONE_VALUE_SHEET, Some(STYLES));
    let mut counts = Vec::new();
    for row in 2..=5 {
        bytes = edit_and_save(
            &bytes,
            vec![WorkbookChange::set_cell_format(
                sheet1(),
                CellAddress::from_indices(row, 1).expect("address"),
                red.clone(),
            )],
        );
        let styles = support::part_text(&bytes, STYLES_PART);
        counts.push((
            section_count(&styles, "fonts"),
            section_count(&styles, "fills"),
            section_count(&styles, "cellXfs"),
        ));
    }
    assert_eq!(
        counts,
        [(2, 4, 2); 4],
        "the first session adds one fill and one xf"
    );

    // A source fill equal to the requested one is reused, whitespace and all; a fill that only a
    // reserved index holds and a font with unmodeled attributes are not.
    let green = PatternFill::solid(Color::argb(0xFF00_FF00));
    let blue = PatternFill::solid(Color::argb(0xFF00_00FF));
    let bold = cellrune::Font::new()
        .with_name("Calibri")
        .and_then(|font| font.with_size(11.0))
        .expect("font")
        .with_bold(true);
    let output = edit_and_save(
        &one_sheet_package(ONE_VALUE_SHEET, Some(STYLES)),
        vec![
            WorkbookChange::set_cell_format(
                sheet1(),
                address("A1"),
                CellFormatEdit::new().with_fill(green),
            ),
            WorkbookChange::set_cell_format(
                sheet1(),
                address("B1"),
                CellFormatEdit::new()
                    .with_fill(blue)
                    .with_font(bold.clone()),
            ),
        ],
    );
    let styles = support::part_text(&output, STYLES_PART);
    assert_eq!(section_count(&styles, "fills"), 4, "{styles}");
    assert_eq!(section_count(&styles, "fonts"), 3, "{styles}");
    assert!(
        styles.contains(r#"fillId="2""#) && styles.contains(r#"fillId="3""#),
        "{styles}"
    );
    let reopened = WorkbookDraft::from_document(
        &open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen"),
    );
    let a1 = reopened.cell_format(sheet1(), address("A1")).expect("A1");
    let b1 = reopened.cell_format(sheet1(), address("B1")).expect("B1");
    assert_eq!(a1.fill(), Fill::Pattern(green));
    assert_eq!((b1.fill(), b1.font()), (Fill::Pattern(blue), &bold));
}

#[test]
fn a_package_without_styles_displays_the_default_style_it_is_saved_with() {
    let source = one_sheet_package(ONE_VALUE_SHEET, None);
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let draft = WorkbookDraft::from_document(&document);
    let unedited = draft.cell_format(sheet1(), address("B1")).expect("B1");
    assert_eq!(
        (unedited.font().name(), unedited.font().size()),
        (Some("Calibri"), Some(11.0))
    );
    let green = PatternFill::solid(Color::argb(0xFF00_FF00));
    let output = edit_and_save(
        &source,
        vec![WorkbookChange::set_cell_format(
            sheet1(),
            address("A1"),
            CellFormatEdit::new().with_fill(green),
        )],
    );
    let styles = support::part_text(&output, STYLES_PART);
    assert_eq!(section_count(&styles, "fonts"), 1, "{styles}");
    let reopened = WorkbookDraft::from_document(
        &open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen"),
    );
    assert_eq!(
        reopened.cell_format(sheet1(), address("B1")).expect("B1"),
        unedited
    );
    let edited = reopened.cell_format(sheet1(), address("A1")).expect("A1");
    assert_eq!(
        (edited.font(), edited.fill()),
        (unedited.font(), Fill::Pattern(green))
    );
}
