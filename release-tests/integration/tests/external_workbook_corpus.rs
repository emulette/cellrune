use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use cellrune::{
    CalculationCellResult, CalculationOptions, CellContent, CellValue, FormulaCapability,
    ReadOptions, SavedResult, calculate_workbook, read_xlsx_path, scan_formula_capabilities,
};

const CORPUS_PATH_ENV: &str = "CELLRUNE_WORKBOOK_CORPUS";
const NUMERIC_TOLERANCE: f64 = 1e-8;
const MAX_MISMATCH_SAMPLES: usize = 10;

#[derive(Debug, Default)]
struct AuditCounts {
    formulas: usize,
    calculated_values: usize,
    unavailable: usize,
    saved_present: usize,
    saved_missing: usize,
    saved_invalid: usize,
    compared: usize,
    matched: usize,
    mismatched: usize,
}

#[test]
#[ignore = "requires an external XLSX file or directory in CELLRUNE_WORKBOOK_CORPUS"]
fn external_workbook_corpus_preserves_pipeline_invariants_and_audits_saved_results() {
    let root = std::env::var_os(CORPUS_PATH_ENV).expect("external workbook corpus path");
    let workbooks = collect_workbooks(Path::new(&root)).expect("discover external XLSX corpus");
    assert!(
        !workbooks.is_empty(),
        "external XLSX corpus must not be empty"
    );

    for path in workbooks {
        audit_workbook(&path);
    }
}

fn collect_workbooks(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let metadata = fs::metadata(root)?;
    let mut workbooks = Vec::new();
    if metadata.is_file() {
        if is_xlsx(root) {
            workbooks.push(root.to_path_buf());
        }
    } else if metadata.is_dir() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && is_xlsx(&entry.path()) {
                workbooks.push(entry.path());
            }
        }
    }
    workbooks.sort();
    Ok(workbooks)
}

fn is_xlsx(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

fn audit_workbook(path: &Path) {
    let workbook = read_xlsx_path(path, ReadOptions::default())
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let capability = scan_formula_capabilities(&workbook);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let formulas = workbook
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
        .count();
    assert_eq!(
        capability.formula_count(),
        formulas,
        "capability cardinality"
    );
    assert_eq!(calculation.len(), formulas, "calculation cardinality");

    let mut issue_counts = BTreeMap::<String, usize>::new();
    for entry in capability.entries() {
        let FormulaCapability::Unsupported(issues) = entry.capability() else {
            continue;
        };
        for issue in issues {
            let key = match issue.detail() {
                Some(detail) => format!("{}:{detail}", issue.code().as_str()),
                None => issue.code().as_str().to_owned(),
            };
            *issue_counts.entry(key).or_default() += 1;
        }
    }

    let mut counts = AuditCounts {
        formulas,
        ..AuditCounts::default()
    };
    let mut mismatch_samples = Vec::new();
    for (cell_id, result) in calculation.cells() {
        let sheet = workbook
            .sheet_by_id(cell_id.sheet_id())
            .expect("calculated cell sheet");
        let cell = sheet
            .cell(cell_id.address())
            .expect("calculated source cell");
        let CellContent::Formula(formula) = cell.content() else {
            panic!("calculated source must be a formula");
        };
        match formula.saved_result() {
            SavedResult::Present(saved) => {
                counts.saved_present += 1;
                if let CalculationCellResult::Value(actual) = result {
                    counts.compared += 1;
                    if cell_values_match(actual, saved) {
                        counts.matched += 1;
                    } else {
                        counts.mismatched += 1;
                        if mismatch_samples.len() < MAX_MISMATCH_SAMPLES {
                            mismatch_samples.push(format!(
                                "{}!{} formula={:?} expected={saved:?} actual={actual:?}",
                                sheet.name().as_str(),
                                cell_id.address(),
                                formula.text().map(|text| text.as_str()),
                            ));
                        }
                    }
                }
            }
            SavedResult::Missing => counts.saved_missing += 1,
            SavedResult::Invalid(_) => counts.saved_invalid += 1,
        }
        match result {
            CalculationCellResult::Value(_) => counts.calculated_values += 1,
            CalculationCellResult::Unavailable(issue) => {
                counts.unavailable += 1;
                let key = format!("runtime:{}", issue.code().as_str());
                *issue_counts.entry(key).or_default() += 1;
            }
        }
    }

    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("<non-utf8-name>");
    println!(
        "{file_name}: sheets={} diagnostics={} formulas={} supported={} values={} unavailable={} saved_present={} saved_missing={} saved_invalid={} compared={} matched={} mismatched={}",
        workbook.sheets().len(),
        workbook.diagnostics().len(),
        counts.formulas,
        capability.supported_count(),
        counts.calculated_values,
        counts.unavailable,
        counts.saved_present,
        counts.saved_missing,
        counts.saved_invalid,
        counts.compared,
        counts.matched,
        counts.mismatched,
    );
    for (issue, count) in issue_counts {
        println!("  issue {issue}={count}");
    }
    for mismatch in mismatch_samples {
        println!("  mismatch {mismatch}");
    }
}

fn cell_values_match(actual: &CellValue, saved: &CellValue) -> bool {
    match (actual, saved) {
        (CellValue::Number(actual), CellValue::Number(saved)) => {
            let scale = actual.get().abs().max(saved.get().abs()).max(1.0);
            (actual.get() - saved.get()).abs() <= NUMERIC_TOLERANCE * scale
        }
        _ => actual == saved,
    }
}
