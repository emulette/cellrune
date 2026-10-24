//! Verification for the 0.1.4 Excel-saved-cache boundary of the calculation compatibility modes.
//!
//! Three tiers, in the order the roadmap fixes:
//!
//! 1. A committed oracle decides what is correct. Arithmetic uses values observed in Excel's saved
//!    cache; solvers use the residual, which is closed-form even though the root is not.
//! 2. Metamorphic invariants that need no expected value at all, and so cover inputs no table
//!    enumerates.
//! 3. Excel's recorded behaviour, which only the existing conformance corpus can speak to, and
//!    which is not re-litigated here.

use cellrune::{
    ArithmeticSemantics, CalculationCellId, CalculationCellResult, CalculationHints,
    CalculationOptions, CellAddress, CellContent, CellValue, DateSystem, FinancialSolverSemantics,
    FiniteNumber, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, Provenance,
    ProviderIdentity, SavedResult, Sheet, SheetId, SheetName, SheetVisibility, WorkbookSnapshot,
    WorkbookSource, calculate_workbook,
};
use cellrune_integration_tests::financial_reference::{
    bisect_root, irr_residual, rate_residual, xirr_residual,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalculationModesManifest {
    schema: String,
    arithmetic_cases: Vec<ArithmeticCase>,
    solver_cases: Vec<SolverCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArithmeticCase {
    id: String,
    formula: String,
    oracle: ArithmeticOracle,
    expected_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArithmeticOracle {
    ExcelSavedCache,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SolverFunction {
    Irr,
    Xirr,
    Rate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SolverCase {
    id: String,
    function: SolverFunction,
    formula: String,
    oracle: SolverOracle,
    cashflows: Vec<f64>,
    dates: Vec<f64>,
    periods: f64,
    payment: f64,
    present: f64,
    future: f64,
    payment_type: f64,
    lower: f64,
    upper: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SolverOracle {
    ResidualAndBisection,
}

impl SolverCase {
    fn residual(&self, rate: f64) -> f64 {
        match self.function {
            SolverFunction::Irr => irr_residual(&self.cashflows, rate),
            SolverFunction::Xirr => xirr_residual(&self.cashflows, &self.dates, rate),
            SolverFunction::Rate => rate_residual(
                self.periods,
                self.payment,
                self.present,
                self.future,
                self.payment_type,
                rate,
            ),
        }
    }

    fn residual_scale(&self) -> f64 {
        match self.function {
            SolverFunction::Irr | SolverFunction::Xirr => self
                .cashflows
                .iter()
                .copied()
                .map(f64::abs)
                .fold(1.0, f64::max),
            SolverFunction::Rate => self
                .present
                .abs()
                .max(self.future.abs())
                .max(self.payment.abs())
                .max(1.0),
        }
    }
}

fn calculation_modes_manifest() -> CalculationModesManifest {
    serde_json::from_str(include_str!("golden/calculation_modes.json"))
        .expect("calculation mode manifest must stay valid")
}

fn workbook_with_formulas(formulas: &[&str]) -> WorkbookSnapshot {
    let sheet_id = SheetId::new(1).expect("sheet id");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (index, formula) in formulas.iter().enumerate() {
        let column = u32::try_from(index + 1).expect("column fits");
        sheet
            .insert_cell(
                CellAddress::from_indices(1, column).expect("address"),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_user_input(format!("={formula}")).expect("formula"),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("unique cell");
    }
    WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("cellrune-modes", "1").expect("provider"),
            None,
        ),
    )
    .expect("workbook")
}

fn numbers(formulas: &[&str], options: CalculationOptions) -> Vec<Option<f64>> {
    let workbook = workbook_with_formulas(formulas);
    let calculation = calculate_workbook(&workbook, options);
    (1..=formulas.len())
        .map(|index| {
            let column = u32::try_from(index).expect("column fits");
            let id = CalculationCellId::new(
                SheetId::new(1).expect("sheet id"),
                CellAddress::from_indices(1, column).expect("address"),
            );
            match calculation.cell(id) {
                Some(CalculationCellResult::Value(CellValue::Number(value))) => Some(value.get()),
                _ => None,
            }
        })
        .collect()
}

fn conditional_aggregate_workbook(values: [f64; 3]) -> WorkbookSnapshot {
    let sheet_id = SheetId::new(1).expect("sheet id");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    for (row, value) in values.into_iter().enumerate() {
        sheet
            .insert_cell(
                CellAddress::from_indices(u32::try_from(row + 1).expect("row"), 1)
                    .expect("address"),
                CellContent::Literal(CellValue::Number(
                    FiniteNumber::new(value).expect("finite fixture"),
                )),
            )
            .expect("unique cell");
    }
    for (row, formula) in [
        "SUM(A1:A3)",
        "SUMIF(A1:A3,\">-1000\")",
        "AVERAGE(A1:A3)",
        "AVERAGEIF(A1:A3,\">-1000\")",
    ]
    .into_iter()
    .enumerate()
    {
        sheet
            .insert_cell(
                CellAddress::from_indices(u32::try_from(row + 1).expect("row"), 2)
                    .expect("address"),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_user_input(format!("={formula}")).expect("formula"),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("unique cell");
    }
    WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("cellrune-conditional-modes", "1").expect("provider"),
            None,
        ),
    )
    .expect("workbook")
}

fn excel_mode() -> CalculationOptions {
    CalculationOptions::default().with_arithmetic_semantics(ArithmeticSemantics::ExcelNearZero)
}

fn ieee_mode() -> CalculationOptions {
    CalculationOptions::default().with_arithmetic_semantics(ArithmeticSemantics::Ieee754)
}

/// Results Excel 16.0 build 20326 returns, read back as doubles, for formulas whose additions and
/// subtractions cancel to near zero. Excel corrects only the addition or subtraction a formula ends
/// with, unparenthesized, and the final addition of `SUM`, `AVERAGE`, `SUBTOTAL`, and `AGGREGATE`;
/// a result below `2^(e-49)` of the larger operand in `[2^e, 2^(e+1))` becomes zero.
const EXCEL_MEASUREMENTS: &[(&str, f64)] = &[
    ("0.1 + 0.2 - 0.3", 0.0),
    ("0.5 - 0.4 - 0.1", 0.0),
    ("1.1 - 1.0 - 0.1", 0.0),
    ("0.7 + 0.1 - 0.8", 0.0),
    ("4.56 - 1.23 - 3.33", 0.0),
    ("1 - 0.9 - 0.1", 0.0),
    ("0.3 - 0.2 - 0.1", 0.0),
    ("-0.3 + 0.1 + 0.2", 0.0),
    ("1E+15 + 0.3 - 1E+15", 0.0),
    ("1 + 7*2^-52 - 1", 0.0),
    ("1 + 8*2^-52 - 1", 1.776_356_839_400_250_5e-15),
    ("0.5 + 7*2^-53 - 0.5", 0.0),
    ("0.5 + 8*2^-53 - 0.5", 8.881_784_197_001_252e-16),
    ("3 + 4*2^-51 - 3", 0.0),
    ("3 + 8*2^-51 - 3", 3.552_713_678_800_501e-15),
    ("100.1 - 100.0 - 0.1", -5.689_893_001_203_927e-15),
    (
        "100.1 - 100 - 0.099999999999999",
        -4.690_692_279_041_286e-15,
    ),
    ("1.0000000000001 - 1.0", 9.992_007_221_626_409e-14),
    // A formula keeps fifteen significant digits of a literal, so this is 1 - 1.
    ("1.0000000000000004 - 1.0", 0.0),
    // Not the root addition: parenthesized, multiplied, adding zero, or inside a function.
    ("(0.1 + 0.2 - 0.3)", 5.551_115_123_125_783e-17),
    ("(0.1 + 0.2 - 0.3) * 1E+17", 5.551_115_123_125_783),
    ("0.1 + 0.2 - 0.3 + 0*1", 5.551_115_123_125_783e-17),
    ("IF(TRUE, 0.1 + 0.2 - 0.3, 1)", 5.551_115_123_125_783e-17),
    ("LET(x, 1, 0.1 + 0.2 - 0.3)", 5.551_115_123_125_783e-17),
    ("ABS(0.1 + 0.2 - 0.3)", 5.551_115_123_125_783e-17),
    // Aggregates correct their final addition only.
    ("SUM(0.1, 0.2, -0.3)", 0.0),
    ("SUM(0.1, 0.2, -0.3) * 1E+17", 0.0),
    ("SUM(-0.3, 0.1, 0.2) * 1E+17", 0.0),
    ("SUM(0.3, -0.1, -0.2) * 1E+17", 0.0),
    ("SUM(0.1, 0.2, -0.3, 0) * 1E+17", 5.551_115_123_125_783),
    ("SUM(1E+15, 0.3, -1E+15)", 0.0),
    ("SUM(100.1, -100, -0.1)", -5.689_893_001_203_927e-15),
    ("SUM({0.1, 0.2, -0.3}) * 1E+17", 0.0),
    (
        "SUM({0.1, 0.5} + {0.2, 0.5} - {0.3, 1})",
        5.551_115_123_125_783e-17,
    ),
    ("SUM({0.1} + 0.2 - 0.3) * 1E+17", 5.551_115_123_125_783),
    ("AVERAGE(0.1, 0.2, -0.3) * 1E+17", 0.0),
    (
        "AVERAGE(0.1, 0.2, -0.3, 0) * 1E+17",
        1.387_778_780_781_445_7,
    ),
    ("AVERAGEA(0.1, 0.2, -0.3) * 1E+17", 0.0),
    // These totals are not corrected.
    ("SUMPRODUCT({0.1, 0.2, -0.3})", 5.551_115_123_125_783e-17),
    ("SUMPRODUCT({0.1, -0.3}, {3, 1})", 5.551_115_123_125_783e-17),
    ("NPV(0, 0.1, 0.2, -0.3) * 1E+17", 5.551_115_123_125_783),
];

/// TIER 1 — every measured formula returns Excel's double bit for bit.
#[test]
fn excel_mode_matches_excel_measurements() {
    let formulas = EXCEL_MEASUREMENTS
        .iter()
        .map(|(formula, _)| *formula)
        .collect::<Vec<_>>();
    let results = numbers(&formulas, excel_mode());
    for ((formula, expected), result) in EXCEL_MEASUREMENTS.iter().zip(results) {
        let result = result.unwrap_or_else(|| panic!("{formula} produced no number"));
        assert_eq!(
            result.to_bits(),
            expected.to_bits(),
            "{formula}: expected {expected:e}, got {result:e}"
        );
    }
}

/// TIER 1 — the opt-in mode keeps the residue of every cancellation.
#[test]
fn ieee_mode_keeps_the_residue_for_chains_that_cancel() {
    let residue = 0.1_f64 + 0.2 - 0.3;
    let results = numbers(
        &[
            "0.1 + 0.2 - 0.3",
            "SUM(0.1, 0.2, -0.3)",
            "AVERAGE(0.1, 0.2, -0.3) * 3",
        ],
        ieee_mode(),
    );
    for result in results {
        assert_eq!(result, Some(residue));
    }
}

/// TIER 1 — the root of a dependent formula is corrected with the operands it reads.
#[test]
fn cancellation_boundary_is_preserved_across_formula_cells() {
    let preserved = ["100.1 - 100.0", "A1 - 0.1"];
    let excel = numbers(&preserved, excel_mode());
    let ieee = numbers(&preserved, ieee_mode());
    assert_eq!(excel[1].map(f64::to_bits), ieee[1].map(f64::to_bits));

    let corrected = ["0.1 + 0.2", "A1 - 0.3"];
    let excel = numbers(&corrected, excel_mode());
    let ieee = numbers(&corrected, ieee_mode());
    assert_eq!(excel[1], Some(0.0));
    assert!(
        ieee[1].is_some_and(|value| value != 0.0),
        "the IEEE path must preserve the smaller inherited residue"
    );
}

/// TIER 1 — every committed arithmetic case matches an Excel saved-cache observation.
#[test]
fn generated_arithmetic_manifest_matches_the_excel_observations() {
    let manifest = calculation_modes_manifest();
    assert_eq!(manifest.schema, "cellrune_calculation_modes_v2");
    let formulas: Vec<&str> = manifest
        .arithmetic_cases
        .iter()
        .map(|case| case.formula.as_str())
        .collect();
    let results = numbers(&formulas, excel_mode());
    for (case, result) in manifest.arithmetic_cases.iter().zip(results) {
        assert_eq!(case.oracle, ArithmeticOracle::ExcelSavedCache);
        let result = result.unwrap_or_else(|| panic!("{} produced no number", case.id));
        assert_eq!(
            result == 0.0,
            case.expected_zero,
            "{}: result {result:e} disagrees with the Excel saved-cache observation",
            case.id,
        );
    }
}

/// TIER 2 — metamorphic. The corrected result is either the IEEE result or exactly zero.
///
/// A third value would mean the correction altered a number rather than discarding a residue,
/// which is the failure mode that silently corrupts arithmetic.
#[test]
fn the_excel_mode_only_ever_replaces_a_result_with_zero() {
    let formulas = EXCEL_MEASUREMENTS
        .iter()
        .map(|(formula, _)| *formula)
        .filter(|formula| !formula.starts_with("SUMPRODUCT") && !formula.starts_with("NPV"))
        .collect::<Vec<_>>();
    let excel = numbers(&formulas, excel_mode());
    let ieee = numbers(&formulas, ieee_mode());
    for ((formula, excel), ieee) in formulas.iter().zip(excel).zip(ieee) {
        let (excel, ieee) = (
            excel.unwrap_or_else(|| panic!("{formula} produced no number")),
            ieee.unwrap_or_else(|| panic!("{formula} produced no number")),
        );
        assert!(
            excel == ieee || excel == 0.0,
            "{formula}: Excel mode returned {excel:e}, which is neither the IEEE result {ieee:e} nor zero"
        );
    }
}

/// TIER 2 — metamorphic. Results far from cancellation are bit-identical across the two modes.
#[test]
fn results_away_from_cancellation_are_bit_identical_across_modes() {
    const AWAY: &[&str] = &[
        "1.1 - 1.0",
        "2.0 * 3.0",
        "10.0 / 4.0",
        "2.0 ^ 10.0",
        "SUM(1.5,2.5,3.5)",
        "AVERAGE(1.0,2.0,4.0)",
        "1000000.0 + 0.25",
    ];
    let excel = numbers(AWAY, excel_mode());
    let ieee = numbers(AWAY, ieee_mode());
    for ((formula, excel), ieee) in AWAY.iter().zip(excel).zip(ieee) {
        assert_eq!(
            excel.map(f64::to_bits),
            ieee.map(f64::to_bits),
            "{formula} differs between modes"
        );
    }
}

/// TIER 1 — `SUM` and `AVERAGE` correct their totals over cells; `SUMIF` and `AVERAGEIF` do not
/// (Excel 16.0 build 20326).
#[test]
fn ordinary_aggregates_correct_their_totals_and_conditional_ones_do_not() {
    let workbook = conditional_aggregate_workbook([0.1, 0.2, -0.3]);
    let residue = 0.1_f64 + 0.2 - 0.3;
    for (options, corrected) in [(excel_mode(), true), (ieee_mode(), false)] {
        let calculation = calculate_workbook(&workbook, options);
        let results: Vec<f64> = (1..=4)
            .map(|row| {
                let id = CalculationCellId::new(
                    SheetId::new(1).expect("sheet id"),
                    CellAddress::from_indices(row, 2).expect("address"),
                );
                match calculation.cell(id) {
                    Some(CalculationCellResult::Value(CellValue::Number(value))) => value.get(),
                    other => panic!("B{row} did not produce a number: {other:?}"),
                }
            })
            .collect();
        let total = if corrected { 0.0 } else { residue };
        assert_eq!(results, [total, residue, total / 3.0, residue / 3.0]);
    }
}

/// TIER 1 — the solvers, judged by residual rather than by a recorded root.
///
/// A rate `r` solves `IRR` exactly when the discounted cash flows sum to zero at `r`. That check
/// is closed form even though the root is not, so correctness here needs no oracle: whatever the
/// search returns is verified against the equation it claims to have solved.
#[test]
fn whatever_the_solvers_return_actually_solves_the_equation() {
    const CASHFLOWS: [f64; 5] = [-100.0, 30.0, 35.0, 40.0, 45.0];
    let formula = "IRR({-100,30,35,40,45})";
    for options in [
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExcelIterationBudget),
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch),
    ] {
        let rate = numbers(&[formula], options)[0].expect("IRR converges for this input");
        let residual: f64 = CASHFLOWS
            .iter()
            .enumerate()
            .map(|(period, flow)| flow / (1.0 + rate).powi(i32::try_from(period).expect("period")))
            .sum();
        assert!(
            residual.abs() < 1e-6,
            "returned rate {rate} leaves residual {residual:e}"
        );
    }
}

/// TIER 1 — committed financial cases agree with an independent derivative-free search.
#[test]
fn generated_solver_manifest_matches_residual_and_bisection_oracles() {
    let manifest = calculation_modes_manifest();
    let formulas: Vec<&str> = manifest
        .solver_cases
        .iter()
        .map(|case| case.formula.as_str())
        .collect();
    for options in [
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExcelIterationBudget),
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch),
    ] {
        let results = numbers(&formulas, options);
        for (case, result) in manifest.solver_cases.iter().zip(results) {
            assert_eq!(case.oracle, SolverOracle::ResidualAndBisection);
            let reference = bisect_root(case.lower, case.upper, 1e-12, |rate| case.residual(rate))
                .unwrap_or_else(|| panic!("{} does not provide a valid root bracket", case.id));
            let result = result.unwrap_or_else(|| panic!("{} did not converge", case.id));
            let normalized_residual = case.residual(result).abs() / case.residual_scale();
            assert!(
                normalized_residual <= 1e-8,
                "{}: result {result} leaves normalized residual {normalized_residual:e}",
                case.id
            );
            assert!(
                (result - reference).abs() <= reference.abs() * 1e-7 + 1e-8,
                "{}: Newton result {result} disagrees with bisection reference {reference}",
                case.id
            );
        }
    }
}

