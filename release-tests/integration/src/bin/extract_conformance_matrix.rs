//! Extracts a conformance expectation matrix from a workbook whose saved results were produced
//! by a recorded oracle.
//!
//! The workbook itself is usually not redistributable, so this tool reads it from a private
//! location and emits the redistributable part: every literal, every formula, and the value the
//! oracle saved for that formula, together with the oracle and source provenance supplied as
//! metadata. Each case is pre-classified by running `CellRune` over the reconstructed inputs —
//! matches become `match`, everything else becomes `divergent` with `CellRune`'s actual value
//! recorded, awaiting a reviewed status and note before the committed test data accepts it.
//!
//! Usage: `extract_conformance_matrix <workbook.xlsx> <metadata.json> <output.json>`

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellContent, DateSystem,
    DefinedNameScope, FormulaMetadata, ReadOptions, SavedResult, calculate_workbook,
    read_xlsx_path,
};
use serde::Deserialize;

use cellrune_integration_tests::conformance::{
    CaseEntry, CellruneStatus, DefinedNameEntry, LiteralEntry, MATRIX_SCHEMA, Matrix,
    OracleMetadata, SCALED_EPSILON, SourceMetadata, Tolerance, ValueEncoding, encode_cell_value,
    values_match,
};

const USAGE: &str =
    "usage: extract_conformance_matrix <workbook.xlsx> <metadata.json> <output.json>";
const MESSAGE_SHEET_SCOPED_NAME: &str =
    "sheet-scoped defined names are not supported by matrix schema v1";
const MESSAGE_MISSING_FORMULA_TEXT: &str = "formula has no stored text";
const MESSAGE_MISSING_SAVED_RESULT: &str = "oracle saved no result for this formula";
const MESSAGE_UNSUPPORTED_FORMULA_METADATA: &str =
    "matrix schema v1 supports only normal and resolved shared formulas";
const MESSAGE_RECALCULATE_ALWAYS: &str =
    "matrix schema v1 does not preserve recalculate-always metadata";

/// The oracle and source provenance that cannot be read out of the workbook file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractionMetadata {
    oracle: OracleMetadata,
    source: SourceMetadata,
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [workbook_path, metadata_path, output_path] = arguments.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    match run(
        Path::new(workbook_path),
        Path::new(metadata_path),
        Path::new(output_path),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(problems) => {
            for problem in problems {
                eprintln!("error: {problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(workbook_path: &Path, metadata_path: &Path, output_path: &Path) -> Result<(), Vec<String>> {
    let metadata_text = fs::read_to_string(metadata_path)
        .map_err(|error| vec![format!("{}: {error}", metadata_path.display())])?;
    let metadata: ExtractionMetadata = serde_json::from_str(&metadata_text)
        .map_err(|error| vec![format!("{}: {error}", metadata_path.display())])?;

    let workbook = read_xlsx_path(workbook_path, ReadOptions::default())
        .map_err(|error| vec![format!("{}: {error}", workbook_path.display())])?;

    let date_system = match workbook.date_system() {
        DateSystem::Excel1900 => "excel1900",
        DateSystem::Excel1904 => "excel1904",
    };

    let mut problems = Vec::new();
    let mut defined_names = Vec::new();
    for name in workbook.defined_names() {
        if name.scope() == DefinedNameScope::Workbook {
            defined_names.push(DefinedNameEntry {
                name: name.name().to_owned(),
                formula: name.formula().as_str().to_owned(),
            });
        } else {
            problems.push(format!("{}: {}", name.name(), MESSAGE_SHEET_SCOPED_NAME));
        }
    }

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    let mut sheets = Vec::new();
    let mut literals = Vec::new();
    let mut cases = Vec::new();
    for sheet in workbook.sheets() {
        sheets.push(sheet.name().as_str().to_owned());
        let mut cells: Vec<_> = sheet.cells().collect();
        cells.sort_by_key(|cell| (cell.address().row(), cell.address().column()));
        for cell in cells {
            let sheet_name = sheet.name().as_str().to_owned();
            let address = cell.address().to_string();
            match cell.content() {
                CellContent::Literal(value) => literals.push(LiteralEntry {
                    sheet: sheet_name,
                    cell: address,
                    value: encode_cell_value(value),
                }),
                CellContent::Formula(formula) => {
                    if !matches!(
                        formula.metadata(),
                        FormulaMetadata::Normal | FormulaMetadata::Shared { .. }
                    ) {
                        problems.push(format!(
                            "{sheet_name}!{address}: {MESSAGE_UNSUPPORTED_FORMULA_METADATA}: {:?}",
                            formula.metadata()
                        ));
                        continue;
                    }
                    if formula.recalculate_always() {
                        problems.push(format!(
                            "{sheet_name}!{address}: {MESSAGE_RECALCULATE_ALWAYS}"
                        ));
                        continue;
                    }
                    let Some(text) = formula.text() else {
                        problems.push(format!(
                            "{sheet_name}!{address}: {MESSAGE_MISSING_FORMULA_TEXT}"
                        ));
                        continue;
                    };
                    let SavedResult::Present(saved) = formula.saved_result() else {
                        problems.push(format!(
                            "{sheet_name}!{address}: {MESSAGE_MISSING_SAVED_RESULT}"
                        ));
                        continue;
                    };
                    let expected = encode_cell_value(saved);
                    let tolerance = match expected {
                        ValueEncoding::Number { .. } => Tolerance::Scaled {
                            epsilon: SCALED_EPSILON,
                        },
                        _ => Tolerance::Exact,
                    };
                    let result = calculation
                        .cell(CalculationCellId::new(sheet.id(), cell.address()))
                        .expect("every formula cell has a calculation result");
                    let (status, cellrune_value) = match result {
                        CalculationCellResult::Value(actual)
                            if values_match(actual, &expected, tolerance) =>
                        {
                            (CellruneStatus::Match, None)
                        }
                        CalculationCellResult::Value(actual) => {
                            (CellruneStatus::Divergent, Some(encode_cell_value(actual)))
                        }
                        CalculationCellResult::Unavailable(_) => {
                            (CellruneStatus::NotImplemented, None)
                        }
                    };
                    cases.push(CaseEntry {
                        sheet: sheet_name,
                        cell: address,
                        formula: text.as_str().to_owned(),
                        expected,
                        tolerance,
                        cellrune_status: status,
                        cellrune_value,
                        note: None,
                    });
                }
            }
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }

    let matrix = Matrix {
        schema: MATRIX_SCHEMA.to_owned(),
        oracle: metadata.oracle,
        source: metadata.source,
        date_system: date_system.to_owned(),
        sheets,
        defined_names,
        literals,
        cases,
    };

    let mut serialized =
        serde_json::to_string_pretty(&matrix).expect("matrix serialization is infallible");
    serialized.push('\n');
    fs::write(output_path, serialized)
        .map_err(|error| vec![format!("{}: {error}", output_path.display())])?;

    let matched = matrix
        .cases
        .iter()
        .filter(|case| case.cellrune_status == CellruneStatus::Match)
        .count();
    println!(
        "sheets={} defined_names={} literals={} cases={} match={} divergent={} not_implemented={}",
        matrix.sheets.len(),
        matrix.defined_names.len(),
        matrix.literals.len(),
        matrix.cases.len(),
        matched,
        matrix
            .cases
            .iter()
            .filter(|case| case.cellrune_status == CellruneStatus::Divergent)
            .count(),
        matrix
            .cases
            .iter()
            .filter(|case| case.cellrune_status == CellruneStatus::NotImplemented)
            .count(),
    );
    Ok(())
}
