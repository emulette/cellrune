//! Reads phonetic (ruby) annotations the way a consumer converting them into its own model does.
//!
//! `phonetic_cell_entries` walks only the cells that carry runs, and `resolved_runs` translates
//! each run's range from the UTF-16 code units XLSX stores into the byte offsets Rust slices with.
//! Presentation state does not hold cell text, so the base text is joined in from the workbook
//! snapshot using the yielded address.

use std::env;
use std::error::Error;
use std::path::PathBuf;

use cellrune::{CellContent, CellValue, OpenOptions, open_xlsx_document_path};

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

        for (address, phonetics) in document.presentation().phonetic_cell_entries(sheet.id()) {
            // The annotated string lives in the workbook, not in presentation state.
            let Some(CellContent::Literal(CellValue::Text(base_text))) =
                sheet.cell(address).map(|cell| cell.content())
            else {
                println!(
                    "{address} visible={} <no base text>",
                    phonetics.effective_visibility()
                );
                continue;
            };

            // A range that would split a surrogate pair or run past the base text is rejected here
            // rather than producing a byte offset that panics when it is used to slice.
            let resolved = match phonetics.resolved_runs(base_text) {
                Ok(resolved) => resolved,
                Err(error) => {
                    eprintln!("{address}: phonetic range does not fit its base text: {error}");
                    continue;
                }
            };
            let runs = resolved
                .iter()
                .map(|run| {
                    let bytes = run.base_bytes();
                    format!(
                        "{}..{}=\"{}\"->{}",
                        bytes.start,
                        bytes.end,
                        run.base_slice(),
                        run.text()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{address} visible={} runs={runs}",
                phonetics.effective_visibility()
            );
        }
    }
    Ok(())
}
