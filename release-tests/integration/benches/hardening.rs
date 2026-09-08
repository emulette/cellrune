use std::fmt::Write as _;
use std::hint::black_box;
use std::io::{Cursor, Write as _};
use std::time::{Duration, Instant};

use cellrune::{
    CalculationCellResult, CalculationOptions, CellAddress, CellContent, CellValue, EditBatch,
    OpenOptions, RecalculationWriteOptions, WorkbookChange, WorkbookDraft, calculate_workbook,
    open_xlsx_document_bytes, scan_formula_capabilities, write_recalculated_xlsx_bytes,
    write_xlsx_draft_bytes,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const DEFAULT_ROWS: u32 = 50_000;
const DEFAULT_ITERATIONS: u32 = 3;

#[path = "support/calculation_preparation.rs"]
mod calculation_preparation;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let test_mode = cfg!(test) || arguments.iter().any(|value| value == "--test");
    let numeric_arguments: Vec<&str> = arguments
        .iter()
        .filter(|value| !value.starts_with('-'))
        .map(String::as_str)
        .collect();
    let rows = argument(
        &numeric_arguments,
        0,
        if test_mode { 10 } else { DEFAULT_ROWS },
    );
    let iterations = argument(
        &numeric_arguments,
        1,
        if test_mode { 1 } else { DEFAULT_ITERATIONS },
    );
    assert!(rows > 0, "row count must be greater than zero");
    assert!(iterations > 0, "iteration count must be greater than zero");

    if arguments
        .iter()
        .any(|value| value == "--calculation-preparation")
    {
        calculation_preparation::run(rows, iterations, &arguments);
        return;
    }

    if arguments.iter().any(|value| value == "--draft-write") {
        let edited = argument(&numeric_arguments, 2, rows);
        assert!(edited <= rows, "edited rows must fit the source worksheet");
        run_draft_write_benchmark(rows, iterations, edited);
        return;
    }

    let archive = build_workbook(rows);
    let mut read_total = Duration::ZERO;
    let mut scan_total = Duration::ZERO;
    let mut calculate_total = Duration::ZERO;
    let mut write_total = Duration::ZERO;
    let mut reopen_and_recalculate_total = Duration::ZERO;
    let mut output_bytes = 0;

    for _ in 0..iterations {
        let read_started = Instant::now();
        let document = open_xlsx_document_bytes(black_box(&archive), OpenOptions::default())
            .expect("benchmark workbook must open");
        read_total += read_started.elapsed();

        let scan_started = Instant::now();
        let report = scan_formula_capabilities(black_box(document.workbook()));
        scan_total += scan_started.elapsed();
        assert!(report.is_supported(), "benchmark formula must be supported");

        let calculate_started = Instant::now();
        let calculation = calculate_workbook(
            black_box(document.workbook()),
            black_box(CalculationOptions::default()),
        );
        calculate_total += calculate_started.elapsed();
        assert_formula_result(&calculation, rows);

        let write_started = Instant::now();
        let output = write_recalculated_xlsx_bytes(
            black_box(&document),
            black_box(&calculation),
            RecalculationWriteOptions::default(),
        )
        .expect("benchmark workbook must write");
        write_total += write_started.elapsed();
        assert!(output.report().is_complete());
        output_bytes = output.bytes().len();

        let reopen_and_recalculate_started = Instant::now();
        let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
            .expect("benchmark output must reopen");
        let reopened_calculation =
            calculate_workbook(reopened.workbook(), CalculationOptions::default());
        reopen_and_recalculate_total += reopen_and_recalculate_started.elapsed();
        assert_formula_result(&reopened_calculation, rows);
    }

    println!("cellrune_hardening_benchmark_v3");
    println!("rows\t{rows}");
    println!("iterations\t{iterations}");
    println!("archive_bytes\t{}", archive.len());
    println!("output_bytes\t{output_bytes}");
    println!("read_mean_ms\t{:.3}", mean_ms(read_total, iterations));
    println!("scan_mean_ms\t{:.3}", mean_ms(scan_total, iterations));
    println!(
        "calculate_mean_ms\t{:.3}",
        mean_ms(calculate_total, iterations)
    );
    println!("write_mean_ms\t{:.3}", mean_ms(write_total, iterations));
    println!(
        "reopen_and_recalculate_mean_ms\t{:.3}",
        mean_ms(reopen_and_recalculate_total, iterations)
    );
    println!("read_mean_ns\t{}", mean_ns(read_total, iterations));
    println!("scan_mean_ns\t{}", mean_ns(scan_total, iterations));
    println!(
        "calculate_mean_ns\t{}",
        mean_ns(calculate_total, iterations)
    );
    println!("write_mean_ns\t{}", mean_ns(write_total, iterations));
    println!(
        "reopen_and_recalculate_mean_ns\t{}",
        mean_ns(reopen_and_recalculate_total, iterations)
    );
}

