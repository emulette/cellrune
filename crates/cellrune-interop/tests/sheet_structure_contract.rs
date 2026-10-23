use std::io::Write;

use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellDto, CellValueDto, EditBatchDto,
    EditBatchV2Dto, EditReceiptDto, EditReceiptV2Dto, InteropError, RangeRequestDto,
    TableChangeV2Dto, WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookSession,
    WritableCellValueDto,
};

fn apply(
    session: &mut WorkbookSession,
    changes: Vec<WorkbookChangeDto>,
) -> Result<EditReceiptDto, InteropError> {
    let revision = session.summary().semantic_revision;
    session.apply_changes(revision, EditBatchDto { changes })
}

fn apply_v2(
    session: &mut WorkbookSession,
    changes: Vec<WorkbookChangeV2Dto>,
) -> Result<EditReceiptV2Dto, InteropError> {
    let revision = session.summary().semantic_revision;
    session.apply_changes_v2(revision, EditBatchV2Dto { changes })
}

fn sheet_names(session: &WorkbookSession) -> Vec<(u32, String)> {
    session
        .summary()
        .sheets
        .into_iter()
        .map(|sheet| (sheet.id, sheet.name))
        .collect()
}

fn cell(session: &WorkbookSession, sheet: &str, address: &str) -> CellDto {
    session
        .read_range(&RangeRequestDto {
            sheet: sheet.to_owned(),
            start: address.to_owned(),
            end: address.to_owned(),
            offset: 0,
            limit: 1,
        })
        .expect("read cell")
        .cells
        .remove(0)
}

fn number(value: f64) -> WritableCellValueDto {
    WritableCellValueDto::Number { value }
}

fn text(value: &str) -> WritableCellValueDto {
    WritableCellValueDto::Text {
        value: value.to_owned(),
    }
}

/// `Sheet1`, `Data`, and `Last`, with `Data!B1` reading `Data!A1`.
fn sheets_session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session.add_sheet("Data").expect("Data");
    session.add_sheet("Last").expect("Last");
    session
        .set_value("Data", "A1", number(2.0))
        .expect("Data!A1");
    session
        .set_formula("Data", "B1", "=Data!A1*10", None)
        .expect("Data!B1");
    session
}

