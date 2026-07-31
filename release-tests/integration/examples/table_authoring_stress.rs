use std::io::{Cursor, Write};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use cellrune::{
    ApplyChangesError, CancellationToken, EditBatch, OpenOptions, SessionErrorCode, SessionLimits,
    TableId, TableName, WorkbookCalculationSession, WorkbookChange, WorkbookDraft,
    open_xlsx_document_bytes,
};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

const DEFAULT_FORMULAS: u32 = 250_000;

fn main() {
    let formula_count = std::env::args()
        .nth(1)
        .map(|value| value.parse::<u32>().expect("formula count must be u32"))
        .unwrap_or(DEFAULT_FORMULAS);
    assert!(formula_count > 1, "formula count must exceed one");

    let generated_at = Instant::now();
    let bytes = stress_workbook(formula_count);
    let generated_elapsed = generated_at.elapsed();
    let opened_at = Instant::now();
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("open stress workbook");
    let opened_elapsed = opened_at.elapsed();
    assert_eq!(
        document
            .workbook()
            .sheets()
            .iter()
            .map(|sheet| sheet.tables().len())
            .sum::<usize>(),
        3
    );

    let total_rewritten_formulas = formula_count as usize + 2;
    let table_id = TableId::new(1).expect("stable table ID");
    let rename = || {
        EditBatch::new([WorkbookChange::rename_table(
            table_id,
            TableName::new("Orders").expect("valid table name"),
        )])
    };

    let exact_limits = SessionLimits::default()
        .with_formula_rewrite_limits(total_rewritten_formulas, usize::MAX, usize::MAX, usize::MAX)
        .expect("exact positive rewrite limits");
    let exact_session = WorkbookCalculationSession::with_limits(
        WorkbookDraft::from_document(&document),
        exact_limits,
    );
    let exact_at = Instant::now();
    let prepared = exact_session
        .prepare_changes(exact_session.workbook().semantic_revision(), rename())
        .expect("exact whole-workbook formula budget");
    let exact_elapsed = exact_at.elapsed();
    assert_eq!(prepared.receipt().changed_table_ids(), [table_id]);
    assert_eq!(
        prepared
            .workbook()
            .table_by_id(table_id)
            .expect("renamed staged table")
            .display_name()
            .as_str(),
        "Orders"
    );
    assert_eq!(
        exact_session
            .workbook()
            .table_by_id(table_id)
            .expect("unchanged live table")
            .display_name()
            .as_str(),
        "Sales"
    );

    let short_limits = SessionLimits::default()
        .with_formula_rewrite_limits(
            total_rewritten_formulas - 1,
            usize::MAX,
            usize::MAX,
            usize::MAX,
        )
        .expect("short positive rewrite limits");
    let short_session = WorkbookCalculationSession::with_limits(
        WorkbookDraft::from_document(&document),
        short_limits,
    );
    let error = short_session
        .prepare_changes(short_session.workbook().semantic_revision(), rename())
        .expect_err("one-formula-short budget must fail");
    let ApplyChangesError::Session(error) = error else {
        panic!("rewrite budget must be a session error");
    };
    assert_eq!(error.code(), SessionErrorCode::RewriteLimitExceeded);
    assert!(
        error
            .detail()
            .is_some_and(|detail| detail.contains(&format!("actual={total_rewritten_formulas}")))
    );
    assert_eq!(
        short_session
            .workbook()
            .table_by_id(table_id)
            .expect("unchanged budget-failed table")
            .display_name()
            .as_str(),
        "Sales"
    );

    let cancellation_session =
        WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    let cancellation = CancellationToken::new();
    let cancellation_started = Instant::now();
    let cancellation_result = std::thread::scope(|scope| {
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let worker_cancellation = cancellation.clone();
        let worker_session = &cancellation_session;
        let worker = scope.spawn(move || {
            worker_barrier.wait();
            worker_session.prepare_changes_cancellable(
                worker_session.workbook().semantic_revision(),
                rename(),
                &worker_cancellation,
            )
        });
        barrier.wait();
        std::thread::sleep(Duration::from_millis(2));
        cancellation.cancel();
        worker.join().expect("cancellation worker")
    });
    let cancellation_elapsed = cancellation_started.elapsed();
    let error = cancellation_result.expect_err("in-flight staging must observe cancellation");
    assert!(matches!(
        error,
        ApplyChangesError::Session(error) if error.code() == SessionErrorCode::Cancelled
    ));
    assert_eq!(
        cancellation_session
            .workbook()
            .table_by_id(table_id)
            .expect("unchanged cancelled table")
            .display_name()
            .as_str(),
        "Sales"
    );

    println!("cellrune_table_authoring_stress_v1");
    println!("formulas\t{formula_count}");
    println!("tables\t3");
    println!("spill_formulas\t2");
    metric("generate_ms", generated_elapsed);
    metric("open_ms", opened_elapsed);
    metric("exact_rewrite_ms", exact_elapsed);
    metric("cancel_ms", cancellation_elapsed);
}

