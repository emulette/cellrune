//! The data-driven conformance test: every committed expectation matrix is reconstructed and
//! calculated, and `CellRune`'s values are held against the recorded oracle's.
//!
//! Statuses are enforced in both directions. A `match` case must match. A divergent case must
//! still diverge and must still produce the recorded `CellRune` value — so neither side of a
//! documented divergence can drift without this test naming it — and must carry a note, because
//! an unexplained divergence is not a documented one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, DateSystem, DefinedName, DefinedNameScope, FormulaCell, FormulaDialect,
    FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId,
    SheetName, SheetVisibility, WorkbookSnapshot, calculate_workbook, scan_formula_capabilities,
};
use cellrune_integration_tests::conformance::{
    CaseEntry, CellruneStatus, MATRIX_SCHEMA, Matrix, decode_cell_value, values_match,
};

const MESSAGE_INVALID_TOLERANCE: &str = "has an invalid tolerance";

#[test]
fn conformance_matrices_hold_against_their_recorded_oracles() {
    let matrices = collect_matrices();
    assert!(
        !matrices.is_empty(),
        "the conformance suite must contain at least one expectation matrix"
    );

    for path in matrices {
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let matrix: Matrix = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            matrix.schema,
            MATRIX_SCHEMA,
            "{}: unknown matrix schema",
            path.display()
        );
        verify_matrix(&path, &matrix);
    }
}

fn collect_matrices() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut matrices = Vec::new();
    collect_matrices_into(&root, &mut matrices);
    matrices.sort();
    matrices
}

/// Collects every `.json` matrix under `directory` at any depth, so a matrix added in a deeper
/// grouping cannot be silently excluded from the suite.
fn collect_matrices_into(directory: &Path, matrices: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("conformance suite at {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("conformance suite directory entry").path();
        if path.is_dir() {
            collect_matrices_into(&path, matrices);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            matrices.push(path);
        }
    }
}

