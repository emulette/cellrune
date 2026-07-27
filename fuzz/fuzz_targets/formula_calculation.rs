#![no_main]

use cellrune::{
    CalculationHints, CalculationOptions, CellAddress, CellContent, DateSystem, FormulaCell,
    FormulaDialect, FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet,
    SheetId, SheetName, SheetVisibility, WorkbookSnapshot, WorkbookSource, calculate_workbook,
    scan_formula_capabilities,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(formula) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(formula) = FormulaText::from_xlsx(formula) else {
        return;
    };
    let workbook = workbook_with_formula(formula);
    let _ = scan_formula_capabilities(&workbook);
    let _ = calculate_workbook(&workbook, CalculationOptions::default());

    if let Some(formula) = nested_scope_formula(data) {
        let workbook = workbook_with_formula(formula);
        let _ = scan_formula_capabilities(&workbook);
        let _ = calculate_workbook(&workbook, CalculationOptions::default());
    }
});

fn nested_scope_formula(data: &[u8]) -> Option<FormulaText> {
    let mut body = "seed".to_owned();
    for (depth, byte) in data.iter().take(8).enumerate().rev() {
        let parameter = if byte & 1 == 0 { "seed" } else { "item" };
        let constant = byte % 10;
        body = format!("MAP({{{constant},{depth}}},LAMBDA(_xlpm.{parameter},{parameter}+{body}))");
    }
    FormulaText::from_xlsx(body).ok()
}

fn workbook_with_formula(formula: FormulaText) -> WorkbookSnapshot {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("constant sheet name"),
        SheetVisibility::Visible,
    );
    sheet
        .insert_cell(
            CellAddress::from_indices(1, 1).expect("constant cell address"),
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                formula,
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        )
        .expect("unique constant cell");
    WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("cellrune-fuzz", "1").expect("constant provider"),
            None,
        ),
    )
    .expect("valid fuzz workbook")
}
