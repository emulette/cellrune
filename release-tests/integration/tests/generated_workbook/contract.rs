use std::error::Error;
use std::io::Cursor;
use std::path::Path;

use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CellAddress, CellContent,
    CellValue, DateSystem, FormulaCapability, NumberFormatKind, ReadOptions, SavedResult,
    WorkbookSnapshot, WorkbookSourceKind, XlsxErrorCode, calculate_workbook, inspect_package,
    read_xlsx, read_xlsx_bytes, read_xlsx_path, scan_formula_capabilities,
};

use crate::support::generated_xlsx::{
    ProducerProfile, TemporaryWorkbook, generated_formula_fixture, generated_workbook,
    generated_xlsb_package,
};

const EXPECTED_FORMULAS: [(&str, &str); 8] = [
    ("B2", "Inputs!B2*2"),
    ("B3", "SUM(Inputs!B2,7.5)"),
    ("B4", "LOWER(Inputs!B3)"),
    ("B5", "NOT(Inputs!B4)"),
    ("B6", "IF(Inputs!B2>0,\"\",\"x\")"),
    ("B7", "Inputs!B5+1"),
    ("B8", "1/0"),
    ("B9", "Inputs!B6&\" / \"&TEXT(Inputs!B2,\"0.0\")"),
];

#[test]
fn generated_reference_grammar_fixture_reaches_typed_capability_analysis() {
    let formulas = [
        "Table1[]",
        "Table1[[#Data], [Amount]]",
        "Table1[ Amount ]",
        "Table1[@[Amount]]",
        "Table1[ @Amount ]",
        "Table1[A|B]",
        "Table1[😀]",
        "Table1['#OfItems]",
        "A1 B1",
        "_xlfn.ANCHORARRAY((A1 B1))",
        "_xlfn.SINGLE((A1,B1))",
        "[Book.xlsx]Sheet1:Sheet3!A1",
        "[1]!DataTable[Amount]",
        "Sheet1!MyLambda(1)",
        "A1:#REF!",
    ];
    let bytes = generated_formula_fixture(&formulas);
    let workbook =
        read_xlsx_bytes(&bytes, ReadOptions::default()).expect("generated grammar fixture");
    let calculations = workbook
        .sheet_by_name("Calculations")
        .expect("Calculations sheet");
    for (index, expected) in formulas.iter().enumerate() {
        let address = format!("B{}", index + 2);
        let CellContent::Formula(formula) = cell(calculations, &address).content() else {
            panic!("expected formula at {address}");
        };
        assert_eq!(
            formula.text().expect("formula text").as_str(),
            *expected,
            "{address}"
        );
    }

    let capabilities = scan_formula_capabilities(&workbook);
    assert_eq!(capabilities.formula_count(), formulas.len());
    for entry in capabilities.entries() {
        if let FormulaCapability::Unsupported(issues) = entry.capability() {
            assert!(
                issues
                    .iter()
                    .all(|issue| issue.code() != CalculationIssueCode::ParseError),
                "{entry:?}"
            );
        }
    }
}

