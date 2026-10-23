//! A generated two-sheet package whose parts hold every coordinate a structural grid edit of the
//! `Data` sheet moves, with `Report` reading `Data` through formulas, a conditional format, and a
//! data validation.

use super::generated_xlsx::generated_package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
  <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
  <Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>
  <Override PartName="/xl/threadedComments/threadedComment1.xml" ContentType="application/vnd.ms-excel.threadedcomments+xml"/>
  <Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>
</Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="0.000"/></numFmts>
  <fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="3">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>
  </cellXfs>
  <dxfs count="1"><dxf><font><b/></font></dxf></dxfs>
</styleSheet>"#;

/// `Data` holds the `Things` table over `A1:B5`, formulas below and beside it (a shared group
/// in `D3:D5`), a styled and taller row 2, column widths, a merged range, conditional formats and
/// data validations in the main part and the x14 extension, a hyperlink, a sheet auto-filter, a
/// sparkline, a page break, a frozen top row, a legacy note and a threaded comment on `B4`, and a
/// chart of `B2:B5`. `Report` reads `Data` by value, range, and whole column.
pub fn generated_grid_edit_fixture() -> Vec<u8> {
    generated_grid_edit_fixture_with(&[])
}

/// The fixture with extra parts and `Data` worksheet children, for refusal cases.
pub fn generated_grid_edit_fixture_with(extra: &[(&str, &str)]) -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <bookViews><workbookView xWindow="0" yWindow="0"/></bookViews>
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Report" sheetId="2" r:id="rId2"/>
  </sheets>
  <definedNames>
    <definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$L$1:$M$4</definedName>
    <definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$H$10</definedName>
    <definedName name="Total">Data!$B$2:$B$5</definedName>
  </definedNames>
  <calcPr calcId="191029"/>
</workbook>"#
    );
    let data = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <dimension ref="A1:N10"/>
  <sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="B3" sqref="B3"/></sheetView></sheetViews>
  <sheetFormatPr defaultRowHeight="15"/>
  <cols><col min="1" max="1" width="15" customWidth="1"/><col min="3" max="4" width="12" customWidth="1"/></cols>
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>key</t></is></c><c r="B1" t="inlineStr"><is><t>val</t></is></c><c r="D1"><f>SUM(B2:B5)</f><v>10</v></c><c r="L1" t="inlineStr"><is><t>k</t></is></c><c r="M1" t="inlineStr"><is><t>v</t></is></c></row>
    <row r="2" ht="25" customHeight="1"><c r="A2" t="inlineStr"><is><t>a</t></is></c><c r="B2" s="2"><v>1</v></c><c r="D2"><f>B3*2</f><v>4</v></c><c r="L2" t="inlineStr"><is><t>x</t></is></c><c r="M2"><v>1</v></c></row>
    <row r="3" s="1" customFormat="1"><c r="A3" t="inlineStr"><is><t>b</t></is></c><c r="B3"><v>2</v></c><c r="D3"><f t="shared" ref="D3:D5" si="0">B2+1</f><v>2</v></c><c r="L3" t="inlineStr"><is><t>y</t></is></c><c r="M3"><v>2</v></c></row>
    <row r="4"><c r="A4" t="inlineStr"><is><t>c</t></is></c><c r="B4"><v>3</v></c><c r="D4"><f t="shared" si="0"/><v>3</v></c><c r="L4" t="inlineStr"><is><t>z</t></is></c><c r="M4"><v>3</v></c></row>
    <row r="5"><c r="A5" t="inlineStr"><is><t>d</t></is></c><c r="B5"><v>4</v></c><c r="D5"><f t="shared" si="0"/><v>4</v></c></row>
    <row r="7"><c r="A7"><f>SUM(Things[val])</f><v>10</v></c><c r="B7"><f>ROW()</f><v>7</v></c></row>
  </sheetData>
  <autoFilter ref="L1:M4"><filterColumn colId="1"><filters><filter val="2"/></filters></filterColumn></autoFilter>
  <mergeCells count="1"><mergeCell ref="G2:H3"/></mergeCells>
  <conditionalFormatting sqref="B2:B5"><cfRule type="expression" dxfId="0" priority="1"><formula>B2&gt;2</formula></cfRule></conditionalFormatting>
  <dataValidations count="1"><dataValidation type="list" allowBlank="1" sqref="A2:A5"><formula1>$L$2:$L$4</formula1></dataValidation></dataValidations>
  <hyperlinks><hyperlink ref="G5" location="'Data'!A1" display="top"/></hyperlinks>
  <rowBreaks count="1" manualBreakCount="1"><brk id="4" max="16383" man="1"/></rowBreaks>
  <drawing r:id="rId2"/>
  <legacyDrawing r:id="rId3"/>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
  <extLst>
    <ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="{X14}"><x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm="{XM}"><x14:cfRule type="dataBar" id="{{00000000-000E-0000-0000-000001000000}}"><x14:dataBar minLength="0" maxLength="100"><x14:cfvo type="num"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type="formula"><xm:f>$B$5</xm:f></x14:cfvo></x14:dataBar></x14:cfRule><xm:sqref>B2:B5</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext>
    <ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="{X14}"><x14:dataValidations count="1" xmlns:xm="{XM}"><x14:dataValidation type="list" allowBlank="1"><x14:formula1><xm:f>Report!$C$1:$C$2</xm:f></x14:formula1><xm:sqref>C2:C3</xm:sqref></x14:dataValidation></x14:dataValidations></ext>
    <ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="{X14}"><x14:sparklineGroups xmlns:xm="{XM}"><x14:sparklineGroup><x14:colorSeries rgb="FF376092"/><x14:sparklines><x14:sparkline><xm:f>Data!B2:B5</xm:f><xm:sqref>N2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext>
  </extLst>
  {extra_data}
