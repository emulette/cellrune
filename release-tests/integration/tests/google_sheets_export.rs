use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationOptions,
    CalculationSnapshot, CellAddress, CellContent, CellValue, ExcelError, OpenOptions,
    RecalculationWriteOptions, RecalculationWritePolicy, SavedResult, WorkbookDraft, WriteOptions,
    XlsxDocument, XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes,
    write_recalculated_xlsx_bytes, write_xlsx_draft_edits_bytes,
};
use zip::read::ZipArchive;

/// Exported by Google Sheets from synthetic data: literals, dates, booleans, shared strings,
/// error results, legacy array ranges, and Google-only functions behind `__xludf.DUMMYFUNCTION`.
const CALCULATIONS: &[u8] = include_bytes!("fixtures/google_sheets/calculations.xlsx");
/// Exported by Google Sheets with one cell of 40,000 Hangul syllables.
const LONG_TEXT: &[u8] = include_bytes!("fixtures/google_sheets/long_text.xlsx");

/// Google writes formula results rounded for display; CellRune keeps the binary result.
const ROUNDED_RESULTS: [(&str, f64); 3] = [
    ("B9", 308_642.272_75),
    ("B10", 3.300_000_000_000_000_3),
    ("B21", 0.300_000_000_000_000_04),
];
/// `NOSUCHFUNC(1)` is not a known function and `TRANSPOSE(FILTER(A2:A5` is the unparseable text
/// Google wrote for a formula it could not read.
const UNAVAILABLE: [(&str, CalculationIssueCode); 2] = [
    ("B29", CalculationIssueCode::UnsupportedFunction),
    ("B33", CalculationIssueCode::ParseError),
];

#[test]
fn calculation_reproduces_the_google_sheets_results() {
    let document = open(CALCULATIONS);
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let sheet = &document.workbook().sheets()[0];

    let mut compared = 0;
    for cell in sheet.cells() {
        let CellContent::Formula(formula) = cell.content() else {
            continue;
        };
        let a1 = cell.address().to_string();
        let result = calculation
            .cell(CalculationCellId::new(sheet.id(), cell.address()))
            .expect("formula result");
        if let Some((_, code)) = UNAVAILABLE.iter().find(|(address, _)| *address == a1) {
            let CalculationCellResult::Unavailable(issue) = result else {
                panic!("{a1} must be unavailable: {result:?}");
            };
            assert_eq!(issue.code(), *code, "{a1}");
            continue;
        }
        let CalculationCellResult::Value(value) = result else {
            panic!("{a1} must calculate: {result:?}");
        };
        if let Some((_, expected)) = ROUNDED_RESULTS.iter().find(|(address, _)| *address == a1) {
            assert_eq!(
                value,
                &CellValue::number(*expected).expect("finite"),
                "{a1}"
            );
            continue;
        }
        let SavedResult::Present(saved) = formula.saved_result() else {
            panic!("{a1} must keep the Google result");
        };
        assert_eq!(value, &google_result(saved), "{a1}");
        compared += 1;
    }
    assert_eq!(compared, 31);
}

