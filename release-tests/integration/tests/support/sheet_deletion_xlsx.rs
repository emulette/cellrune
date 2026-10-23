//! A generated four-sheet package whose parts reach each sheet in every way a sheet deletion
//! handles, after the workbook Excel 16.114 saved for its sheet deletion measurements.

use super::generated_xlsx::generated_package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet4.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/tables/table2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
  <Override PartName="/xl/drawings/drawing2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
  <Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
  <Override PartName="/xl/charts/chart2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
  <Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/>
  <Override PartName="/xl/pivotTables/pivotTable2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheRecords1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheDefinition2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>
  <Override PartName="/xl/pivotCache/pivotCacheRecords2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/>
  <Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>
  <Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet4.xml"/>
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/>
  <Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition2.xml"/>
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>
</Relationships>"#;

/// Sheets `S1` through `S4`:
///
/// - `S1` holds the probe formulas, conditional formats, data validations, a hyperlink, and
///   sparklines that read `S2`, a drawing with a chart of `S2` and a shared image, and the pivot
///   table of the cache whose source is `S3`.
/// - `S2` holds the `Things` table, a drawing with its own chart and the shared image, legacy
///   comments, and the pivot table of the cache whose source is `S2` itself.
/// - `S3` holds the `Calc` table, whose calculated column reads `S2`.
/// - `S4` holds two 3-D and plain references.
pub fn generated_sheet_deletion_fixture() -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <bookViews><workbookView xWindow="0" yWindow="0" firstSheet="1" activeTab="2"/></bookViews>
  <sheets>
    <sheet name="S1" sheetId="1" r:id="rId1"/>
    <sheet name="S2" sheetId="2" r:id="rId2"/>
    <sheet name="S3" sheetId="3" r:id="rId3"/>
    <sheet name="S4" sheetId="4" r:id="rId4"/>
  </sheets>
  <definedNames>
    <definedName name="LocalS1" localSheetId="0">'S2'!$A$1</definedName>
    <definedName name="LocalS2" localSheetId="1">'S2'!$A$1</definedName>
    <definedName name="LocalS3" localSheetId="2">'S3'!$A$1</definedName>
    <definedName name="LocalS4" localSheetId="3">'S4'!$A$1</definedName>
    <definedName name="Unused2" localSheetId="1">'S2'!$A$2</definedName>
    <definedName name="Wb_S2">'S2'!$A$1</definedName>
    <definedName name="Wb_S2S4">'S2:S4'!$A$1</definedName>
    <definedName name="Wb_S3">'S3'!$A$1</definedName>
    <definedName name="Wb_Used">'S2'!$A$1:$A$3</definedName>
  </definedNames>
  <calcPr calcId="191029"/>
  <pivotCaches><pivotCache cacheId="1" r:id="rId5"/><pivotCache cacheId="2" r:id="rId6"/></pivotCaches>
</workbook>"#
    );
    let s1 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <sheetData>
    <row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>'S2'!A1</f><v>2</v></c></row>
    <row r="2"><c r="B2"><f>SUM('S1:S3'!A1)</f><v>6</v></c></row>
    <row r="3"><c r="B3"><f>SUM('S2:S4'!A1)</f><v>9</v></c></row>
    <row r="4"><c r="B4"><f>SUM('S2:S3'!A1)</f><v>5</v></c></row>
    <row r="5"><c r="B5"><f>SUM(Wb_Used)</f><v>222</v></c></row>
    <row r="6"><c r="B6"><f>SUM(Things[val])</f><v>3</v></c></row>
    <row r="7"><c r="B7"><f>'S3'!LocalS3</f><v>3</v></c></row>
    <row r="8"><c r="B8"><f>'S2'!LocalS2</f><v>2</v></c></row>
    <row r="9"><c r="B9"><f>Wb_S3</f><v>3</v></c></row>
    <row r="10"><c r="B10"><f>'S3'!A1</f><v>3</v></c></row>
    <row r="11"><c r="B11"><f ca="1">INDIRECT("'S2'!A1")</f><v>2</v></c></row>
    <row r="12"><c r="B12"><f>SUM('S2'!A1:A3)</f><v>222</v></c></row>
    <row r="13"><c r="B13"><f>'S2'!A1+'S3'!A1</f><v>5</v></c></row>
    <row r="14"><c r="B14"><f>LocalS1</f><v>2</v></c></row>
    <row r="15"><c r="B15"><f>SHEETS()</f><v>4</v></c></row>
  </sheetData>
  <conditionalFormatting sqref="C1"><cfRule type="expression" priority="1"><formula>'S2'!$A$1&gt;0</formula></cfRule></conditionalFormatting>
  <conditionalFormatting sqref="C2"><cfRule type="colorScale" priority="2"><colorScale><cfvo type="formula" val="'S2'!$A$1"/><cfvo type="max"/><color rgb="FFF8696B"/><color rgb="FF63BE7B"/></colorScale></cfRule></conditionalFormatting>
  <dataValidations count="1"><dataValidation type="list" allowBlank="1" sqref="D1"><formula1>'S2'!$A$1:$A$3</formula1></dataValidation></dataValidations>
  <hyperlinks><hyperlink ref="E1" location="'S2'!A1" display="go"/></hyperlinks>
  <drawing r:id="rId1"/>
  <extLst>
    <ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="expression" priority="3" id="{{00000000-000E-0000-0000-000001000000}}"><xm:f>'S2'!$A$2&gt;1</xm:f><x14:dxf/></x14:cfRule><xm:sqref>C3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext>
    <ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataValidations count="1" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidation type="list" allowBlank="1"><x14:formula1><xm:f>'S2'!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>D2</xm:sqref></x14:dataValidation></x14:dataValidations></ext>
    <ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroup><x14:colorSeries rgb="FF376092"/><x14:sparklines><x14:sparkline><xm:f>'S2'!A1:A3</xm:f><xm:sqref>F1</xm:sqref></x14:sparkline><x14:sparkline><xm:f>'S3'!A1:A1</xm:f><xm:sqref>F2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext>
  </extLst>
