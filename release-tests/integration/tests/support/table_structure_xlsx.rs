//! A generated package with the `Things` table Excel 16.114 converted and resized in its
//! measurements: C1:E5 on `S2`, columns `key`, `val`, and `dbl`, a totals row, and a calculated
//! `dbl` column stored as a shared formula in Excel's `[#This Row]` form. `S1` reads the table.

use super::generated_xlsx::generated_package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub fn generated_table_structure_fixture() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/></Types>"#;
    let root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheets><sheet name="S1" sheetId="1" r:id="rId1"/><sheet name="S2" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="ThingsVal">Things[val]</definedName></definedNames><calcPr calcId="191029"/></workbook>"#
    );
    let s1 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1"><f>SUM(Things[val])</f><v>6</v></c></row><row r="2"><c r="A2"><f>SUM(Things[[#Data],[val]:[dbl]])</f><v>18</v></c></row><row r="3"><c r="A3"><f>Things[[#Totals],[val]]</f><v>6</v></c></row><row r="4"><c r="A4"><f>ROWS(Things[#All])</f><v>5</v></c></row><row r="5"><c r="A5"><f>SUM(ThingsVal)</f><v>6</v></c></row><row r="6"><c r="A6"><f>SUM(Things[dbl])</f><v>12</v></c></row></sheetData></worksheet>"#
    );
    let text = |cell: &str, value: &str| {
        format!(r#"<c r="{cell}" t="inlineStr"><is><t>{value}</t></is></c>"#)
    };
    let s2 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheetData><row r="1">{}{}{}</row><row r="2">{}<c r="D2"><v>1</v></c><c r="E2"><f t="shared" ref="E2:E4" si="0">Things[[#This Row],[val]]*2</f><v>2</v></c></row><row r="3">{}<c r="D3"><v>2</v></c><c r="E3"><f t="shared" si="0"/><v>4</v></c></row><row r="4">{}<c r="D4"><v>3</v></c><c r="E4"><f t="shared" si="0"/><v>6</v></c></row><row r="5"><c r="D5"><f>SUBTOTAL(109,Things[val])</f><v>6</v></c></row></sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>"#,
        text("C1", "key"),
        text("D1", "val"),
        text("E1", "dbl"),
        text("C2", "a"),
        text("C3", "b"),
        text("C4", "c"),
    );
    let s2_relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/></Relationships>"#;
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="{MAIN}" id="1" name="Things" displayName="Things" ref="C1:E5" totalsRowCount="1"><autoFilter ref="C1:E4"/><tableColumns count="3"><tableColumn id="1" name="key"/><tableColumn id="2" name="val" totalsRowFunction="sum"/><tableColumn id="3" name="dbl"><calculatedColumnFormula>Things[[#This Row],[val]]*2</calculatedColumnFormula></tableColumn></tableColumns><tableStyleInfo name="TableStyleMedium2" showRowStripes="1"/></table>"#
    );
    generated_package(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", workbook_relationships),
        ("xl/worksheets/sheet1.xml", &s1),
        ("xl/worksheets/sheet2.xml", &s2),
        ("xl/worksheets/_rels/sheet2.xml.rels", s2_relationships),
        ("xl/tables/table1.xml", &table),
    ])
}
