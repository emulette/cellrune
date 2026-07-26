use std::collections::BTreeMap;
use std::error::Error;

use cellrune::{
    CalculationCellResult, CalculationLimits, CalculationOptions, CellContent, FiniteNumber,
    FormulaCapability, ReadOptions, calculate_workbook, read_xlsx_path,
    scan_formula_capabilities_with_options,
};

const USAGE: &str = "usage: cargo run -p cellrune --example read_and_calculate -- \
                     <input.xlsx> [today-excel-serial|--read-only|--scan-only]";
const MAX_SAMPLES_PER_ISSUE: usize = 3;
const MAX_FORMULA_SAMPLE_CHARS: usize = 320;
const MAX_DEPENDENCY_EDGES_ENV: &str = "CELLRUNE_MAX_DEPENDENCY_EDGES";
const READ_ONLY: &str = "--read-only";
const SCAN_ONLY: &str = "--scan-only";

fn main() -> Result<(), Box<dyn Error>> {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("{USAGE}");
        return Ok(());
    };

    let workbook = read_xlsx_path(path, ReadOptions::default())?;
    println!(
        "read {} sheets and {} compatibility diagnostics",
        workbook.sheets().len(),
        workbook.diagnostics().len()
    );
    for diagnostic in workbook.diagnostics() {
        eprintln!(
            "[{:?}] {}: {}",
            diagnostic.severity(),
            diagnostic.code().as_str(),
            diagnostic.message()
        );
    }

    let second_argument = std::env::args_os().nth(2);
    if second_argument.as_deref() == Some(std::ffi::OsStr::new(READ_ONLY)) {
        return Ok(());
    }

    let limits = match std::env::var(MAX_DEPENDENCY_EDGES_ENV) {
        Ok(value) => {
            CalculationLimits::default().with_max_dependency_edges(value.parse::<u64>()?)?
        }
        Err(std::env::VarError::NotPresent) => CalculationLimits::default(),
        Err(error) => return Err(error.into()),
    };
    let options = CalculationOptions::default().with_limits(limits);
    if second_argument.as_deref() == Some(std::ffi::OsStr::new(SCAN_ONLY)) {
        let capabilities = scan_formula_capabilities_with_options(&workbook, options);
        println!(
            "formula capability inventory: {} supported, {} unsupported",
            capabilities.supported_count(),
            capabilities.unsupported_count()
        );
        let mut issue_counts =
            BTreeMap::<(cellrune::CalculationIssueCode, Option<String>), usize>::new();
        let mut issue_samples =
            BTreeMap::<(cellrune::CalculationIssueCode, Option<String>), Vec<String>>::new();
        for entry in capabilities.entries() {
            let FormulaCapability::Unsupported(issues) = entry.capability() else {
                continue;
            };
            let sheet_name = workbook
                .sheet_by_id(entry.cell().sheet_id())
                .map_or("<unknown>", |sheet| sheet.name().as_str());
            for issue in issues {
                let key = (issue.code(), issue.detail().map(str::to_owned));
                *issue_counts.entry(key.clone()).or_default() += 1;
                let samples = issue_samples.entry(key).or_default();
                if samples.len() < MAX_SAMPLES_PER_ISSUE {
                    let formula = workbook
                        .sheet_by_id(entry.cell().sheet_id())
                        .and_then(|sheet| sheet.cell(entry.cell().address()))
                        .and_then(|cell| match cell.content() {
                            CellContent::Formula(formula) => formula.text(),
                            CellContent::Literal(_) => None,
                        })
                        .map_or("<missing formula text>".to_owned(), |formula| {
                            truncate_formula(formula.as_str())
                        });
                    samples.push(format!(
                        "{sheet_name}!{}: ={formula}",
                        entry.cell().address()
                    ));
                }
            }
        }
        for (key @ (code, detail), count) in &issue_counts {
            let rendered_detail = detail
                .as_deref()
                .map_or(String::new(), |detail| format!(" ({detail})"));
            eprintln!("{count:>8} {}{rendered_detail}", code.as_str());
            if let Some(samples) = issue_samples.get(key) {
                for sample in samples {
                    eprintln!("           {sample}");
                }
            }
        }
        return Ok(());
    }

    let options = match second_argument {
        Some(serial) => {
            let serial = serial.to_string_lossy().parse::<f64>()?;
            options.with_today_serial(FiniteNumber::new(serial)?)
        }
        None => options,
    };
    let calculation = calculate_workbook(&workbook, options);
    let mut issue_counts = BTreeMap::<cellrune::CalculationIssueCode, usize>::new();
    for (_, result) in calculation.cells() {
        if let CalculationCellResult::Unavailable(issue) = result {
            *issue_counts.entry(issue.code()).or_default() += 1;
        }
    }
    let unavailable = issue_counts.values().sum::<usize>();
    println!(
        "calculated {} formula cells; {} unavailable",
        calculation.len(),
        unavailable
    );
    for (code, count) in issue_counts {
        eprintln!("{count:>8} {}", code.as_str());
    }

    Ok(())
}

fn truncate_formula(formula: &str) -> String {
    let mut characters = formula.chars();
    let truncated = characters
        .by_ref()
        .take(MAX_FORMULA_SAMPLE_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
