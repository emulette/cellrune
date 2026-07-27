#![no_main]

use cellrune::{
    CalculationCellId, CalculationHints, CalculationLimits, CalculationOptions, CellAddress,
    CellContent, CellValue, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
    Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName, SheetVisibility,
    WorkbookSnapshot, WorkbookSource, calculate_workbook, scan_formula_capabilities,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    exercise_three_d_sum_invariant(data);

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

fn exercise_three_d_sum_invariant(data: &[u8]) {
    let sheet_count = usize::from(data.first().copied().unwrap_or(0) % 8) + 1;
    let reverse = data.get(1).is_some_and(|byte| byte & 1 == 1);
    let mut sheets = Vec::with_capacity(sheet_count);
    for index in 0..sheet_count {
        let sheet_number = index + 1;
        let sheet_id = SheetId::new(sheet_number as u32).expect("bounded fuzz sheet ID");
        let visibility = if index == 1 && sheet_count >= 3 {
            SheetVisibility::Hidden
        } else {
            SheetVisibility::Visible
        };
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new(format!("Sheet{sheet_number}")).expect("bounded fuzz sheet name"),
            visibility,
        );
        let raw = data.get(index + 2).copied().unwrap_or(index as u8);
        let value = f64::from(raw) - 128.0;
        sheet
            .insert_cell(
                CellAddress::from_indices(1, 1).expect("constant input address"),
                CellContent::Literal(CellValue::number(value).expect("finite fuzz input")),
            )
            .expect("unique fuzz input");
        sheets.push(sheet);
    }

    let (start, end) = if reverse {
        (sheet_count, 1)
    } else {
        (1, sheet_count)
    };
    let three_d = format!("SUM(Sheet{start}:Sheet{end}!A1)");
    let explicit = format!(
        "SUM({})",
        (1..=sheet_count)
            .map(|sheet| format!("Sheet{sheet}!A1"))
            .collect::<Vec<_>>()
            .join(",")
    );
    insert_formula(&mut sheets[0], 2, three_d);
    insert_formula(&mut sheets[0], 3, explicit);

    let workbook = workbook_from_sheets(sheets);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let limits = CalculationLimits::default()
        .with_max_array_cells(sheet_count as u64)
        .expect("positive bounded span budget");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
    let sheet_id = SheetId::new(1).expect("constant calculation sheet ID");
    let three_d_cell = CalculationCellId::new(
        sheet_id,
        CellAddress::from_indices(1, 2).expect("constant 3-D result address"),
    );
    let explicit_cell = CalculationCellId::new(
        sheet_id,
        CellAddress::from_indices(1, 3).expect("constant explicit result address"),
    );
    assert_eq!(
        calculation.cell(three_d_cell),
        calculation.cell(explicit_cell),
        "3-D SUM must equal the explicit sheet fold",
    );
}

fn insert_formula(sheet: &mut Sheet, column: u32, formula: String) {
    sheet
        .insert_cell(
            CellAddress::from_indices(1, column).expect("bounded formula address"),
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(formula).expect("generated valid formula"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        )
        .expect("unique fuzz formula");
}

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
    workbook_from_sheets(vec![sheet])
}

fn workbook_from_sheets(sheets: Vec<Sheet>) -> WorkbookSnapshot {
    WorkbookSnapshot::new(
        sheets,
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
