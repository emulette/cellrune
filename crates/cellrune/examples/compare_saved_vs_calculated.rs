use std::error::Error;

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellContent,
    ReadOptions, SavedResult, calculate_workbook, read_xlsx_bytes,
};

#[path = "support/mod.rs"]
mod support;

fn main() -> Result<(), Box<dyn Error>> {
    let bytes = support::minimal_workbook_bytes();
    let workbook = read_xlsx_bytes(&bytes, ReadOptions::default())?;
    let sheet = workbook.sheet_by_name("Sheet1").expect("Sheet1 exists");

    let address = CellAddress::from_a1("B1")?;
    let CellContent::Formula(formula) = sheet.cell(address).expect("B1 exists").content() else {
        return Err("B1 must contain a formula".into());
    };
    println!(
        "saved result, straight from the file, untouched by calculation: {:?}",
        formula.saved_result()
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let cell_id = CalculationCellId::new(sheet.id(), address);
    let result = calculation.cell(cell_id).expect("B1 was calculated");
    println!("calculated result, a separate owned snapshot: {result:?}");

    let SavedResult::Present(saved_value) = formula.saved_result() else {
        return Err("this example's fixture always saves a value".into());
    };
    let CalculationCellResult::Value(calculated_value) = result else {
        return Err("this example's fixture always calculates successfully".into());
    };
    assert_eq!(
        saved_value, calculated_value,
        "the producer's saved SUM should agree with CellRune's calculation"
    );
    println!("saved and calculated results agree; reading never rewrote the saved result");

    Ok(())
}
