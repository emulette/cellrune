//! Explicit local audit for committed Excel-saved workbook oracles.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cellrune::{
    CalculationCellId, CalculationCellResult, CellContent, ReadOptions, SavedResult,
    WorkbookSnapshot, calculate_workbook, read_xlsx_path,
};
use cellrune_integration_tests::oracle::{
    Classification, Comparator, Expectation, Expectations, METADATA_SCHEMA, Metadata,
    ObservedValue, values_match,
};
use sha2::{Digest, Sha256};

#[path = "check_excel_oracle/report.rs"]
mod report;
#[path = "check_excel_oracle/selection.rs"]
mod selection;

const USAGE: &str = "usage: check_excel_oracle [--report <oracle-directory>]";
const METADATA_FILE: &str = "metadata.json";
const EXPECTATIONS_FILE: &str = "expectations.json";
const MESSAGE_NO_ORACLES: &str = "no oracle metadata files found";
const MESSAGE_EXPECTATION_KEYS: &str =
    "expectation keys must exactly equal the selected workbook cases";
const MESSAGE_UNCLASSIFIED: &str = "unclassified oracle case";
const MESSAGE_NOTE_REQUIRED: &str = "reviewed non-match classification requires a note";
const MESSAGE_WORKBOOK_FILENAME: &str = "workbook must be a filename within the oracle directory";
const MESSAGE_SHA_FORMAT: &str = "SHA-256 must contain exactly 64 hexadecimal digits";
const MESSAGE_ITERATIVE_CALCULATION: &str =
    "workbook iterative-calculation setting does not match metadata";

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [] => audit_all(&oracle_root()),
        [flag, directory] if flag == "--report" => report::report(Path::new(directory)),
        _ => Err(vec![USAGE.to_owned()]),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problems) => {
            for problem in problems {
                eprintln!("error: {problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn oracle_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn audit_all(root: &Path) -> Result<(), Vec<String>> {
    let mut metadata_files = Vec::new();
    collect_metadata(root, &mut metadata_files)?;
    metadata_files.sort();
    if metadata_files.is_empty() {
        return Err(vec![format!("{}: {MESSAGE_NO_ORACLES}", root.display())]);
    }
    let mut problems = Vec::new();
    for metadata_path in metadata_files {
        match load_oracle(
            metadata_path
                .parent()
                .expect("metadata path always has a parent"),
            true,
        ) {
            Ok(loaded) => {
                let counts = audit_loaded(&loaded, &mut problems);
                println!(
                    "{}: cases={} match={} divergent={} not_implemented={} host_unsupported={} excluded={}",
                    loaded.directory.display(),
                    counts.total,
                    counts.matched,
                    counts.divergent,
                    counts.not_implemented,
                    counts.host_unsupported,
                    counts.excluded,
                );
            }
            Err(error) => problems.push(error),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn collect_metadata(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Vec<String>> {
    let entries = fs::read_dir(directory)
        .map_err(|error| vec![format!("{}: {error}", directory.display())])?;
    for entry in entries {
        let path = entry
            .map_err(|error| vec![format!("{}: {error}", directory.display())])?
            .path();
        if path.is_dir() {
            collect_metadata(&path, output)?;
        } else if path.file_name().is_some_and(|name| name == METADATA_FILE) {
            output.push(path);
        }
    }
    Ok(())
}

struct LoadedOracle {
    directory: PathBuf,
    metadata: Metadata,
    expectations: Expectations,
    workbook: WorkbookSnapshot,
    selected: BTreeMap<String, CalculationCellId>,
    calculation: cellrune::CalculationSnapshot,
}

fn load_oracle(directory: &Path, require_expectations: bool) -> Result<LoadedOracle, String> {
    let metadata_path = directory.join(METADATA_FILE);
    let metadata: Metadata = read_json(&metadata_path)?;
    if metadata.schema != METADATA_SCHEMA {
        return Err(format!(
            "{}: unsupported metadata schema {}",
            metadata_path.display(),
            metadata.schema
        ));
    }
    if metadata.workbook.is_empty()
        || metadata.workbook == "."
        || metadata.workbook == ".."
        || metadata.workbook.contains('/')
        || metadata.workbook.contains('\\')
    {
        return Err(format!(
            "{}: {MESSAGE_WORKBOOK_FILENAME}",
            metadata_path.display()
        ));
    }
    let workbook_path = directory.join(&metadata.workbook);
    verify_sha256(&workbook_path, &metadata.sha256)?;
    let workbook = read_xlsx_path(&workbook_path, ReadOptions::default())
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let formula_cells = workbook
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
        .count();
    if formula_cells != metadata.formula_cells {
        return Err(format!(
            "{}: formula cell count {} != metadata {}",
            workbook_path.display(),
            formula_cells,
            metadata.formula_cells
        ));
    }
    let actual_date_system = match workbook.date_system() {
        cellrune::DateSystem::Excel1900 => "excel1900",
        cellrune::DateSystem::Excel1904 => "excel1904",
    };
    if metadata.date_system != actual_date_system {
        return Err(format!(
            "{}: date system {} != metadata {}",
            workbook_path.display(),
            actual_date_system,
            metadata.date_system
        ));
    }
    verify_iterative_calculation(
        &workbook_path,
        metadata.iterative_calculation,
        workbook.calculation_hints().iterative_calculation(),
    )?;
    let selected = selection::select_cases(&workbook, &metadata.case_selection)?;
    let expectations_path = directory.join(EXPECTATIONS_FILE);
    let expectations = if expectations_path.exists() {
        read_json(&expectations_path)?
    } else if require_expectations {
        return Err(format!("{}: file is required", expectations_path.display()));
    } else {
        Expectations::new()
    };
    let calculation = calculate_workbook(&workbook, cellrune::CalculationOptions::default());
    Ok(LoadedOracle {
        directory: directory.to_path_buf(),
        metadata,
        expectations,
        workbook,
        selected,
        calculation,
    })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{}: {MESSAGE_SHA_FORMAT}", path.display()));
    }
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{}: SHA-256 {actual} != metadata {expected}",
            path.display()
        ))
    }
}

fn verify_iterative_calculation(
    workbook_path: &Path,
    expected: bool,
    declared: Option<bool>,
) -> Result<(), String> {
    let actual = declared.unwrap_or(false);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{}: {MESSAGE_ITERATIVE_CALCULATION}; workbook={actual} metadata={expected}",
            workbook_path.display()
        ))
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    matched: usize,
    divergent: usize,
    not_implemented: usize,
    host_unsupported: usize,
    excluded: usize,
}