#[test]
fn generated_package_exposes_the_expected_reader_contract() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let summary = inspect_package(Cursor::new(&bytes), ReadOptions::default())
        .expect("generated package must be discoverable");
    assert_eq!(summary.workbook_part().as_str(), "xl/workbook.xml");
    assert_eq!(summary.worksheet_parts().len(), 2);
    assert!(summary.styles_part().is_some());
    assert_eq!(summary.entry_count(), 7);

    let workbook =
        read_xlsx_bytes(&bytes, ReadOptions::default()).expect("generated workbook must read");
    assert_eq!(workbook.date_system(), DateSystem::Excel1900);
    assert_eq!(workbook.calculation_hints().calculation_id(), Some(191_029));
    assert_eq!(
        workbook
            .sheets()
            .iter()
            .map(|sheet| sheet.name().as_str())
            .collect::<Vec<_>>(),
        ["Inputs", "Calculations"]
    );
    assert_eq!(workbook.defined_names().len(), 1);
    assert_eq!(workbook.defined_names()[0].name(), "InputAmount");

    let inputs = workbook.sheet_by_name("inputs").expect("Inputs sheet");
    assert_eq!(literal(inputs, "B2"), &number(42.5));
    assert_eq!(literal(inputs, "B3"), &CellValue::Text("CellRune".into()));
    assert_eq!(literal(inputs, "B4"), &CellValue::Logical(true));
    assert_eq!(literal(inputs, "B5"), &number(46_225.0));
    assert_eq!(
        cell(inputs, "B5").number_format().kind(),
        NumberFormatKind::Date
    );
    assert_eq!(literal(inputs, "B6"), &CellValue::Text("한글 Ω".into()));
    assert_eq!(literal(inputs, "B7"), &number(-3.25));

    let calculations = workbook
        .sheet_by_name("calculations")
        .expect("Calculations sheet");
    for (address, expected_text) in EXPECTED_FORMULAS {
        let CellContent::Formula(formula) = cell(calculations, address).content() else {
            panic!("expected formula at Calculations!{address}");
        };
        assert_eq!(
            formula.text().expect("generated formula text").as_str(),
            expected_text,
            "Calculations!{address}"
        );
        assert!(matches!(formula.saved_result(), SavedResult::Present(_)));
    }
    assert_eq!(
        cell(calculations, "B7").number_format().kind(),
        NumberFormatKind::Date
    );
}

#[test]
fn generated_workbook_recalculates_all_typed_saved_results() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let workbook =
        read_xlsx_bytes(&bytes, ReadOptions::default()).expect("generated workbook must read");
    let capabilities = scan_formula_capabilities(&workbook);
    assert_eq!(capabilities.formula_count(), EXPECTED_FORMULAS.len());
    assert_eq!(capabilities.supported_count(), EXPECTED_FORMULAS.len());
    assert_eq!(capabilities.unsupported_count(), 0);

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_eq!(calculation.len(), EXPECTED_FORMULAS.len());
    let calculations = workbook
        .sheet_by_name("Calculations")
        .expect("Calculations sheet");
    for (address, _) in EXPECTED_FORMULAS {
        let source = cell(calculations, address);
        let CellContent::Formula(formula) = source.content() else {
            panic!("expected formula at Calculations!{address}");
        };
        let SavedResult::Present(expected) = formula.saved_result() else {
            panic!("expected typed saved result at Calculations!{address}");
        };
        let result = calculation
            .cell(cellrune::CalculationCellId::new(
                calculations.id(),
                source.address(),
            ))
            .expect("every formula has a result");
        let CalculationCellResult::Value(actual) = result else {
            panic!("generated formula must calculate at Calculations!{address}");
        };
        assert_value_equivalent(actual, expected, address);
    }
}

#[test]
fn path_bytes_and_reader_adapters_produce_the_same_public_model() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let temporary = TemporaryWorkbook::new(&bytes);
    let input_before = std::fs::read(temporary.path()).expect("temporary workbook bytes");
    let from_path = read_xlsx_path(temporary.path(), ReadOptions::default())
        .expect("path adapter should read generated workbook");
    let input_after = std::fs::read(temporary.path()).expect("temporary workbook remains readable");
    let from_bytes = read_xlsx_bytes(&bytes, ReadOptions::default())
        .expect("bytes adapter should read generated workbook");
    let from_reader = read_xlsx(Cursor::new(&bytes), ReadOptions::default())
        .expect("reader adapter should read generated workbook");

    assert_eq!(from_path.source().kind(), WorkbookSourceKind::Path);
    assert_eq!(from_bytes.source().kind(), WorkbookSourceKind::Bytes);
    assert_eq!(from_reader.source().kind(), WorkbookSourceKind::Reader);
    assert_eq!(input_after, input_before);
    assert_models_equivalent(&from_path, &from_bytes);
    assert_models_equivalent(&from_bytes, &from_reader);

    let calculations = from_path
        .sheet_by_name("Calculations")
        .expect("Calculations sheet");
    assert_eq!(
        calculations
            .cell_by_a1("B2")
            .expect("valid A1 address")
            .expect("generated cell"),
        calculations
            .cell(CellAddress::from_a1("B2").expect("valid cell address"))
            .expect("generated cell")
    );
    let saved_before = saved_results(&from_path);
    let result = calculate_workbook(&from_path, CalculationOptions::default());
    assert_eq!(result.len(), EXPECTED_FORMULAS.len());
    assert_eq!(saved_results(&from_path), saved_before);
}