#[test]
fn a_batch_copies_and_moves_sheets_and_reaches_each_copy_by_its_predicted_name() {
    let mut session = sheets_session();
    let receipt = apply(
        &mut session,
        vec![
            WorkbookChangeDto::CopySheet {
                sheet: "data".to_owned(),
                position: 1,
            },
            WorkbookChangeDto::CopySheet {
                sheet: "Data".to_owned(),
                position: 4,
            },
            WorkbookChangeDto::SetValue {
                sheet: "data (2)".to_owned(),
                address: "A1".to_owned(),
                value: number(7.0),
            },
            WorkbookChangeDto::SetValue {
                sheet: "Data (3)".to_owned(),
                address: "A1".to_owned(),
                value: number(9.0),
            },
            WorkbookChangeDto::MoveSheet {
                sheet: "LAST".to_owned(),
                position: 0,
            },
        ],
    )
    .expect("copy, edit the copies, and move");

    assert_eq!(receipt.created_sheet_ids, [4, 5]);
    assert!(receipt.topology_changed);
    assert_eq!(
        sheet_names(&session),
        [
            (3, "Last".to_owned()),
            (1, "Sheet1".to_owned()),
            (4, "Data (2)".to_owned()),
            (2, "Data".to_owned()),
            (5, "Data (3)".to_owned()),
        ]
    );
    // A copy's reference to its source names the copy.
    assert_eq!(
        cell(&session, "Data (2)", "B1").formula.as_deref(),
        Some("='Data (2)'!A1*10")
    );
    for (sheet, value) in [("Data", 2.0), ("Data (2)", 7.0), ("Data (3)", 9.0)] {
        assert_eq!(
            cell(&session, sheet, "A1").source_value,
            CellValueDto::Number { value }
        );
    }

    let revision = session.summary().semantic_revision;
    let error = apply(
        &mut session,
        vec![WorkbookChangeDto::MoveSheet {
            sheet: "Data".to_owned(),
            position: 5,
        }],
    )
    .expect_err("position 5 is past the last of five tabs");
    assert_eq!(error.code(), "validation.sheet_position_out_of_range");
    assert_eq!(session.summary().semantic_revision, revision);
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

// The chartsheet holds the name a first copy of `Data` would take.
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Data" sheetId="1" r:id="rId1"/>
    <sheet name="Data (2)" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
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

#[test]
fn a_copy_skips_the_name_a_chartsheet_holds() {
    let mut session = chartsheet_session();
    let receipt = apply(
        &mut session,
        vec![
            WorkbookChangeDto::CopySheet {
                sheet: "Data".to_owned(),
                position: 2,
            },
            WorkbookChangeDto::SetValue {
                sheet: "Data (3)".to_owned(),
                address: "B1".to_owned(),
                value: number(3.0),
            },
        ],
    )
    .expect("the copy takes the next free name");

    assert_eq!(receipt.created_sheet_ids, [3]);
    assert_eq!(
        sheet_names(&session),
        [(1, "Data".to_owned()), (3, "Data (3)".to_owned())]
    );
    assert_eq!(
        cell(&session, "Data (3)", "B1").source_value,
        CellValueDto::Number { value: 3.0 }
    );
}

#[test]
fn a_batch_renames_a_defined_name_and_sets_and_removes_a_sheet_filter() {
    let mut session = sheets_session();
    apply(
        &mut session,
        vec![
            WorkbookChangeDto::SetDefinedName {
                name: "Rate".to_owned(),
                scope_sheet: None,
                formula: "=Data!$A$1".to_owned(),
                hidden: false,
            },
            WorkbookChangeDto::SetDefinedName {
                name: "Local".to_owned(),
                scope_sheet: Some("Data".to_owned()),
                formula: "=Data!$B$1".to_owned(),
                hidden: false,
            },
            WorkbookChangeDto::SetFormula {
                sheet: "Sheet1".to_owned(),
                address: "A1".to_owned(),
                formula: "=Rate*2".to_owned(),
                dynamic_range: None,
            },
            WorkbookChangeDto::SetFormula {
                sheet: "Data".to_owned(),
                address: "C1".to_owned(),
                formula: "=Local+1".to_owned(),
                dynamic_range: None,
            },
        ],
    )
    .expect("names and their uses");

    let receipt = apply(
        &mut session,
        vec![
            WorkbookChangeDto::RenameDefinedName {
                name: "rate".to_owned(),
                scope_sheet: None,
                new_name: "Price".to_owned(),
            },
            WorkbookChangeDto::RenameDefinedName {
                name: "Local".to_owned(),
                scope_sheet: Some("data".to_owned()),
                new_name: "Mine".to_owned(),
            },
            WorkbookChangeDto::SetSheetAutoFilter {
                sheet: "sheet1".to_owned(),
                range: Some("A1:C9".to_owned()),
            },
        ],
    )
    .expect("rename names and filter Sheet1");
    assert!(receipt.topology_changed);
    assert_eq!(
        cell(&session, "Sheet1", "A1").formula.as_deref(),
        Some("=Price*2")
    );
    assert_eq!(
        cell(&session, "Data", "C1").formula.as_deref(),
        Some("=Mine+1")
    );
    let auto_filter = |session: &WorkbookSession| {
        session
            .summary()
            .sheets
            .into_iter()
            .find(|sheet| sheet.name == "Sheet1")
            .expect("Sheet1")
            .auto_filter
    };
    assert_eq!(auto_filter(&session).as_deref(), Some("A1:C9"));

    apply(
        &mut session,
        vec![WorkbookChangeDto::SetSheetAutoFilter {
            sheet: "Sheet1".to_owned(),
            range: None,
        }],
    )
    .expect("remove the filter");
    assert_eq!(auto_filter(&session), None);

    let revision = session.summary().semantic_revision;
    let error = apply(
        &mut session,
        vec![WorkbookChangeDto::RenameDefinedName {
            name: "Rate".to_owned(),
            scope_sheet: None,
            new_name: "Other".to_owned(),
        }],
    )
    .expect_err("Rate was renamed");
    assert_eq!(error.code(), "validation.unknown_defined_name");
    assert_eq!(session.summary().semantic_revision, revision);
}

#[test]
fn a_v2_batch_creates_resizes_copies_and_converts_tables() {
    let mut session = WorkbookSession::create();
    for (address, value) in [
        ("A1", text("Item")),
        ("B1", text("Amount")),
        ("C1", text("Tax")),
        ("A2", text("Pen")),
        ("B2", number(2.0)),
        ("A3", text("Ink")),
        ("B3", number(3.0)),
        ("B4", number(4.0)),
    ] {
        session.set_value("Sheet1", address, value).expect("value");
    }

    let revision = session.summary().semantic_revision;
    let error = apply_v2(
        &mut session,
        vec![WorkbookChangeV2Dto::Table(TableChangeV2Dto::CreateTable {
            sheet: "Sheet1".to_owned(),
            range: "A1:B1".to_owned(),
            name: None,
        })],
    )
    .expect_err("a table needs a row below its header");
    assert_eq!(error.code(), "validation.table_range_invalid");
    assert_eq!(session.summary().semantic_revision, revision);

    let created = apply_v2(
        &mut session,
        vec![
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::CreateTable {
                sheet: "sheet1".to_owned(),
                range: "A1:B3".to_owned(),
                name: None,
            }),
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::SetTableRange {
                table_id: 1,
                range: "A1:C4".to_owned(),
            }),
            WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetFormula {
                sheet: "Sheet1".to_owned(),
                address: "E1".to_owned(),
                formula: "=SUM(Table1[Amount])".to_owned(),
                dynamic_range: None,
            }),
        ],
    )
    .expect("create and widen a table");
    assert_eq!(created.changed_table_ids, [1]);
    let table = session.summary().sheets[0].tables[0].clone();
    assert_eq!(
        (table.id, table.display_name.as_str(), table.range.as_str()),
        (1, "Table1", "A1:C4")
    );
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["Item", "Amount", "Tax"]
    );

    let copied = apply_v2(
        &mut session,
        vec![
            WorkbookChangeV2Dto::V1(WorkbookChangeDto::CopySheet {
                sheet: "Sheet1".to_owned(),
                position: 1,
            }),
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::CreateTable {
                sheet: "Sheet1 (2)".to_owned(),
                range: "G1:G3".to_owned(),
                name: Some("Extra".to_owned()),
            }),
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::ConvertTableToRange { table_id: 1 }),
        ],
    )
    .expect("copy the sheet, add a table to the copy, and convert the source table");
    assert_eq!(copied.receipt.created_sheet_ids, [2]);
    assert_eq!(copied.changed_table_ids, [2, 3]);
    let summary = session.summary();
    assert!(summary.sheets[0].tables.is_empty());
    assert_eq!(
        summary.sheets[1]
            .tables
            .iter()
            .map(|table| (table.id, table.range.as_str()))
            .collect::<Vec<_>>(),
        [(2, "A1:C4"), (3, "G1:G3")]
    );
    // The structured reference became a reference to the same cells.
    let total = cell(&session, "Sheet1", "E1");
    assert!(
        total
            .formula
            .as_deref()
            .is_some_and(|formula| formula.ends_with("!$B$2:$B$4)")),
        "{total:?}"
    );
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    assert_eq!(
        cell(&session, "Sheet1", "E1").calculated,
        Some(CalculationResultDto::Value {
            value: CellValueDto::Number { value: 9.0 }
        })
    );
}