fn audit_loaded(loaded: &LoadedOracle, problems: &mut Vec<String>) -> Counts {
    let selected_keys = loaded.selected.keys().collect::<BTreeSet<_>>();
    let expectation_keys = loaded.expectations.keys().collect::<BTreeSet<_>>();
    if selected_keys != expectation_keys {
        let missing = selected_keys.difference(&expectation_keys).take(10);
        let extra = expectation_keys.difference(&selected_keys).take(10);
        problems.push(format!(
            "{}: {MESSAGE_EXPECTATION_KEYS}; missing={:?} extra={:?}",
            loaded.directory.display(),
            missing.collect::<Vec<_>>(),
            extra.collect::<Vec<_>>()
        ));
    }
    let mut counts = Counts {
        total: loaded.expectations.len(),
        ..Counts::default()
    };
    for (key, expectation) in &loaded.expectations {
        let context = format!("{}: {key}", loaded.directory.display());
        let Some(id) = loaded.selected.get(key).copied() else {
            continue;
        };
        audit_saved_cache(&context, loaded, id, expectation, problems);
        let result = calculated_result(loaded, id);
        match expectation.classification {
            Classification::Match => {
                let actual = match observed_result(result) {
                    Ok(Some(actual)) => actual,
                    Ok(None) => {
                        problems.push(format!("{context}: expected a value, got unavailable"));
                        continue;
                    }
                    Err(error) => {
                        problems.push(format!("{context}: {error}"));
                        continue;
                    }
                };
                match values_match(
                    &actual,
                    &ObservedValue::from_expectation(expectation),
                    expectation.comparator,
                ) {
                    Ok(true) => counts.matched += 1,
                    Ok(false) => problems.push(format!(
                        "{context}: expected {:?}, got {actual:?}",
                        ObservedValue::from_expectation(expectation)
                    )),
                    Err(error) => problems.push(format!("{context}: {error}")),
                }
            }
            Classification::Divergent => {
                require_note(&context, expectation, problems);
                let actual = match observed_result(result) {
                    Ok(Some(actual)) => actual,
                    Ok(None) => {
                        problems.push(format!("{context}: divergent case did not produce a value"));
                        continue;
                    }
                    Err(error) => {
                        problems.push(format!("{context}: {error}"));
                        continue;
                    }
                };
                let expected = ObservedValue::from_expectation(expectation);
                if values_match(&actual, &expected, expectation.comparator) == Ok(true) {
                    problems.push(format!("{context}: divergent case now matches Excel"));
                }
                let Some(recorded) = ObservedValue::from_recorded_cellrune(expectation) else {
                    problems.push(format!(
                        "{context}: divergent case lacks CellRune value/type"
                    ));
                    continue;
                };
                match values_match(&actual, &recorded, expectation.comparator) {
                    Ok(true) => counts.divergent += 1,
                    Ok(false) => problems.push(format!(
                        "{context}: CellRune side changed from {recorded:?} to {actual:?}"
                    )),
                    Err(error) => problems.push(format!("{context}: {error}")),
                }
            }
            Classification::NotImplemented => {
                require_note(&context, expectation, problems);
                if matches!(result, Some(CalculationCellResult::Unavailable(_))) {
                    counts.not_implemented += 1;
                } else {
                    problems.push(format!("{context}: not-implemented case now calculates"));
                }
            }
            Classification::HostUnsupported => {
                require_note(&context, expectation, problems);
                counts.host_unsupported += 1;
            }
            Classification::Excluded | Classification::Unreadable => {
                require_note(&context, expectation, problems);
                counts.excluded += 1;
            }
            Classification::Unclassified => {
                problems.push(format!("{context}: {MESSAGE_UNCLASSIFIED}"));
            }
        }
    }
    counts
}