fn argument(arguments: &[&str], index: usize, default: u32) -> u32 {
    arguments
        .get(index)
        .map(|value| {
            value
                .parse::<u32>()
                .expect("benchmark argument must be u32")
        })
        .unwrap_or(default)
}

fn run_draft_write_benchmark(rows: u32, iterations: u32, edited: u32) {
    let mut canonical = WorkbookDraft::new();
    set_draft_values(&mut canonical, rows, 1.0);
    let document = open_xlsx_document_bytes(&build_workbook(rows), OpenOptions::default())
        .expect("source workbook");
    let mut patched = WorkbookDraft::from_document(&document);
    set_draft_values(&mut patched, edited, 2.0);

    println!("cellrune_draft_write_benchmark_v1");
    println!("rows\t{rows}");
    println!("edited_rows\t{edited}");
    println!("iterations\t{iterations}");
    for (name, draft, changed) in [("canonical", &canonical, 0), ("patch", &patched, edited)] {
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        if name == "patch" {
            assert_formula_result(&calculation, rows + edited);
        }
        let mut elapsed = Duration::ZERO;
        let mut output_bytes = 0;
        for _ in 0..iterations {
            let started = Instant::now();
            let output = write_xlsx_draft_bytes(
                black_box(draft),
                black_box(&calculation),
                RecalculationWriteOptions::default(),
            )
            .expect("draft output");
            elapsed += started.elapsed();
            assert!(output.report().is_complete());
            output_bytes = output.bytes().len();
            let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
                .expect("draft output reopens");
            let sheet = &reopened.workbook().sheets()[0];
            for row in 1..=rows {
                let address = CellAddress::from_indices(row, 1).expect("generated address");
                let expected = CellValue::number(if row <= changed { 2.0 } else { 1.0 })
                    .expect("finite value");
                assert_eq!(
                    sheet.cell(address).expect("written cell").content(),
                    &CellContent::Literal(expected),
                );
            }
            if name == "patch" {
                let reopened_calculation =
                    calculate_workbook(reopened.workbook(), CalculationOptions::default());
                assert_formula_result(&reopened_calculation, rows + edited);
            }
        }
        println!("{name}_write_mean_ms\t{:.3}", mean_ms(elapsed, iterations));
        println!("{name}_output_bytes\t{output_bytes}");
    }
}

fn set_draft_values(draft: &mut WorkbookDraft, rows: u32, value: f64) {
    let sheet = draft.workbook().sheets()[0].id();
    for start in (1..=rows).step_by(25_000) {
        let end = (start + 24_999).min(rows);
        draft
            .apply_changes(EditBatch::new((start..=end).map(|row| {
                WorkbookChange::set_cell_value(
                    sheet,
                    CellAddress::from_indices(row, 1).expect("generated address"),
                    CellValue::number(value).expect("finite value"),
                )
            })))
            .expect("bounded benchmark batch");
    }
}

fn mean_ms(total: Duration, iterations: u32) -> f64 {
    total.as_secs_f64() * 1_000.0 / f64::from(iterations)
}

fn mean_ns(total: Duration, iterations: u32) -> u128 {
    total.as_nanos() / u128::from(iterations)
}

fn assert_formula_result(calculation: &cellrune::CalculationSnapshot, rows: u32) {
    let (_, result) = calculation
        .cells()
        .next()
        .expect("benchmark formula result");
    let CalculationCellResult::Value(CellValue::Number(number)) = result else {
        panic!("benchmark formula must produce a number");
    };
    assert_eq!(number.get(), f64::from(rows));
}

fn build_workbook(rows: u32) -> Vec<u8> {
    let mut worksheet = String::with_capacity(rows as usize * 40);
    worksheet.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for row in 1..=rows {
        if row == 1 {
            write!(
                worksheet,
                r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>SUM(A:A)</f><v>{rows}</v></c></row>"#,
            )
            .expect("write worksheet row");
        } else {
            write!(
                worksheet,
                r#"<row r="{row}"><c r="A{row}"><v>1</v></c></row>"#,
            )
            .expect("write worksheet row");
        }
    }
    worksheet.push_str("</sheetData></worksheet>");

    let parts = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", worksheet.as_str()),
    ];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (name, contents) in parts {
        writer
            .start_file(name, options)
            .expect("start benchmark package part");
        writer
            .write_all(contents.as_bytes())
            .expect("write benchmark package part");
    }
    writer
        .finish()
        .expect("finish benchmark package")
        .into_inner()
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
