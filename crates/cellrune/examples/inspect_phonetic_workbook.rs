use std::env;
use std::error::Error;
use std::path::PathBuf;

use cellrune::{OpenOptions, open_xlsx_document_path};

fn main() -> Result<(), Box<dyn Error>> {
    let input = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: inspect_phonetic_workbook <input.xlsx>")?;
    let document = open_xlsx_document_path(input, OpenOptions::default())?;
    for sheet in document.workbook().sheets() {
        println!("sheet={}", sheet.name().as_str());
        if let Some(pane) = document.presentation().frozen_pane(sheet.id()) {
            println!(
                "pane rows={} columns={}",
                pane.frozen_rows(),
                pane.frozen_columns()
            );
        }
        for cell in sheet.cells() {
            let Some(phonetics) = document
                .presentation()
                .cell_phonetics(sheet.id(), cell.address())
            else {
                continue;
            };
            let ranges = phonetics
                .runs()
                .iter()
                .map(|run| {
                    format!(
                        "{}..{}={}",
                        run.base_range().start_utf16(),
                        run.base_range().end_utf16(),
                        run.text()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{} visible={} runs={}",
                cell.address(),
                phonetics.effective_visibility(),
                ranges
            );
        }
    }
    Ok(())
}
