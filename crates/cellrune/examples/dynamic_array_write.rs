use std::error::Error;

use cellrune::{
    CalculationOptions, CellAddress, FormulaText, RecalculationWriteOptions, WorkbookDraft,
    calculate_workbook, write_xlsx_draft_path,
};

fn main() -> Result<(), Box<dyn Error>> {
    let destination = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cellrune-dynamic-array.xlsx".to_owned());
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft.set_cell_dynamic_formula(
        sheet_id,
        CellAddress::from_a1("A1")?,
        FormulaText::from_user_input("=FILTER({1,10;2,20;3,30},{1;0;1})")?,
        None,
    )?;

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let report = write_xlsx_draft_path(
        &draft,
        &calculation,
        destination,
        RecalculationWriteOptions::default(),
    )?;
    println!(
        "wrote {} dynamic-array result cells (complete: {})",
        report.materialized_count(),
        report.is_complete()
    );
    Ok(())
}
