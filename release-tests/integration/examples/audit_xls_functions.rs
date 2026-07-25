use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use calamine::{Reader, open_workbook_auto};

const CORPUS_ENV: &str = "CELLRUNE_XLS_CORPUS";
const ERROR_NO_XLS_FILES: &str = "no .xls files found in the requested inputs";
const ERROR_MISSING_INPUT: &str = "pass an .xls path/directory or set CELLRUNE_XLS_CORPUS";
const ERROR_UNSUPPORTED_INPUT: &str = "input must be an .xls file or a directory";
const SAMPLE_LIMIT: usize = 3;

#[derive(Debug, Default)]
struct Audit {
    requested_file_count: usize,
    readable_file_count: usize,
    formula_count: usize,
    functions: BTreeMap<String, FunctionUsage>,
    issues: Vec<AuditIssue>,
}

#[derive(Debug, Default)]
struct FunctionUsage {
    count: usize,
    samples: Vec<String>,
}

#[derive(Debug)]
struct AuditIssue {
    source: String,
    error: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let inputs = env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let inputs = if inputs.is_empty() {
        vec![
            env::var_os(CORPUS_ENV)
                .map(PathBuf::from)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, ERROR_MISSING_INPUT))?,
        ]
    } else {
        inputs
    };
    let files = collect_xls_files(&inputs)?;
    let audit = audit_files(&files);
    print_audit(&audit);
    Ok(())
}

fn collect_xls_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_file() {
            if is_xls(input) {
                files.push(input.clone());
            } else {
                return Err(
                    io::Error::new(io::ErrorKind::InvalidInput, ERROR_UNSUPPORTED_INPUT).into(),
                );
            }
        } else if input.is_dir() {
            for entry in fs::read_dir(input)? {
                let path = entry?.path();
                if path.is_file() && is_xls(&path) {
                    files.push(path);
                }
            }
        } else {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, ERROR_UNSUPPORTED_INPUT).into(),
            );
        }
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, ERROR_NO_XLS_FILES).into());
    }
    Ok(files)
}

fn is_xls(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xls"))
}

fn audit_files(files: &[PathBuf]) -> Audit {
    let mut audit = Audit {
        requested_file_count: files.len(),
        ..Audit::default()
    };
    for path in files {
        let mut workbook = match open_workbook_auto(path) {
            Ok(workbook) => workbook,
            Err(error) => {
                audit.issues.push(AuditIssue {
                    source: path.display().to_string(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        audit.readable_file_count += 1;
        for sheet_name in workbook.sheet_names().to_vec() {
            let formulas = match workbook.worksheet_formula(&sheet_name) {
                Ok(formulas) => formulas,
                Err(error) => {
                    audit.issues.push(AuditIssue {
                        source: format!("{} :: {sheet_name}", path.display()),
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            let start = formulas.start().unwrap_or((0, 0));
            for (row, column, formula) in formulas.cells() {
                if formula.is_empty() {
                    continue;
                }
                audit.formula_count += 1;
                let location = format!(
                    "{}!{}{}",
                    sheet_name,
                    column_name(start.1 + column as u32 + 1),
                    start.0 + row as u32 + 1
                );
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?");
                for function in formula_functions(formula) {
                    let usage = audit.functions.entry(function).or_default();
                    usage.count += 1;
                    if usage.samples.len() < SAMPLE_LIMIT {
                        usage
                            .samples
                            .push(format!("{file_name} :: {location} :: ={formula}"));
                    }
                }
            }
        }
    }
    audit
}

fn formula_functions(formula: &str) -> Vec<String> {
    let bytes = formula.as_bytes();
    let mut functions = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = skip_quoted(bytes, index, b'"'),
            b'\'' => index = skip_quoted(bytes, index, b'\''),
            byte if is_identifier_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let mut next = index;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if next < bytes.len() && bytes[next] == b'(' {
                    functions.push(normalize_function_name(&formula[start..index]));
                }
            }
            _ => index += 1,
        }
    }
    functions
}

fn skip_quoted(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] != quote {
            index += 1;
            continue;
        }
        if index + 1 < bytes.len() && bytes[index + 1] == quote {
            index += 2;
            continue;
        }
        return index + 1;
    }
    index
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
}

fn normalize_function_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    upper
        .strip_prefix("_XLFN.")
        .or_else(|| upper.strip_prefix("_XLUDF."))
        .unwrap_or(&upper)
        .to_owned()
}

fn column_name(mut column: u32) -> String {
    let mut reversed = Vec::new();
    while column > 0 {
        column -= 1;
        reversed.push((b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    reversed.into_iter().rev().collect()
}

fn print_audit(audit: &Audit) {
    println!("requested files: {}", audit.requested_file_count);
    println!("readable files: {}", audit.readable_file_count);
    println!("formulas: {}", audit.formula_count);
    println!("unique functions: {}", audit.functions.len());
    for (function, usage) in &audit.functions {
        println!("\n{function}: {}", usage.count);
        for sample in &usage.samples {
            println!("  {sample}");
        }
    }
    if !audit.issues.is_empty() {
        println!("\nissues: {}", audit.issues.len());
        for issue in &audit.issues {
            println!("  {} :: {}", issue.source, issue.error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{column_name, formula_functions};

    #[test]
    fn finds_functions_without_counting_quoted_text_or_sheet_names() {
        assert_eq!(
            formula_functions("IF('A(B)'!A1=\"SUM(C1)\",_xlfn.NORM.S.DIST(A1,TRUE),MyUdf (A2))"),
            ["IF", "NORM.S.DIST", "MYUDF"]
        );
    }

    #[test]
    fn renders_excel_column_names() {
        assert_eq!(column_name(1), "A");
        assert_eq!(column_name(26), "Z");
        assert_eq!(column_name(27), "AA");
        assert_eq!(column_name(16_384), "XFD");
    }
}
