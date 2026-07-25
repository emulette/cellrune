use std::error::Error;

use cellrune::{FormulaCapability, ReadOptions, read_xlsx_bytes, scan_formula_capabilities};

#[path = "support/mod.rs"]
mod support;

fn main() -> Result<(), Box<dyn Error>> {
    let bytes = support::minimal_workbook_bytes();
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())?;

    let capabilities = scan_formula_capabilities(&workbook);
    println!(
        "{} of {} formulas are statically supported",
        capabilities.supported_count(),
        capabilities.formula_count()
    );

    for entry in capabilities.entries() {
        match entry.capability() {
            FormulaCapability::Supported => {
                println!("{}: supported", entry.cell().address());
            }
            FormulaCapability::Unsupported(issues) => {
                for issue in issues {
                    let detail = issue
                        .detail()
                        .map_or_else(String::new, |detail| format!(": {detail}"));
                    println!(
                        "{}: unsupported ({}{detail})",
                        entry.cell().address(),
                        issue.code().as_str()
                    );
                }
            }
        }
    }

    if capabilities.is_supported() {
        println!("safe to call calculate_workbook next");
    } else {
        println!(
            "calculate_workbook is still safe to call: unsupported formulas report a \
             CalculationIssue instead of a wrong value"
        );
    }

    Ok(())
}