</worksheet>"#
    );
    let s1_relationships = relationships(&[
        ("rId1", "drawing", "../drawings/drawing1.xml"),
        ("rId2", "pivotTable", "../pivotTables/pivotTable2.xml"),
    ]);
    let s2 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <sheetData>
    <row r="1"><c r="A1"><v>2</v></c><c r="C1" t="inlineStr"><is><t>key</t></is></c><c r="D1" t="inlineStr"><is><t>val</t></is></c><c r="F1" t="inlineStr"><is><t>k</t></is></c><c r="G1" t="inlineStr"><is><t>v</t></is></c></row>
    <row r="2"><c r="A2"><v>20</v></c><c r="C2" t="inlineStr"><is><t>a</t></is></c><c r="D2"><v>1</v></c><c r="F2" t="inlineStr"><is><t>x</t></is></c><c r="G2"><v>10</v></c></row>
    <row r="3"><c r="A3"><v>200</v></c><c r="C3" t="inlineStr"><is><t>b</t></is></c><c r="D3"><v>2</v></c><c r="F3" t="inlineStr"><is><t>y</t></is></c><c r="G3"><v>20</v></c></row>
  </sheetData>
  <drawing r:id="rId2"/>
  <legacyDrawing r:id="rId4"/>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#
    );
    let s2_relationships = relationships(&[
        ("rId1", "table", "../tables/table1.xml"),
        ("rId2", "drawing", "../drawings/drawing2.xml"),
        ("rId3", "pivotTable", "../pivotTables/pivotTable1.xml"),
        ("rId4", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
        ("rId5", "comments", "../comments1.xml"),
    ]);
    let s3 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}">
  <sheetData>
    <row r="1"><c r="A1"><v>3</v></c><c r="B1"><f>'S2'!A1</f><v>2</v></c><c r="D1" t="inlineStr"><is><t>x</t></is></c></row>
    <row r="2"><c r="D2"><f>'S2'!$A$1+1</f><v>3</v></c></row>
  </sheetData>
  <tableParts count="1"><tablePart r:id="rId1"/></tableParts>
</worksheet>"#
    );
    let s3_relationships = relationships(&[("rId1", "table", "../tables/table2.xml")]);
    let s4 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="{MAIN}">
  <sheetData>
    <row r="1"><c r="A1"><v>4</v></c><c r="B1"><f>SUM('S1:S3'!A1)</f><v>6</v></c></row>
    <row r="2"><c r="B2"><f>'S1'!A1</f><v>1</v></c></row>
  </sheetData>
</worksheet>"#
    );
    let things = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="{MAIN}" id="1" name="Things" displayName="Things" ref="C1:D3"><autoFilter ref="C1:D3"/><tableColumns count="2"><tableColumn id="1" name="key"/><tableColumn id="2" name="val"/></tableColumns><tableStyleInfo name="TableStyleMedium9" showRowStripes="1"/></table>"#
    );
    let calc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="{MAIN}" id="2" name="Calc" displayName="Calc" ref="D1:D2"><autoFilter ref="D1:D2"/><tableColumns count="1"><tableColumn id="1" name="x"><calculatedColumnFormula>'S2'!$A$1+1</calculatedColumnFormula></tableColumn></tableColumns></table>"#
    );
    let drawing = |chart: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="{RELATIONSHIPS}">
  <xdr:twoCellAnchor><xdr:from><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>14</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="{chart}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>
  <xdr:oneCellAnchor><xdr:from><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="9525" cy="9525"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="3" name="Picture"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>
