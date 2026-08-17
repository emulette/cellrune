use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellValueDto, INTEROP_SCHEMA_VERSION,
    InteropErrorKind, MAX_PAGE_SIZE, RangeRequestDto, RecalculationModeDto, WorkbookFingerprintDto,
    WorkbookSession, WritableCellValueDto, WriteOptionsDto, function_catalog,
};

#[test]
fn typed_values_edits_and_stable_errors_cover_the_public_boundary() {
    let mut session = WorkbookSession::create();
    let summary = session.summary();
    assert_eq!(summary.schema_version, INTEROP_SCHEMA_VERSION);
    assert_eq!(summary.semantic_revision, 0);
    assert_eq!(summary.fingerprint.schema_version, 7);
    assert_eq!(summary.fingerprint.digest_hex.len(), 64);
    assert!(
        summary
            .fingerprint
            .digest_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let serialized = serde_json::to_string(&summary.fingerprint).expect("fingerprint JSON");
    let round_trip: WorkbookFingerprintDto =
        serde_json::from_str(&serialized).expect("fingerprint JSON round trip");
    assert_eq!(round_trip, summary.fingerprint);
    assert!(!summary.document_backed);
    assert_eq!(summary.document_kind, "new_xlsx");
    assert_eq!(summary.date_system, "excel_1900");
    assert_eq!(summary.sheets.len(), 1);
    assert_eq!(summary.sheets[0].id, 1);
    assert_eq!(summary.sheets[0].name, "Sheet1");
    assert_eq!(summary.sheets[0].visibility, "visible");
    assert_eq!(summary.sheets[0].cell_count, 0);
    assert_eq!(summary.sheets[0].used_range, None);

    assert_eq!(session.add_sheet("Inputs").expect("sheet add must work"), 2);
    session
        .rename_sheet("Inputs", "Typed Values")
        .expect("sheet rename must work");
    let values = [
        WritableCellValueDto::Blank,
        WritableCellValueDto::Number { value: 7.5 },
        WritableCellValueDto::Text {
            value: "hello".to_owned(),
        },
        WritableCellValueDto::Logical { value: true },
    ];
    for (index, value) in values.into_iter().enumerate() {
        session
            .set_value("Typed Values", &format!("A{}", index + 1), value)
            .expect("typed value must be accepted");
    }

    let excel_errors = [
        "#NULL!",
        "#DIV/0!",
        "#VALUE!",
        "#REF!",
        "#NAME?",
        "#NUM!",
        "#N/A",
        "#GETTING_DATA",
        "#SPILL!",
        "#CALC!",
    ];
    for (index, value) in excel_errors.into_iter().enumerate() {
        session
            .set_value(
                "Typed Values",
                &format!("B{}", index + 1),
                WritableCellValueDto::Error {
                    value: value.to_owned(),
                },
            )
            .expect("canonical Excel error must be accepted");
    }

    let page = session
        .read_range(&RangeRequestDto {
            sheet: "Typed Values".to_owned(),
            start: "A1".to_owned(),
            end: "B10".to_owned(),
            offset: 0,
            limit: 20,
        })
        .expect("typed values must be readable");
    assert_eq!(page.start, "A1");
    assert_eq!(page.end, "B10");
    assert_eq!(page.total_cells, 20);
    assert_eq!(page.cells[0].source_value, CellValueDto::Blank);
    assert_eq!(
        page.cells[2].source_value,
        CellValueDto::Number { value: 7.5 }
    );
    assert_eq!(
        page.cells[4].source_value,
        CellValueDto::Text {
            value: "hello".to_owned()
        }
    );
    assert_eq!(
        page.cells[6].source_value,
        CellValueDto::Logical { value: true }
    );
    for (index, expected) in excel_errors.into_iter().enumerate() {
        assert_eq!(
            page.cells[index * 2 + 1].source_value,
            CellValueDto::Error {
                value: expected.to_owned()
            }
        );
    }
    assert_eq!(
        session.summary().sheets[1].used_range.as_deref(),
        Some("A1:B10")
    );

    assert!(
        session
            .clear_cell("Typed Values", "A2")
            .expect("existing cell clear must work")
    );
    assert!(
        !session
            .clear_cell("Typed Values", "A2")
            .expect("missing cell clear must work")
    );

    let unknown_sheet = session
        .set_value("Missing", "A1", WritableCellValueDto::Number { value: 1.0 })
        .expect_err("unknown sheet must fail");
    assert_eq!(unknown_sheet.kind(), InteropErrorKind::Input);
    assert_eq!(unknown_sheet.code(), "interop.sheet.not_found");
    assert_eq!(
        unknown_sheet.message(),
        "workbook does not contain the requested sheet"
    );
    assert_eq!(
        unknown_sheet.to_string(),
        "interop.sheet.not_found: workbook does not contain the requested sheet"
    );

    let invalid_error = session
        .set_value(
            "Typed Values",
            "A20",
            WritableCellValueDto::Error {
                value: "#UNKNOWN!".to_owned(),
            },
        )
        .expect_err("unknown Excel error must fail");
    assert_eq!(invalid_error.kind(), InteropErrorKind::Input);
    assert_eq!(invalid_error.code(), "interop.value.excel_error_invalid");
    assert_eq!(invalid_error.details().detail.as_deref(), Some("#UNKNOWN!"));
    assert!(invalid_error.to_string().ends_with(": #UNKNOWN!"));

    assert!(
        serde_json::from_str::<WritableCellValueDto>(r#"{"kind":"unsupported"}"#).is_err(),
        "the output-only sentinel must not deserialize as a writable value"
    );
}

#[test]
fn v0_1_15_fixed_income_crosses_capability_usage_and_recalculation_boundaries() {
    let mut session = WorkbookSession::create();
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 0.05 })
        .expect("fixed-income input must be accepted");
    session
        .set_formula(
            "Sheet1",
            "B1",
            "=ACCRINT(DATE(2024,1,1),DATE(2024,7,1),DATE(2025,1,1),A1,1000,2)",
            None,
        )
        .expect("fixed-income formula must be accepted");

    let capabilities = session
        .capabilities(CalculationOptionsDto::default(), 0, 1)
        .expect("fixed-income capability scan must work");
    assert_eq!(capabilities.formula_count, 1);
    assert_eq!(capabilities.supported_count, 1);
    assert!(capabilities.entries[0].supported);
    assert!(capabilities.entries[0].issue_codes.is_empty());

    let usage = session.function_usage();
    let accrint = usage
        .entries
        .iter()
        .find(|entry| entry.name == "ACCRINT")
        .expect("ACCRINT usage must be present");
    assert!(accrint.supported);
    assert_eq!(accrint.call_count, 1);
    assert_eq!(accrint.formula_count, 1);
    assert_eq!(accrint.sample_cells[0].address, "B1");

    session
        .recalculate(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .expect("fixed-income full calculation must work");
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 0.06 })
        .expect("fixed-income input edit must be accepted");
    let delta = session
        .recalculate(
            RecalculationModeDto::Incremental,
            CalculationOptionsDto::default(),
        )
        .expect("fixed-income incremental calculation must work");
    assert_eq!(delta.mode, "incremental");
    assert_eq!(delta.dirty_count, 1);
    assert_eq!(delta.evaluated_count, 1);
    assert_eq!(delta.changed_cells.len(), 1);
    assert_eq!(delta.changed_cells[0].cell.address, "B1");

    let result = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "B1".to_owned(),
            end: "B1".to_owned(),
            offset: 0,
            limit: 1,
        })
        .expect("fixed-income result must be readable");
    assert_eq!(
        result.cells[0].calculated,
        Some(CalculationResultDto::Value {
            value: CellValueDto::Number { value: 60.0 },
        })
    );
}