</worksheet>"#,
        extra_data = extra
            .iter()
            .find(|(name, _)| *name == "Data")
            .map_or("", |(_, xml)| *xml)
    );
    let data_relationships = relationships(&[
        ("rId1", "table", "../tables/table1.xml"),
        ("rId2", "drawing", "../drawings/drawing1.xml"),
        ("rId3", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
        ("rId4", "comments", "../comments1.xml"),
        (
            "rId5",
            "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment",
            "../threadedComments/threadedComment1.xml",
        ),
    ]);
    let report = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <sheetData>
    <row r="1"><c r="A1"><f>SUM(Data!B2:B5)</f><v>10</v></c><c r="C1" t="inlineStr"><is><t>p</t></is></c></row>
    <row r="2"><c r="A2"><f>Data!B3</f><v>2</v></c><c r="C2" t="inlineStr"><is><t>q</t></is></c></row>
    <row r="3"><c r="A3"><f>SUM(Data!B:B)</f><v>10</v></c></row>
    <row r="4"><c r="A4"><f>Data!D1+Total</f><v>11</v></c></row>
  </sheetData>
  <conditionalFormatting sqref="A1"><cfRule type="expression" dxfId="0" priority="1"><formula>Data!$B$3&gt;0</formula></cfRule></conditionalFormatting>
  <dataValidations count="1"><dataValidation type="list" allowBlank="1" sqref="B1"><formula1>Data!$A$2:$A$5</formula1></dataValidation></dataValidations>
  <hyperlinks><hyperlink ref="B2" location="Data!B3" display="value"/></hyperlinks>
</worksheet>"#
    );
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="{MAIN}" id="1" name="Things" displayName="Things" ref="A1:B5"><autoFilter ref="A1:B5"/><tableColumns count="2"><tableColumn id="1" name="key"/><tableColumn id="2" name="val"/></tableColumns><tableStyleInfo name="TableStyleMedium9" showRowStripes="1"/></table>"#
    );
    let drawing = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="{RELATIONSHIPS}"><xdr:twoCellAnchor><xdr:from><xdr:col>9</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>9</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#
    );
    let drawing_relationships = relationships(&[("rId1", "chart", "../charts/chart1.xml")]);
    let chart = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Data!$B$1</c:f></c:strRef></c:tx><c:cat><c:strRef><c:f>Data!$A$2:$A$5</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Data!$B$2:$B$5</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let vml = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype><v:shape id="_x0000_s1025" type="#_x0000_t202" style="position:absolute;visibility:hidden" fillcolor="infoBackground [80]"><v:textbox/><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>2, 15, 2, 10, 4, 15, 6, 4</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>3</x:Row><x:Column>1</x:Column></x:ClientData></v:shape></xml>"##;
    let comments = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<comments xmlns="{MAIN}"><authors><author>a</author></authors><commentList><comment ref="B4" authorId="0"><text><t>check</t></text></comment></commentList></comments>"#
    );
    let threaded = r#"<?xml version="1.0" encoding="UTF-8"?>
<ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"><threadedComment ref="B4" dT="2026-10-23T00:00:00.00" personId="{00000000-0000-0000-0000-000000000001}" id="{00000000-0000-0000-0000-000000000002}"><text>check</text></threadedComment></ThreadedComments>"#;
    let calc_chain = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<calcChain xmlns="{MAIN}"><c r="D1" i="1"/><c r="D2"/><c r="D3"/><c r="A1" i="2"/></calcChain>"#
    );
    let mut entries = vec![
        ("[Content_Types].xml", CONTENT_TYPES.to_owned()),
        ("_rels/.rels", ROOT_RELATIONSHIPS.to_owned()),
        ("xl/workbook.xml", workbook),
        (
            "xl/_rels/workbook.xml.rels",
            WORKBOOK_RELATIONSHIPS.to_owned(),
        ),
        ("xl/styles.xml", STYLES.to_owned()),
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/_rels/sheet1.xml.rels", data_relationships),
        ("xl/worksheets/sheet2.xml", report),
        ("xl/tables/table1.xml", table),
        ("xl/drawings/drawing1.xml", drawing),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_relationships),
        ("xl/charts/chart1.xml", chart.to_owned()),
        ("xl/drawings/vmlDrawing1.vml", vml.to_owned()),
        ("xl/comments1.xml", comments),
        (
            "xl/threadedComments/threadedComment1.xml",
            threaded.to_owned(),
        ),
        ("xl/calcChain.xml", calc_chain),
    ];
    for (name, contents) in extra {
        if *name != "Data" {
            if let Some(entry) = entries.iter_mut().find(|(existing, _)| existing == name) {
                entry.1 = (*contents).to_owned();
            } else {
                entries.push((name, (*contents).to_owned()));
            }
        }
    }
    let entries = entries
        .iter()
        .map(|(name, contents)| (*name, contents.as_str()))
        .collect::<Vec<_>>();
    generated_package(&entries)
}

fn relationships(entries: &[(&str, &str, &str)]) -> String {
    let entries = entries
        .iter()
        .map(|(id, kind, target)| {
            let kind = if kind.starts_with("http") {
                (*kind).to_owned()
            } else {
                format!(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"
                )
            };
            format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"/>"#)
        })
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{entries}</Relationships>"#
    )
}