#[test]
fn edited_saves_reopen_with_the_edit_and_the_recalculated_results() {
    let document = open(CALCULATIONS);
    let sheet_id = document.workbook().sheets()[0].id();
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            address("B2"),
            CellValue::number(5.0).expect("finite"),
        )
        .expect("edit quantity");

    let minimal = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("minimal write of a Google Sheets export");
    assert_eq!(
        minimal
            .report()
            .changed_parts()
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>(),
        ["xl/workbook.xml", "xl/worksheets/sheet1.xml"]
    );
    assert!(minimal.report().requests_host_recalculation());
    let source_parts = archive_parts(CALCULATIONS);
    for (name, bytes) in archive_parts(minimal.bytes()) {
        if name != "xl/workbook.xml" && name != "xl/worksheets/sheet1.xml" {
            assert_eq!(
                source_parts.get(&name),
                Some(&bytes),
                "{name} keeps its bytes"
            );
        }
    }

    let edited = open(minimal.bytes());
    assert_eq!(
        literal(&edited, "B2"),
        &CellValue::number(5.0).expect("finite")
    );
    let affected: BTreeSet<CalculationCellId> = minimal
        .report()
        .affected_formulas()
        .iter()
        .copied()
        .collect();
    for a1 in ["B8", "B10", "B12", "B19", "B22", "B31", "B32"] {
        let id = CalculationCellId::new(sheet_id, address(a1));
        assert!(affected.contains(&id), "{a1} depends on B2");
        assert_eq!(saved(&edited, a1), &SavedResult::Missing, "{a1}");
    }
    assert_eq!(
        saved(&edited, "B35"),
        &SavedResult::Present(CellValue::Text("a".to_owned()))
    );

    let calculation = calculate_workbook(edited.workbook(), CalculationOptions::default());
    let unavailable: BTreeSet<CalculationCellId> = UNAVAILABLE
        .iter()
        .map(|(a1, _)| CalculationCellId::new(sheet_id, address(a1)))
        .collect();
    let strict =
        write_recalculated_xlsx_bytes(&edited, &calculation, RecalculationWriteOptions::default())
            .expect_err("a complete write needs every formula result");
    assert_eq!(strict.code(), XlsxWriteErrorCode::IncompleteCalculation);
    assert_eq!(
        strict
            .unavailable_formulas()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        unavailable
    );
    let recalculated = write_recalculated_xlsx_bytes(
        &edited,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("recalculated write of the edited export");
    assert_eq!(
        recalculated
            .report()
            .invalidated_cells()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        unavailable
    );

    let reopened = open(recalculated.bytes());
    assert_saved_results_match(&reopened, &calculation);
    assert_eq!(
        saved(&reopened, "B8"),
        &SavedResult::Present(CellValue::number(13.5).expect("finite"))
    );
    assert_eq!(
        saved(&reopened, "B23"),
        &SavedResult::Present(CellValue::Error(ExcelError::DivisionByZero))
    );
    assert_eq!(saved(&reopened, "B29"), &SavedResult::Missing);
}

#[test]
fn text_beyond_the_excel_cell_limit_survives_an_edited_save() {
    let document = open(LONG_TEXT);
    let long_text = CellValue::Text("가".repeat(40_000));
    assert_eq!(literal(&document, "A1"), &long_text);

    let mut draft = WorkbookDraft::from_document(&document);
    let sheet_id = document.workbook().sheets()[0].id();
    draft
        .set_cell_value(sheet_id, address("B1"), CellValue::Text("next".to_owned()))
        .expect("edit beside the long text");
    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("minimal write beside the long text");

    let reopened = open(output.bytes());
    assert_eq!(literal(&reopened, "A1"), &long_text);
    assert_eq!(
        literal(&reopened, "B1"),
        &CellValue::Text("next".to_owned())
    );
}

/// Google stores error results as text cells; they read back as the error they name.
fn google_result(saved: &CellValue) -> CellValue {
    match saved {
        CellValue::Text(text) => {
            ExcelError::from_spelling(text).map_or_else(|| saved.clone(), CellValue::Error)
        }
        _ => saved.clone(),
    }
}

fn assert_saved_results_match(document: &XlsxDocument, calculation: &CalculationSnapshot) {
    let sheet = &document.workbook().sheets()[0];
    for cell in sheet.cells() {
        let CellContent::Formula(formula) = cell.content() else {
            continue;
        };
        if let Some(CalculationCellResult::Value(value)) =
            calculation.cell(CalculationCellId::new(sheet.id(), cell.address()))
        {
            assert_eq!(
                formula.saved_result(),
                &SavedResult::Present(value.clone()),
                "{}",
                cell.address()
            );
        }
    }
}

fn open(bytes: &[u8]) -> XlsxDocument {
    open_xlsx_document_bytes(bytes, OpenOptions::default()).expect("Google Sheets export opens")
}

fn address(text: &str) -> CellAddress {
    CellAddress::from_a1(text).expect("address")
}

fn literal<'a>(document: &'a XlsxDocument, a1: &str) -> &'a CellValue {
    let cell = document.workbook().sheets()[0]
        .cell(address(a1))
        .expect("cell");
    let CellContent::Literal(value) = cell.content() else {
        panic!("{a1} must be a literal");
    };
    value
}

fn saved<'a>(document: &'a XlsxDocument, a1: &str) -> &'a SavedResult {
    let cell = document.workbook().sheets()[0]
        .cell(address(a1))
        .expect("cell");
    let CellContent::Formula(formula) = cell.content() else {
        panic!("{a1} must be a formula");
    };
    formula.saved_result()
}

fn archive_parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("archive");
    let mut parts = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).expect("entry bytes");
        parts.insert(file.name().expect("entry name").into_owned(), contents);
    }
    parts
}