#[test]
fn deterministic_calculation_options_and_edits_invalidate_saved_state() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=TODAY()", None)
        .expect("TODAY formula must be accepted");
    session
        .set_formula("Sheet1", "A2", "=NOW()", None)
        .expect("NOW formula must be accepted");
    let report = session
        .calculate(CalculationOptionsDto {
            today_serial: Some(45_000.0),
            now_serial: Some(45_000.25),
            ..CalculationOptionsDto::default()
        })
        .expect("deterministic inputs must calculate");
    assert_eq!(report.value_count, 2);
    assert_eq!(report.unavailable_count, 0);

    let page = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "A2".to_owned(),
            offset: 0,
            limit: 2,
        })
        .expect("calculated values must be readable");
    assert_eq!(
        page.cells[0].calculated,
        Some(CalculationResultDto::Value {
            value: CellValueDto::Number { value: 45_000.0 }
        })
    );
    assert_eq!(
        page.cells[1].calculated,
        Some(CalculationResultDto::Value {
            value: CellValueDto::Number { value: 45_000.25 }
        })
    );

    session
        .set_value("Sheet1", "B1", WritableCellValueDto::Number { value: 1.0 })
        .expect("edit must succeed");
    let stale_page = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "A2".to_owned(),
            offset: 0,
            limit: 2,
        })
        .expect("source cells must remain readable after an edit");
    assert!(
        stale_page
            .cells
            .iter()
            .all(|cell| cell.calculated.is_none()),
        "an internally retained prior calculation must not cross the interop revision boundary"
    );
    assert_eq!(
        session
            .calculation_report()
            .expect_err("a stale calculation must not produce a current report")
            .code(),
        "interop.calculation.required"
    );
    let state_error = session
        .save_bytes(WriteOptionsDto::default())
        .expect_err("editing must invalidate the retained calculation");
    assert_eq!(state_error.kind(), InteropErrorKind::State);
    assert_eq!(state_error.code(), "interop.calculation.required");
    let stale_path = unique_test_path();
    let path_state_error = session
        .save_path(&stale_path, WriteOptionsDto::default())
        .expect_err("path save must reject an internally retained stale calculation");
    assert_eq!(path_state_error.kind(), InteropErrorKind::State);
    assert_eq!(path_state_error.code(), "interop.calculation.required");
    assert!(!stale_path.exists());

    let validation_error = session
        .calculate(CalculationOptionsDto {
            today_serial: Some(f64::NAN),
            now_serial: None,
            ..CalculationOptionsDto::default()
        })
        .expect_err("non-finite deterministic input must fail");
    assert_eq!(validation_error.kind(), InteropErrorKind::Validation);
    assert_eq!(validation_error.code(), "validation.non_finite_number");
    assert_eq!(
        validation_error.details().source_code.as_deref(),
        Some("validation.non_finite_number")
    );
}

