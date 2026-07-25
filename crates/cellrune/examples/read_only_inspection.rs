use std::error::Error;

use cellrune::{CellContent, ReadOptions, SavedResult, read_xlsx_bytes};

#[path = "support/mod.rs"]
mod support;

fn main() -> Result<(), Box<dyn Error>> {
    let bytes = support::minimal_workbook_bytes();
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())?;

    println!(
        "read {} sheet(s), no calculation performed",
        workbook.sheets().len()
    );
    for diagnostic in workbook.diagnostics() {
        println!(
            "diagnostic [{:?}] {}: {}",
            diagnostic.severity(),
            diagnostic.code().as_str(),
            diagnostic.message()
        );
    }

    let sheet = workbook.sheet_by_name("Sheet1").expect("Sheet1 exists");
    for cell in sheet.cells() {
        match cell.content() {
            CellContent::Literal(value) => {
                println!("{}: literal {value:?}", cell.address());
            }
            CellContent::Formula(formula) => {
                let text = formula.text().map_or("<none>", |text| text.as_str());
                let saved = match formula.saved_result() {
                    SavedResult::Missing => "no saved result".to_owned(),
                    SavedResult::Present(value) => format!("saved result {value:?}"),
                    SavedResult::Invalid(issue) => {
                        format!("invalid saved result: {}", issue.code().as_str())
                    }
                };
                println!("{}: formula ={text} ({saved})", cell.address());
            }
        }
    }

    Ok(())
}
