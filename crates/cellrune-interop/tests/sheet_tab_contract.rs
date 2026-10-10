use std::io::Write;

use cellrune_interop::{
    DefinedNameInspectionRequestDto, DefinedNameInspectionResultDto, EditBatchDto, EditReceiptDto,
    InteropError, WorkbookChangeDto, WorkbookSession, WritableCellValueDto,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

// The chartsheet holds the highest sheetId, one above every worksheet's.
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Chart" sheetId="3" r:id="rId3"/>
    <sheet name="Summary" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
</Relationships>"#;

const WORKSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
</worksheet>"#;

const CHARTSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"/></sheetViews></chartsheet>"#;

fn chartsheet_session() -> WorkbookSession {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELATIONSHIPS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
            ("xl/worksheets/sheet1.xml", WORKSHEET),
            ("xl/worksheets/sheet2.xml", WORKSHEET),
            ("xl/chartsheets/sheet1.xml", CHARTSHEET),
        ] {
            writer.start_file(name, options).expect("fixture part");
            writer
                .write_all(contents.as_bytes())
                .expect("fixture bytes");
        }
        writer.finish().expect("fixture archive");
    }
    WorkbookSession::open_bytes(buffer.get_ref()).expect("chartsheet fixture must open")
}

fn apply(
    session: &mut WorkbookSession,
    changes: Vec<WorkbookChangeDto>,
) -> Result<EditReceiptDto, InteropError> {
    let revision = session.summary().semantic_revision;
    session.apply_changes(revision, EditBatchDto { changes })
}

#[test]
fn a_batch_targets_its_added_sheet_by_the_id_core_assigns_past_a_chartsheet() {
    let mut session = chartsheet_session();
    let receipt = apply(
        &mut session,
        vec![
            WorkbookChangeDto::AddSheet {
                name: "Added".to_owned(),
            },
            WorkbookChangeDto::SetValue {
                sheet: "Added".to_owned(),
                address: "A1".to_owned(),
                value: WritableCellValueDto::Number { value: 42.0 },
            },
            WorkbookChangeDto::SetDefinedName {
                name: "AddedLocal".to_owned(),
                scope_sheet: Some("Added".to_owned()),
                formula: "=Added!A1".to_owned(),
                hidden: false,
            },
        ],
    )
    .expect("one batch must add a sheet and edit it by name");

    assert_eq!(receipt.created_sheet_ids, vec![4]);
    let summary = session.summary();
    let added = summary
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Added")
        .expect("added sheet");
    assert_eq!((added.id, added.cell_count), (4, 1));

    let inspect = |current_sheet: &str| {
        session
            .inspect_defined_name(&DefinedNameInspectionRequestDto {
                name: "AddedLocal".to_owned(),
                current_sheet: Some(current_sheet.to_owned()),
            })
            .expect("inspection must succeed")
            .result
    };
    assert!(matches!(
        inspect("Added"),
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id: 4,
            ref sheet_name,
            ref range,
        } if sheet_name == "Added" && range == "A1:A1"
    ));
    assert_eq!(inspect("Data"), DefinedNameInspectionResultDto::NotFound);
}