fn stress_workbook(formula_count: u32) -> Vec<u8> {
    let mut formula_rows = String::with_capacity(formula_count as usize * 96);
    formula_rows.push_str(
        r#"<row r="1"><c r="A1"><f t="array" ref="A1:A2">SEQUENCE(2)</f></c><c r="B1"><f>A1#</f></c><c r="C1"><f>SUM(Sales[Amount])+1</f></c></row>"#,
    );
    for row in 2..=formula_count {
        formula_rows.push_str(&format!(
            r#"<row r="{row}"><c r="C{row}"><f>SUM(Sales[Amount])+{row}</f></c></row>"#
        ));
    }
    let formula_sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>{formula_rows}</sheetData>
</worksheet>"#
    );
    let data_sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="D1" t="inlineStr"><is><t>Item</t></is></c><c r="E1" t="inlineStr"><is><t>Amount</t></is></c>
      <c r="G1" t="inlineStr"><is><t>Item</t></is></c><c r="H1" t="inlineStr"><is><t>Amount</t></is></c>
    </row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>A</t></is></c><c r="B2"><v>1</v></c><c r="D2" t="inlineStr"><is><t>B</t></is></c><c r="E2"><v>2</v></c><c r="G2" t="inlineStr"><is><t>C</t></is></c><c r="H2"><v>3</v></c></row>
    <row r="3"><c r="A3" t="inlineStr"><is><t>D</t></is></c><c r="B3"><v>4</v></c><c r="D3" t="inlineStr"><is><t>E</t></is></c><c r="E3"><v>5</v></c><c r="G3" t="inlineStr"><is><t>F</t></is></c><c r="H3"><v>6</v></c></row>
  </sheetData>
  <tableParts count="3"><tablePart r:id="rId1"/><tablePart r:id="rId2"/><tablePart r:id="rId3"/></tableParts>
</worksheet>"#;
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/tables/table2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
  <Override PartName="/xl/tables/table3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>
</Types>"#;
    let root_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Formulas" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let workbook_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
    let worksheet_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table3.xml"/></Relationships>"#;
    let table_one = table_xml(1, "Sales", "A1:B3");
    let table_two = table_xml(2, "Costs", "D1:E3");
    let table_three = table_xml(3, "Inventory", "G1:H3");

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (path, content) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_relationships),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", workbook_relationships),
        ("xl/worksheets/sheet1.xml", data_sheet),
        ("xl/worksheets/sheet2.xml", &formula_sheet),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            worksheet_relationships,
        ),
        ("xl/tables/table1.xml", &table_one),
        ("xl/tables/table2.xml", &table_two),
        ("xl/tables/table3.xml", &table_three),
    ] {
        writer.start_file(path, options).expect("stress entry");
        writer
            .write_all(content.as_bytes())
            .expect("stress content");
    }
    writer.finish().expect("stress archive").into_inner()
}

fn table_xml(id: u32, name: &str, range: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="{id}" name="{name}" displayName="{name}" ref="{range}" headerRowCount="1"><autoFilter ref="{range}"/><tableColumns count="2"><tableColumn id="1" name="Item"/><tableColumn id="2" name="Amount"/></tableColumns></table>"#
    )
}

fn metric(name: &str, duration: Duration) {
    println!("{name}\t{:.3}", duration.as_secs_f64() * 1_000.0);
}