#[test]
fn read_failures_expose_stable_codes_without_third_party_errors() {
    let invalid = read_xlsx_bytes(b"not an XLSX archive", ReadOptions::default())
        .expect_err("invalid ZIP bytes must fail");
    assert_eq!(invalid.code(), XlsxErrorCode::InvalidZip);
    assert_eq!(invalid.code().as_str(), "xlsx.invalid_zip");

    let missing_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generated-does-not-exist.xlsx");
    let missing =
        read_xlsx_path(missing_path, ReadOptions::default()).expect_err("missing path must fail");
    assert_eq!(missing.code(), XlsxErrorCode::Io);
    assert_eq!(missing.code().as_str(), "xlsx.io");
    assert!(missing.detail().is_none());
    assert!(Error::source(&missing).is_some());
}

#[test]
fn non_xlsx_binary_formats_are_explicitly_outside_the_reader_contract() {
    const COMPOUND_FILE_SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

    let legacy_error = read_xlsx_bytes(&COMPOUND_FILE_SIGNATURE, ReadOptions::default())
        .expect_err("legacy BIFF/OLE input must not be accepted as XLSX");
    assert_eq!(legacy_error.code(), XlsxErrorCode::InvalidZip);
    assert_eq!(legacy_error.code().as_str(), "xlsx.invalid_zip");

    let xlsb_error = read_xlsx_bytes(&generated_xlsb_package(), ReadOptions::default())
        .expect_err("XLSB workbook parts must not be accepted as XLSX XML parts");
    assert_eq!(xlsb_error.code(), XlsxErrorCode::UnsupportedContentType);
    assert_eq!(xlsb_error.code().as_str(), "xlsx.unsupported_content_type");
}

fn assert_models_equivalent(left: &WorkbookSnapshot, right: &WorkbookSnapshot) {
    assert_eq!(left.sheets().len(), right.sheets().len());
    assert_eq!(formula_count(left), EXPECTED_FORMULAS.len());
    assert_eq!(formula_count(left), formula_count(right));
    assert_eq!(saved_results(left), saved_results(right));
}

fn formula_count(workbook: &WorkbookSnapshot) -> usize {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
        .count()
}

fn saved_results(workbook: &WorkbookSnapshot) -> Vec<SavedResult> {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter_map(|cell| match cell.content() {
            CellContent::Formula(formula) => Some(formula.saved_result().clone()),
            CellContent::Literal(_) => None,
        })
        .collect()
}

fn cell<'a>(sheet: &'a cellrune::Sheet, address: &str) -> &'a cellrune::Cell {
    sheet
        .cell_by_a1(address)
        .expect("valid generated address")
        .unwrap_or_else(|| panic!("missing generated cell {address}"))
}

fn literal<'a>(sheet: &'a cellrune::Sheet, address: &str) -> &'a CellValue {
    let CellContent::Literal(value) = cell(sheet, address).content() else {
        panic!("expected generated literal at {address}");
    };
    value
}

fn number(value: f64) -> CellValue {
    CellValue::Number(cellrune::FiniteNumber::new(value).expect("finite test number"))
}

fn assert_value_equivalent(actual: &CellValue, expected: &CellValue, address: &str) {
    match (actual, expected) {
        (CellValue::Number(actual), CellValue::Number(expected)) => assert!(
            (actual.get() - expected.get()).abs() <= 1e-9,
            "numeric mismatch at {address}: {actual:?} != {expected:?}"
        ),
        _ => assert_eq!(actual, expected, "value mismatch at {address}"),
    }
}