/// TIER 2 — the default budget is selected per function, rather than once for all solvers.
#[test]
fn documented_solver_budgets_are_function_specific() {
    let options = CalculationOptions::default()
        .with_financial_solver_semantics(FinancialSolverSemantics::ExcelIterationBudget);
    let results = numbers(
        &["IRR({-1,100000})", "XIRR({-1,100000},{45000,45365})"],
        options,
    );
    assert_eq!(results[0], None, "IRR must stop after its 20-try budget");
    assert!(
        results[1].is_some(),
        "XIRR must retain its documented 100-try budget"
    );
}

/// TIER 2 — metamorphic. The two solver policies differ in budget, not in which root they find.
#[test]
fn both_solver_policies_find_the_same_root_when_both_converge() {
    const FORMULAS: &[&str] = &[
        "IRR({-100,30,35,40,45})",
        "IRR({-1000,500,400,300,200})",
        "RATE(10,-100,800)",
        "XIRR({-100,120},{45000,45365})",
    ];
    let budgeted = numbers(
        FORMULAS,
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExcelIterationBudget),
    );
    let extended = numbers(
        FORMULAS,
        CalculationOptions::default()
            .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch),
    );
    let mut compared = 0;
    for ((formula, budgeted), extended) in FORMULAS.iter().zip(budgeted).zip(extended) {
        let (Some(budgeted), Some(extended)) = (budgeted, extended) else {
            // One policy giving up where the other does not is the documented difference between
            // them, not a disagreement about the answer.
            continue;
        };
        compared += 1;
        assert!(
            (budgeted - extended).abs() <= extended.abs() * 1e-6 + 1e-9,
            "{formula}: budgeted search found {budgeted}, extended search found {extended}"
        );
    }
    assert!(
        compared > 0,
        "no formula converged under both policies, so this invariant checked nothing"
    );
}

/// The default is the Excel-compatible mode on both axes, and 0.1.2's behaviour remains reachable.
#[test]
fn defaults_select_excel_semantics_on_both_axes() {
    let defaults = CalculationOptions::default();
    assert_eq!(
        defaults.arithmetic_semantics(),
        ArithmeticSemantics::ExcelNearZero
    );
    assert_eq!(
        defaults.financial_solver_semantics(),
        FinancialSolverSemantics::ExcelIterationBudget
    );
    assert_eq!(numbers(&["0.1 + 0.2 - 0.3"], defaults)[0], Some(0.0));

    let compatible = defaults
        .with_arithmetic_semantics(ArithmeticSemantics::Ieee754)
        .with_financial_solver_semantics(FinancialSolverSemantics::ExtendedSearch);
    assert_ne!(numbers(&["0.1 + 0.2 - 0.3"], compatible)[0], Some(0.0));
}