#[test]
fn capability_usage_catalog_and_incomplete_write_contracts_are_explicit() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=SUM(1,2)", None)
        .expect("supported formula must be accepted");
    session
        .set_formula("Sheet1", "A2", "=MYSTERY(1)", None)
        .expect("unsupported formula must remain inspectable");

    let first = session
        .capabilities(CalculationOptionsDto::default(), 0, 1)
        .expect("first capability page must work");
    assert_eq!(first.formula_count, 2);
    assert_eq!(first.supported_count, 1);
    assert_eq!(first.entries.len(), 1);
    assert!(first.entries[0].supported);
    assert_eq!(first.next_offset, Some(1));

    let second = session
        .capabilities(CalculationOptionsDto::default(), 1, 1)
        .expect("second capability page must work");
    assert_eq!(second.entries.len(), 1);
    assert!(!second.entries[0].supported);
    assert!(!second.entries[0].issue_codes.is_empty());
    assert_eq!(second.next_offset, None);

    let empty = session
        .capabilities(CalculationOptionsDto::default(), 2, 0)
        .expect("an end offset must return an empty capability page");
    assert!(empty.entries.is_empty());
    assert_eq!(empty.next_offset, None);
    assert_eq!(
        session
            .capabilities(CalculationOptionsDto::default(), 3, 1)
            .expect_err("an excessive capability offset must fail")
            .code(),
        "interop.page.offset_out_of_range"
    );
    assert_eq!(
        session
            .capabilities(CalculationOptionsDto::default(), 0, MAX_PAGE_SIZE + 1)
            .expect_err("an excessive capability page must fail")
            .code(),
        "interop.page.limit_exceeded"
    );

    let usage = session.function_usage();
    assert_eq!(usage.formula_count, 2);
    assert_eq!(usage.parsed_formula_count, 2);
    assert_eq!(usage.unparsed_formula_count, 0);
    let sum = usage
        .entries
        .iter()
        .find(|entry| entry.name == "SUM")
        .expect("SUM usage must be present");
    assert!(sum.supported);
    assert_eq!(sum.call_count, 1);
    assert_eq!(sum.formula_count, 1);
    assert_eq!(sum.sample_cells[0].sheet_name, "Sheet1");
    assert_eq!(sum.sample_cells[0].address, "A1");
    let mystery = usage
        .entries
        .iter()
        .find(|entry| entry.name == "MYSTERY")
        .expect("unsupported usage must be present");
    assert!(!mystery.supported);

    let catalog = function_catalog();
    assert_eq!(catalog.schema_version, INTEROP_SCHEMA_VERSION);
    let rust_catalog = cellrune::supported_function_catalog();
    assert_eq!(catalog.entries.len(), rust_catalog.len());
    for (interop, rust) in catalog.entries.iter().zip(&rust_catalog) {
        assert_eq!(interop.name, rust.name());
        assert_eq!(interop.canonical_name, rust.canonical_name());
        assert_eq!(interop.alias, rust.is_alias());
        assert_eq!(interop.returns_array, rust.returns_array());
        assert_eq!(interop.official, rust.is_official());
    }
    let sum_catalog = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "SUM")
        .expect("SUM must be cataloged");
    assert_eq!(sum_catalog.canonical_name, "SUM");
    assert!(!sum_catalog.alias);
    assert!(!sum_catalog.returns_array);
    assert!(sum_catalog.official);

    let report = session
        .calculate(CalculationOptionsDto::default())
        .expect("calculation with explicit unavailability must complete");
    assert_eq!(report.value_count, 1);
    assert_eq!(report.unavailable_count, 1);
    let strict_error = session
        .save_bytes(WriteOptionsDto::default())
        .expect_err("strict save must reject an unavailable result");
    assert_eq!(strict_error.kind(), InteropErrorKind::Write);

    let (_, invalidated) = session
        .save_bytes(WriteOptionsDto {
            invalidate_unavailable: true,
            replace_existing: false,
        })
        .expect("explicit invalidation must produce a verified package");
    assert!(!invalidated.complete);
    assert_eq!(invalidated.policy, "invalidate_unavailable");
    assert_eq!(invalidated.materialized_count, 1);
    assert_eq!(invalidated.invalidated_cells.len(), 1);
    assert_eq!(invalidated.invalidated_cells[0].address, "A2");
    assert_eq!(invalidated.output_sha256.len(), 64);
    assert!(
        invalidated
            .output_sha256
            .bytes()
            .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) })
    );
}