</xdr:wsDr>"#
        )
    };
    let drawing_relationships = |chart: &str| {
        relationships(&[
            ("rId1", "chart", &format!("../charts/{chart}.xml")),
            ("rId2", "image", "../media/image1.png"),
        ])
    };
    let chart = |series: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>'S1'!$A$1</c:f></c:strRef></c:tx><c:val><c:numRef><c:f>{series}</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        )
    };
    let pivot_table = |name: &str, cache: u32| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<pivotTableDefinition xmlns="{MAIN}" name="{name}" cacheId="{cache}" dataCaption="Values"><location ref="J1:K3" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/></pivotTableDefinition>"#
        )
    };
    let pivot_cache = |source: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<pivotCacheDefinition xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}" r:id="rId1" recordCount="0"><cacheSource type="worksheet">{source}</cacheSource><cacheFields count="1"><cacheField name="k" numFmtId="0"><sharedItems/></cacheField></cacheFields></pivotCacheDefinition>"#
        )
    };
    let pivot_records = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<pivotCacheRecords xmlns="{MAIN}" count="0"/>"#
    );
    let comments = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<comments xmlns="{MAIN}"><authors><author>a</author></authors><commentList><comment ref="A1" authorId="0"><text><t>note</t></text></comment></commentList></comments>"#
    );
    let calc_chain = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<calcChain xmlns="{MAIN}"><c r="B1" i="4"/><c r="B2"/><c r="B1" i="1"/><c r="B1" i="3"/></calcChain>"#
    );
    let app = r#"<?xml version="1.0" encoding="UTF-8"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><TitlesOfParts><vt:vector size="4" baseType="lpstr"><vt:lpstr>S1</vt:lpstr><vt:lpstr>S2</vt:lpstr><vt:lpstr>S3</vt:lpstr><vt:lpstr>S4</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#;
    let pivot_relationships =
        |target: &str| relationships(&[("rId1", "pivotCacheDefinition", target)]);
    let records_relationships =
        |target: &str| relationships(&[("rId1", "pivotCacheRecords", target)]);
    generated_package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("docProps/app.xml", app),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/worksheets/sheet1.xml", &s1),
        ("xl/worksheets/_rels/sheet1.xml.rels", &s1_relationships),
        ("xl/worksheets/sheet2.xml", &s2),
        ("xl/worksheets/_rels/sheet2.xml.rels", &s2_relationships),
        ("xl/worksheets/sheet3.xml", &s3),
        ("xl/worksheets/_rels/sheet3.xml.rels", &s3_relationships),
        ("xl/worksheets/sheet4.xml", &s4),
        ("xl/tables/table1.xml", &things),
        ("xl/tables/table2.xml", &calc),
        ("xl/drawings/drawing1.xml", &drawing("Chart 1")),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            &drawing_relationships("chart1"),
        ),
        ("xl/drawings/drawing2.xml", &drawing("Chart 2")),
        (
            "xl/drawings/_rels/drawing2.xml.rels",
            &drawing_relationships("chart2"),
        ),
        (
            "xl/drawings/vmlDrawing1.vml",
            "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\"/>",
        ),
        ("xl/charts/chart1.xml", &chart("'S2'!$A$1:$A$3")),
        ("xl/charts/chart2.xml", &chart("'S2'!$A$1:$A$3")),
        ("xl/media/image1.png", "png"),
        (
            "xl/pivotTables/pivotTable1.xml",
            &pivot_table("PivotTable1", 1),
        ),
        (
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            &pivot_relationships("../pivotCache/pivotCacheDefinition1.xml"),
        ),
        (
            "xl/pivotTables/pivotTable2.xml",
            &pivot_table("PivotTable2", 2),
        ),
        (
            "xl/pivotTables/_rels/pivotTable2.xml.rels",
            &pivot_relationships("../pivotCache/pivotCacheDefinition2.xml"),
        ),
        (
            "xl/pivotCache/pivotCacheDefinition1.xml",
            &pivot_cache(r#"<worksheetSource ref="F1:G3" sheet="S2"/>"#),
        ),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            &records_relationships("pivotCacheRecords1.xml"),
        ),
        ("xl/pivotCache/pivotCacheRecords1.xml", &pivot_records),
        (
            "xl/pivotCache/pivotCacheDefinition2.xml",
            &pivot_cache(r#"<worksheetSource ref="D1:D2" sheet="S3"/>"#),
        ),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition2.xml.rels",
            &records_relationships("pivotCacheRecords2.xml"),
        ),
        ("xl/pivotCache/pivotCacheRecords2.xml", &pivot_records),
        ("xl/comments1.xml", &comments),
        ("xl/calcChain.xml", &calc_chain),
    ])
}

fn relationships(entries: &[(&str, &str, &str)]) -> String {
    let entries = entries
        .iter()
        .map(|(id, kind, target)| {
            format!(
                r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
            )
        })
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{entries}</Relationships>"#
    )
}
