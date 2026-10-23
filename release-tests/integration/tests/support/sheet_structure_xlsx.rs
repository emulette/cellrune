//! A generated six-sheet package for sheet moves and copies, after the workbook Excel 16.114 used
//! for its measurements: `S1` through `S6` each hold their index in `A1`, and `S1` sums 3-D spans.

use super::generated_xlsx::generated_package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// `S1` holds `SUM('S2:S5'!A1)`, `SUM('S3:S4'!A1)`, `SUM('S2:S3'!A1)`, `SHEET('S5'!A1)`, and
/// reads of `Span` and `'S6'!A1`; `Local` is scoped to `S3` and `Span` reads `'S3:S4'!$A$1`. `S3` is
/// active and `S2` first in the tab bar.
pub fn generated_sheet_structure_fixture() -> Vec<u8> {
    let overrides = (1..=6)
        .map(|index| {
            format!(
                r#"<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
            )
        })
        .collect::<String>();
    let content_types = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{overrides}</Types>"#
    );
    let root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook_relationships = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#,
        (1..=6)
            .map(|index| format!(
                r#"<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>"#
            ))
            .collect::<String>()
    );
    let sheets = (1..=6)
        .map(|index| format!(r#"<sheet name="S{index}" sheetId="{index}" r:id="rId{index}"/>"#))
        .collect::<String>();
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><bookViews><workbookView xWindow="0" yWindow="0" firstSheet="1" activeTab="2"/></bookViews><sheets>{sheets}</sheets><definedNames><definedName name="Local" localSheetId="2">'S3'!$A$1</definedName><definedName name="Span">'S3:S4'!$A$1</definedName></definedNames><calcPr calcId="191029"/></workbook>"#
    );
    let s1 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>SUM('S2:S5'!A1)</f><v>14</v></c></row><row r="2"><c r="B2"><f>SUM('S3:S4'!A1)</f><v>7</v></c></row><row r="3"><c r="B3"><f>SUM('S2:S3'!A1)</f><v>5</v></c></row><row r="4"><c r="B4"><f>SHEET('S5'!A1)</f><v>5</v></c></row><row r="5"><c r="B5"><f>SUM(Span)</f><v>7</v></c></row><row r="7"><c r="B7"><f>'S6'!A1*2</f><v>12</v></c></row></sheetData><conditionalFormatting sqref="C1"><cfRule type="expression" priority="1"><formula>'S3'!$A$1&gt;0</formula></cfRule></conditionalFormatting></worksheet>"#
    );
    let sheet = |index: u32| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1"><v>{index}</v></c></row></sheetData></worksheet>"#
        )
    };
    let parts = (2..=6).map(sheet).collect::<Vec<_>>();
    let mut entries = vec![
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", root),
        ("xl/workbook.xml", workbook.as_str()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships.as_str(),
        ),
        ("xl/worksheets/sheet1.xml", s1.as_str()),
    ];
    let names = (2..=6)
        .map(|index| format!("xl/worksheets/sheet{index}.xml"))
        .collect::<Vec<_>>();
    for (name, part) in names.iter().zip(&parts) {
        entries.push((name.as_str(), part.as_str()));
    }
    generated_package(&entries)
}
