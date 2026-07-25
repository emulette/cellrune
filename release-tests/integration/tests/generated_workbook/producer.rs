use std::collections::{BTreeMap, BTreeSet};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, Cell, CellContent, CellValue,
    DateSystem, ExcelError, NumberFormatKind, ReadOptions, SavedResult, WorkbookSnapshot,
    calculate_workbook, read_xlsx_bytes, scan_formula_capabilities,
};
use serde::Deserialize;

use crate::support::generated_xlsx::{ProducerProfile, generated_workbook};

const MANIFEST_TEXT: &str = include_str!("../golden/producer_matrix.json");
const MANIFEST_SCHEMA: &str = "cellrune_producer_golden_v3";
const EXPECTED_DATE_SYSTEM: &str = "excel1900";
const REQUIRED_FIXTURE_IDS: [&str; 4] = ["excel", "google_sheets", "libreoffice", "openpyxl"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema: String,
    expected_date_system: String,
    expected_sheets: Vec<String>,
    literals: Vec<CellExpectation>,
    formulas: Vec<FormulaExpectation>,
    fixtures: Vec<FixtureProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellExpectation {
    cell: String,
    value: ValueExpectation,
    number_format: Option<ExpectedNumberFormat>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormulaExpectation {
    cell: String,
    text: String,
    value: ValueExpectation,
    number_format: Option<ExpectedNumberFormat>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ValueExpectation {
    Number { value: f64 },
    Text { value: String },
    Logical { value: bool },
    Error { value: String },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedNumberFormat {
    Date,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedResultPolicy {
    Present,
    Missing,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureProfile {
    id: String,
    golden_profile: GoldenProfile,
    saved_results: SavedResultPolicy,
    saved_result_overrides: Vec<SavedResultOverride>,
    literal_formula_rewrites: Vec<FormulaRewrite>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GoldenProfile {
    Excel,
    OpenPyxl,
    LibreOffice,
    GoogleSheets,
}

impl GoldenProfile {
    const fn generated(self) -> ProducerProfile {
        match self {
            Self::Excel => ProducerProfile::Excel,
            Self::OpenPyxl => ProducerProfile::OpenPyxl,
            Self::LibreOffice => ProducerProfile::LibreOffice,
            Self::GoogleSheets => ProducerProfile::GoogleSheets,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormulaRewrite {
    cell: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedResultOverride {
    cell: String,
    value: ValueExpectation,
}

#[test]
fn non_binary_producer_golden_preserves_the_workbook_contract() {
    let manifest: FixtureManifest =
        serde_json::from_str(MANIFEST_TEXT).expect("producer fixture manifest must be valid JSON");
    validate_manifest(&manifest);

    for fixture in &manifest.fixtures {
        let bytes = generated_workbook(fixture.golden_profile.generated());
        let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())
            .expect("generated producer Golden must be readable");
        assert_workbook_contract(&manifest, fixture, &workbook);
    }
}

fn validate_manifest(manifest: &FixtureManifest) {
    assert_eq!(manifest.schema, MANIFEST_SCHEMA);
    assert_eq!(manifest.expected_date_system, EXPECTED_DATE_SYSTEM);
    assert!(!manifest.expected_sheets.is_empty());
    assert!(!manifest.literals.is_empty());
    assert!(!manifest.formulas.is_empty());
    assert!(!manifest.fixtures.is_empty());

    let literal_cells: BTreeSet<&str> = manifest
        .literals
        .iter()
        .map(|expectation| expectation.cell.as_str())
        .collect();
    let formula_cells: BTreeSet<&str> = manifest
        .formulas
        .iter()
        .map(|expectation| expectation.cell.as_str())
        .collect();
    assert_eq!(literal_cells.len(), manifest.literals.len());
    assert_eq!(formula_cells.len(), manifest.formulas.len());
    assert!(literal_cells.is_disjoint(&formula_cells));

    let mut ids = BTreeSet::new();
    for fixture in &manifest.fixtures {
        assert!(!fixture.id.trim().is_empty());
        assert!(ids.insert(fixture.id.as_str()), "duplicate fixture id");
        let mut rewrite_cells = BTreeSet::new();
        for rewrite in &fixture.literal_formula_rewrites {
            assert!(literal_cells.contains(rewrite.cell.as_str()));
            assert!(rewrite_cells.insert(rewrite.cell.as_str()));
            assert!(!rewrite.text.trim().is_empty());
        }
        let mut saved_result_override_cells = BTreeSet::new();
        for saved_result_override in &fixture.saved_result_overrides {
            assert!(formula_cells.contains(saved_result_override.cell.as_str()));
            assert!(saved_result_override_cells.insert(saved_result_override.cell.as_str()));
        }
    }
    let required_ids: BTreeSet<&str> = REQUIRED_FIXTURE_IDS.into_iter().collect();
    assert_eq!(ids, required_ids);
}

fn assert_workbook_contract(
    manifest: &FixtureManifest,
    fixture: &FixtureProfile,
    workbook: &WorkbookSnapshot,
) {
    assert_eq!(
        workbook.date_system(),
        DateSystem::Excel1900,
        "{}",
        fixture.id
    );
    assert!(workbook.diagnostics().is_empty(), "{}", fixture.id);
    let sheet_names: Vec<&str> = workbook
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect();
    assert_eq!(sheet_names, manifest.expected_sheets, "{}", fixture.id);

    let rewrites: BTreeMap<&str, &FormulaRewrite> = fixture
        .literal_formula_rewrites
        .iter()
        .map(|rewrite| (rewrite.cell.as_str(), rewrite))
        .collect();
    for expectation in &manifest.literals {
        let (_, cell) = resolve_cell(workbook, &expectation.cell);
        if let Some(rewrite) = rewrites.get(expectation.cell.as_str()) {
            let CellContent::Formula(formula) = cell.content() else {
                panic!(
                    "{} must be a producer formula rewrite in {}",
                    expectation.cell, fixture.id
                );
            };
            assert_eq!(
                formula.text().expect("fixture formula text").as_str(),
                rewrite.text,
                "{} in {}",
                expectation.cell,
                fixture.id
            );
            assert_saved_result(
                formula.saved_result(),
                fixture.saved_results,
                &expectation.value,
                &expectation.cell,
                &fixture.id,
            );
        } else {
            let CellContent::Literal(actual) = cell.content() else {
                panic!("{} must be a literal in {}", expectation.cell, fixture.id);
            };
            assert_value(actual, &expectation.value, &expectation.cell, &fixture.id);
        }
        assert_number_format(cell, expectation.number_format, &expectation.cell);
    }

    let report = scan_formula_capabilities(workbook);
    let expected_formula_count = manifest.formulas.len() + fixture.literal_formula_rewrites.len();
    assert_eq!(
        report.formula_count(),
        expected_formula_count,
        "{}",
        fixture.id
    );
    assert_eq!(
        report.supported_count(),
        expected_formula_count,
        "{}",
        fixture.id
    );
    assert_eq!(report.unsupported_count(), 0, "{}", fixture.id);

    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    assert_eq!(calculation.len(), expected_formula_count, "{}", fixture.id);
    let saved_result_overrides: BTreeMap<&str, &SavedResultOverride> = fixture
        .saved_result_overrides
        .iter()
        .map(|saved_result_override| (saved_result_override.cell.as_str(), saved_result_override))
        .collect();
    for expectation in &manifest.formulas {
        let (sheet_id, cell) = resolve_cell(workbook, &expectation.cell);
        let CellContent::Formula(formula) = cell.content() else {
            panic!("{} must be a formula in {}", expectation.cell, fixture.id);
        };
        assert_eq!(
            formula.text().expect("fixture formula text").as_str(),
            expectation.text,
            "{} in {}",
            expectation.cell,
            fixture.id
        );
        assert_saved_result(
            formula.saved_result(),
            fixture.saved_results,
            saved_result_overrides
                .get(expectation.cell.as_str())
                .map_or(&expectation.value, |saved_result_override| {
                    &saved_result_override.value
                }),
            &expectation.cell,
            &fixture.id,
        );
        assert_number_format(cell, expectation.number_format, &expectation.cell);

        let result = calculation
            .cell(CalculationCellId::new(sheet_id, cell.address()))
            .expect("every fixture formula must have a calculation result");
        let CalculationCellResult::Value(actual) = result else {
            panic!("{} must calculate in {}", expectation.cell, fixture.id);
        };
        assert_value(actual, &expectation.value, &expectation.cell, &fixture.id);
    }

    let literal_expectations: BTreeMap<&str, &CellExpectation> = manifest
        .literals
        .iter()
        .map(|expectation| (expectation.cell.as_str(), expectation))
        .collect();
    for rewrite in &fixture.literal_formula_rewrites {
        let expectation = literal_expectations
            .get(rewrite.cell.as_str())
            .expect("validated rewrite must reference a literal expectation");
        let (sheet_id, cell) = resolve_cell(workbook, &rewrite.cell);
        let result = calculation
            .cell(CalculationCellId::new(sheet_id, cell.address()))
            .expect("every producer rewrite must have a calculation result");
        let CalculationCellResult::Value(actual) = result else {
            panic!("{} must calculate in {}", rewrite.cell, fixture.id);
        };
        assert_value(actual, &expectation.value, &rewrite.cell, &fixture.id);
    }
}

fn resolve_cell<'workbook>(
    workbook: &'workbook WorkbookSnapshot,
    qualified_cell: &str,
) -> (cellrune::SheetId, &'workbook Cell) {
    let (sheet_name, address) = qualified_cell
        .split_once('!')
        .expect("fixture cell must be qualified with one sheet name");
    assert!(!sheet_name.is_empty());
    assert!(!address.is_empty());
    assert!(!address.contains('!'));
    let sheet = workbook
        .sheet_by_name(sheet_name)
        .expect("fixture expectation must reference an existing sheet");
    let cell = sheet
        .cell_by_a1(address)
        .expect("fixture expectation must use a valid A1 address")
        .expect("fixture expectation must reference an existing cell");
    (sheet.id(), cell)
}

fn assert_saved_result(
    actual: &SavedResult,
    policy: SavedResultPolicy,
    expected: &ValueExpectation,
    cell: &str,
    fixture_id: &str,
) {
    match (policy, actual) {
        (SavedResultPolicy::Present, SavedResult::Present(actual)) => {
            assert_value(actual, expected, cell, fixture_id);
        }
        (SavedResultPolicy::Missing, SavedResult::Missing) => {}
        _ => panic!("unexpected saved result for {cell} in {fixture_id}"),
    }
}

fn assert_number_format(cell: &Cell, expected: Option<ExpectedNumberFormat>, address: &str) {
    if let Some(ExpectedNumberFormat::Date) = expected {
        assert_eq!(
            cell.number_format().kind(),
            NumberFormatKind::Date,
            "{address}"
        );
    }
}

fn assert_value(actual: &CellValue, expected: &ValueExpectation, cell: &str, fixture_id: &str) {
    let matches = match (actual, expected) {
        (CellValue::Number(actual), ValueExpectation::Number { value }) => {
            (actual.get() - value).abs() <= f64::EPSILON
        }
        (CellValue::Text(actual), ValueExpectation::Text { value }) => actual == value,
        (CellValue::Logical(actual), ValueExpectation::Logical { value }) => actual == value,
        (CellValue::Error(actual), ValueExpectation::Error { value }) => {
            expected_error(value).is_some_and(|expected| *actual == expected)
        }
        _ => false,
    };
    assert!(
        matches,
        "unexpected value for {cell} in {fixture_id}: {actual:?}"
    );
}

fn expected_error(value: &str) -> Option<ExcelError> {
    match value {
        "#NULL!" => Some(ExcelError::Null),
        "#DIV/0!" => Some(ExcelError::DivisionByZero),
        "#VALUE!" => Some(ExcelError::Value),
        "#REF!" => Some(ExcelError::Reference),
        "#NAME?" => Some(ExcelError::Name),
        "#NUM!" => Some(ExcelError::Number),
        "#N/A" => Some(ExcelError::NotAvailable),
        "#GETTING_DATA" => Some(ExcelError::GettingData),
        "#SPILL!" => Some(ExcelError::Spill),
        "#CALC!" => Some(ExcelError::Calculation),
        _ => None,
    }
}