fn verify_matrix(path: &Path, matrix: &Matrix) {
    let context = path.display().to_string();
    let context = context.as_str();
    for case in &matrix.cases {
        assert!(
            case.tolerance.is_valid(),
            "{context}: {}!{} {MESSAGE_INVALID_TOLERANCE}",
            case.sheet,
            case.cell
        );
    }
    let date_system = match matrix.date_system.as_str() {
        "excel1900" => DateSystem::Excel1900,
        "excel1904" => DateSystem::Excel1904,
        other => panic!("{context}: unknown date system {other}"),
    };

    let mut sheet_ids = BTreeMap::new();
    let mut sheets = Vec::new();
    for (index, name) in matrix.sheets.iter().enumerate() {
        let id = u32::try_from(index + 1).expect("sheet index fits an ID");
        sheet_ids.insert(name.as_str(), SheetId::new(id).expect("valid sheet ID"));
        sheets.push(Sheet::new(
            SheetId::new(id).expect("valid sheet ID"),
            SheetName::new(name).unwrap_or_else(|error| panic!("{context}: {name}: {error:?}")),
            SheetVisibility::Visible,
        ));
    }

    for literal in &matrix.literals {
        let sheet = sheet_index(&matrix.sheets, &literal.sheet)
            .unwrap_or_else(|| panic!("{context}: literal names unknown sheet {}", literal.sheet));
        let address = CellAddress::from_a1(&literal.cell)
            .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", literal.cell));
        let value = decode_cell_value(&literal.value)
            .unwrap_or_else(|error| panic!("{context}: {}: {error}", literal.cell));
        sheets[sheet]
            .insert_cell(address, CellContent::Literal(value))
            .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", literal.cell));
    }
    for case in &matrix.cases {
        let sheet = sheet_index(&matrix.sheets, &case.sheet)
            .unwrap_or_else(|| panic!("{context}: case names unknown sheet {}", case.sheet));
        let address = CellAddress::from_a1(&case.cell)
            .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", case.cell));
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx(&case.formula)
                .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", case.cell)),
            SavedResult::Missing,
            FormulaMetadata::Normal,
        );
        sheets[sheet]
            .insert_cell(address, CellContent::Formula(formula))
            .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", case.cell));
    }

    let defined_names = matrix
        .defined_names
        .iter()
        .map(|entry| {
            DefinedName::new(
                entry.name.as_str(),
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx(&entry.formula)
                    .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", entry.name)),
                false,
            )
            .unwrap_or_else(|error| panic!("{context}: {}: {error:?}", entry.name))
        })
        .collect();

    let provider =
        ProviderIdentity::new("conformance-matrix", "1").expect("valid provider identity");
    let workbook = WorkbookSnapshot::new_with_metadata(
        sheets,
        defined_names,
        Vec::new(),
        date_system,
        CalculationHints::default(),
        cellrune::WorkbookSource::default(),
        Provenance::new(provider, None),
    )
    .unwrap_or_else(|error| panic!("{context}: {error:?}"));

    let not_implemented = matrix
        .cases
        .iter()
        .filter(|case| case.cellrune_status == CellruneStatus::NotImplemented)
        .count();
    if not_implemented == 0 {
        assert!(
            scan_formula_capabilities(&workbook).is_supported(),
            "{context}: the capability scan must support a matrix without not_implemented cases"
        );
    }

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_eq!(
        calculation.len(),
        matrix.cases.len(),
        "{context}: every case must produce a calculation result"
    );

    let mut matched = 0usize;
    let mut divergent = 0usize;
    for case in &matrix.cases {
        let id = CalculationCellId::new(
            sheet_ids[case.sheet.as_str()],
            CellAddress::from_a1(&case.cell).expect("case address already validated"),
        );
        let result = calculation
            .cell(id)
            .unwrap_or_else(|| panic!("{context}: {}!{}: no result", case.sheet, case.cell));
        verify_case(context, case, result, &mut matched, &mut divergent);
    }
    println!(
        "{context}: cases={} match={matched} documented_divergent={divergent} not_implemented={not_implemented}",
        matrix.cases.len(),
    );
}

fn verify_case(
    context: &str,
    case: &CaseEntry,
    result: &CalculationCellResult,
    matched: &mut usize,
    divergent: &mut usize,
) {
    let location = format!("{context}: {}!{} ={}", case.sheet, case.cell, case.formula);
    match case.cellrune_status {
        CellruneStatus::Match => {
            let CalculationCellResult::Value(actual) = result else {
                panic!("{location}: expected a value, got {result:?}");
            };
            assert!(
                values_match(actual, &case.expected, case.tolerance),
                "{location}: expected {:?}, got {actual:?}",
                case.expected,
            );
            *matched += 1;
        }
        CellruneStatus::Divergent | CellruneStatus::IntentionallyMoreAccurate => {
            assert!(
                case.note.is_some(),
                "{location}: a divergence without a note is not documented"
            );
            let CalculationCellResult::Value(actual) = result else {
                panic!("{location}: expected a value, got {result:?}");
            };
            assert!(
                !values_match(actual, &case.expected, case.tolerance),
                "{location}: recorded as divergent but now matches the oracle; update the status"
            );
            let recorded = case
                .cellrune_value
                .as_ref()
                .unwrap_or_else(|| panic!("{location}: a divergence must record CellRune's value"));
            assert!(
                values_match(actual, recorded, case.tolerance),
                "{location}: CellRune's side of the divergence changed: recorded {recorded:?}, got {actual:?}",
            );
            *divergent += 1;
        }
        CellruneStatus::NotImplemented => {
            assert!(
                case.note.is_some(),
                "{location}: a missing capability without a note is not documented"
            );
            assert!(
                matches!(result, CalculationCellResult::Unavailable(_)),
                "{location}: recorded as not implemented but now calculates; update the status"
            );
        }
    }
}

fn sheet_index(sheets: &[String], name: &str) -> Option<usize> {
    sheets.iter().position(|sheet| sheet == name)
}