#[test]
fn path_open_and_save_respect_destination_replacement() {
    let path = unique_test_path();
    let mut session = WorkbookSession::create();
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 3.0 })
        .expect("test workbook edit must succeed");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("test workbook must calculate");
    let first = session
        .save_path(&path, WriteOptionsDto::default())
        .expect("new path save must work");
    assert_eq!(first.policy, "require_complete");
    let (bytes, bytes_report) = session
        .save_bytes(WriteOptionsDto::default())
        .expect("bytes save must work");
    assert_eq!(std::fs::read(&path).expect("path output bytes"), bytes);
    assert_eq!(first.output_sha256, bytes_report.output_sha256);

    let reopened = WorkbookSession::open_path(&path).expect("path output must reopen");
    assert!(reopened.summary().document_backed);
    assert_eq!(reopened.summary().document_kind, "xlsx");
    let page = reopened
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "A1".to_owned(),
            offset: 0,
            limit: 1,
        })
        .expect("path-opened value must be readable");
    assert_eq!(
        page.cells[0].source_value,
        CellValueDto::Number { value: 3.0 }
    );

    let destination_error = session
        .save_path(&path, WriteOptionsDto::default())
        .expect_err("replacement must be opt-in");
    assert_eq!(destination_error.kind(), InteropErrorKind::Write);
    session
        .save_path(
            &path,
            WriteOptionsDto {
                invalidate_unavailable: false,
                replace_existing: true,
            },
        )
        .expect("explicit replacement must work");
    std::fs::remove_file(&path).expect("test output must be removable");
}

fn unique_test_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "cellrune-interop-{}-{nonce}.xlsx",
        std::process::id()
    ))
}

#[test]
fn summary_exposes_tables_and_merged_ranges() {
    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
    const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row></sheetData>
  <mergeCells count="2"><mergeCell ref="D5:E6"/><mergeCell ref="A3:B4"/></mergeCells>
  <tableParts count="1"><tablePart r:id="rId7"/></tableParts>
</worksheet>"#;
    const SHEET_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/>
</Relationships>"#;
    const TABLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Sales" displayName="SalesDisplay" ref="A1:B4" totalsRowCount="1">
  <tableColumns count="2">
    <tableColumn id="1" name="Region"/>
    <tableColumn id="5" name="Amount" totalsRowFunction="sum"/>
  </tableColumns>
</table>"#;

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", SHEET),
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_RELS),
            ("xl/tables/table1.xml", TABLE),
        ] {
            use std::io::Write;
            writer.start_file(name, options).expect("fixture part");
            writer
                .write_all(contents.as_bytes())
                .expect("fixture bytes");
        }
        writer.finish().expect("fixture archive");
    }

    let session = WorkbookSession::open_bytes(buffer.get_ref()).expect("fixture must open");
    let summary = session.summary();
    let sheet = &summary.sheets[0];
    assert_eq!(sheet.merged_ranges, vec!["A3:B4", "D5:E6"]);
    assert_eq!(sheet.tables.len(), 1);
    let table = &sheet.tables[0];
    assert_eq!(table.id, 1);
    assert_eq!(table.name, "Sales");
    assert_eq!(table.display_name, "SalesDisplay");
    assert_eq!(table.range, "A1:B4");
    assert_eq!(table.header_row_count, 1);
    assert_eq!(table.totals_row_count, 1);
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| (
                column.id,
                column.name.as_str(),
                column.totals_row_function.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![(1, "Region", None), (5, "Amount", Some("sum"))]
    );

    // The serde(default) discipline: payloads from producers that predate these fields
    // still deserialize, and the defaults are semantically honest empty lists.
    let legacy = serde_json::json!({
        "id": 1,
        "name": "Data",
        "visibility": "visible",
        "cell_count": 0,
        "used_range": null,
    });
    let deserialized: cellrune_interop::SheetSummaryDto =
        serde_json::from_value(legacy).expect("legacy payload must deserialize");
    assert!(deserialized.merged_ranges.is_empty());
    assert!(deserialized.tables.is_empty());
}