fn audit_saved_cache(
    context: &str,
    loaded: &LoadedOracle,
    id: CalculationCellId,
    expectation: &Expectation,
    problems: &mut Vec<String>,
) {
    let source = source_value(&loaded.workbook, id);
    if matches!(
        expectation.classification,
        Classification::Excluded | Classification::Unreadable
    ) && source.as_ref().is_ok_and(Option::is_none)
    {
        return;
    }
    let Some(source) = source.unwrap_or_else(|error| {
        problems.push(format!("{context}: {error}"));
        None
    }) else {
        problems.push(format!("{context}: saved cache is missing"));
        return;
    };
    if expectation.excel_rich_error {
        if source.value_type != "e" {
            problems.push(format!("{context}: rich error fallback is not an error"));
        }
        return;
    }
    let expected = ObservedValue::from_expectation(expectation);
    let comparator = if expected.value_type == "n" {
        Some(Comparator::ExactBits {})
    } else {
        Some(Comparator::Exact {})
    };
    match values_match(&source, &expected, comparator) {
        Ok(true) => {}
        Ok(false) => problems.push(format!(
            "{context}: expectations record {expected:?}, saved cache contains {source:?}"
        )),
        Err(error) => problems.push(format!("{context}: {error}")),
    }
}

fn source_value(
    workbook: &WorkbookSnapshot,
    id: CalculationCellId,
) -> Result<Option<ObservedValue>, String> {
    let sheet = workbook
        .sheet_by_id(id.sheet_id())
        .ok_or_else(|| format!("unknown sheet id {}", id.sheet_id().get()))?;
    let Some(cell) = sheet.cell(id.address()) else {
        return Ok(None);
    };
    match cell.content() {
        CellContent::Literal(value) => ObservedValue::from_cell(value).map(Some),
        CellContent::Formula(formula) => match formula.saved_result() {
            SavedResult::Present(value) => ObservedValue::from_cell(value).map(Some),
            SavedResult::Missing => Ok(None),
            SavedResult::Invalid(issue) => Err(format!(
                "invalid saved result {} ({:?})",
                issue.code().as_str(),
                issue.raw_value()
            )),
        },
    }
}

fn calculated_result(
    loaded: &LoadedOracle,
    id: CalculationCellId,
) -> Option<&CalculationCellResult> {
    loaded
        .calculation
        .materialized_cell(id)
        .map(cellrune::MaterializedCalculationCell::result)
        .or_else(|| loaded.calculation.cell(id))
}

fn observed_result(
    result: Option<&CalculationCellResult>,
) -> Result<Option<ObservedValue>, String> {
    result.map_or(Ok(None), ObservedValue::from_result)
}

fn require_note(context: &str, expectation: &Expectation, problems: &mut Vec<String>) {
    if expectation.note.as_deref().is_none_or(str::is_empty) {
        problems.push(format!("{context}: {MESSAGE_NOTE_REQUIRED}"));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{MESSAGE_ITERATIVE_CALCULATION, verify_iterative_calculation};

    #[test]
    fn iterative_calculation_metadata_must_match_effective_workbook_setting() {
        assert!(verify_iterative_calculation(Path::new("workbook.xlsx"), false, None).is_ok());
        assert!(verify_iterative_calculation(Path::new("workbook.xlsx"), true, Some(true)).is_ok());

        let error = verify_iterative_calculation(Path::new("workbook.xlsx"), false, Some(true))
            .expect_err("mismatched iterative calculation");
        assert!(error.contains(MESSAGE_ITERATIVE_CALCULATION));
        assert!(error.contains("workbook=true metadata=false"));
    }
}
