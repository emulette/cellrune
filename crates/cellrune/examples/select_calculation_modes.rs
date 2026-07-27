//! The two numeric compatibility axes, shown by calculating the same cells under both settings.
//!
//! Both default to matching Excel rather than to what releases up to 0.1.2 did, so upgrading
//! changes calculated numbers unless the 0.1.2 policies are selected explicitly. This example
//! builds its own workbook so the difference is visible without an input file.

use std::error::Error;

use cellrune::{
    ArithmeticSemantics, CalculationCellId, CalculationCellResult, CalculationOptions,
    CalculationSnapshot, CellAddress, CellValue, FinancialSolverSemantics, FormulaText,
    WorkbookDraft, calculate_workbook,
};

/// Formulas that expose Excel's narrow correction boundary, plus one real difference.
///
/// The first group is corrected, the sixth row cancels exactly but lies outside Excel's observed
/// binary boundary, and the last row is a real difference that must not be swallowed.
const ARITHMETIC: &[(&str, &str)] = &[
    ("A1", "=0.1+0.2-0.3"),
    ("A2", "=SUM(0.1,0.2,-0.3)"),
    ("A3", "=SUMPRODUCT({0.1,0.2,-0.3})"),
    ("A4", "=NPV(0.1,11,-12.1)"),
    ("A5", "=(0.1+0.2-0.3)=0"),
    ("A6", "=100.1-100-0.1"),
    ("A7", "=100.1-100-0.099999999999999"),
];

/// An `IRR` whose answer depends on how long the solver is allowed to search.
const SOLVER: &[(&str, &str)] = &[("B1", "=IRR({-100,30,35,40,45})")];

fn main() -> Result<(), Box<dyn Error>> {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in ARITHMETIC.iter().chain(SOLVER) {
        draft.set_cell_formula(
            sheet_id,
            CellAddress::from_a1(address)?,
            FormulaText::from_user_input(*formula)?,
        )?;
    }

    // Defaults on both axes. Nothing has to be selected to match Excel.
    let excel = CalculationOptions::default();
    // What 0.1.2 did, and what a caller who depended on those numbers should select.
    let legacy = CalculationOptions::default()
        .with_arithmetic_semantics(ArithmeticSemantics::Ieee754)
        .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch);

    let excel_results = calculate_workbook(draft.workbook(), excel);
    let legacy_results = calculate_workbook(draft.workbook(), legacy);

    println!("{:<34} {:<26} 0.1.2 opt-in", "formula", "default (Excel)");
    for (address, formula) in ARITHMETIC.iter().chain(SOLVER) {
        let cell = CalculationCellId::new(sheet_id, CellAddress::from_a1(address)?);
        println!(
            "{formula:<34} {:<26} {}",
            render(&excel_results, cell),
            render(&legacy_results, cell)
        );
    }

    println!();
    println!(
        "The correction requires both exact cancellation and Excel's observed relative binary \
         boundary. The final two arithmetic rows are therefore identical under both policies. \
         Compare calculated numbers with a tolerance rather than for equality under either one; \
         see docs/NUMERICS.md."
    );
    Ok(())
}

fn render(calculation: &CalculationSnapshot, cell: CalculationCellId) -> String {
    match calculation.cell(cell) {
        Some(CalculationCellResult::Value(CellValue::Number(number))) => {
            render_number(number.get())
        }
        Some(CalculationCellResult::Value(CellValue::Logical(logical))) => logical.to_string(),
        Some(CalculationCellResult::Value(CellValue::Error(error))) => error.as_str().to_owned(),
        Some(CalculationCellResult::Unavailable(issue)) => {
            format!("unavailable: {}", issue.code().as_str())
        }
        // `CellValue` and `CalculationCellResult` are `#[non_exhaustive]`, so a wildcard arm is
        // required even when every variant this example can produce is handled above.
        Some(_) => "<other value>".to_owned(),
        None => "<not calculated>".to_owned(),
    }
}

/// Renders enough digits that a residue is visible rather than rounded away in the output.
fn render_number(value: f64) -> String {
    if value != 0.0 && value.abs() < 1e-6 {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
}
