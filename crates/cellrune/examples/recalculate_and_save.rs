use std::env;
use std::error::Error;
use std::path::PathBuf;

use cellrune::{
    CalculationOptions, OpenOptions, RecalculationWriteOptions, calculate_workbook,
    open_xlsx_document_path, write_recalculated_xlsx_path,
};

const USAGE: &str = "usage: recalculate_and_save <input.xlsx|input.xlsm> <output.xlsx|output.xlsm>";

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let input = arguments.next().map(PathBuf::from).ok_or(USAGE)?;
    let output = arguments.next().map(PathBuf::from).ok_or(USAGE)?;
    if arguments.next().is_some() {
        return Err(USAGE.into());
    }

    let document = open_xlsx_document_path(input, OpenOptions::default())?;
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let report = write_recalculated_xlsx_path(
        &document,
        &calculation,
        output,
        RecalculationWriteOptions::default(),
    )?;
    println!(
        "materialized {} cells; complete={}",
        report.materialized_count(),
        report.is_complete()
    );
    Ok(())
}
