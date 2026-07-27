//! Verification for the 0.1.3 calculation compatibility modes.
//!
//! Three tiers, in the order the roadmap fixes:
//!
//! 1. A self-generated oracle decides what is correct. For arithmetic that is exact decimal
//!    evaluation; for the solvers it is the residual, which is closed-form even though the root is
//!    not. Neither needs Excel or another engine.
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
use cellrune_integration_tests::decimal_reference::{evaluate, parse_chain};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArithmeticOracle {
    ExactDecimal,
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

/// Chains whose exact decimal value is zero, and chains whose exact value is not.
///
/// Written with spaces so the same string parses as a formula and as an exact chain — the engine
/// and the reference are given the identical expression rather than two hand-kept copies.
const CHAINS: &[&str] = &[
    // Exact value zero: every residue here is an artefact of binary representation.
    "0.1 + 0.2 - 0.3",
    "0.5 - 0.4 - 0.1",
    "1.1 - 1.0 - 0.1",
    "0.7 + 0.1 - 0.8",
    "4.56 - 1.23 - 3.33",
    "100.1 - 100.0 - 0.1",
    // Exact value nonzero: the mode must leave every one of these alone.
    "1.1 - 1.0",
    "0.3 - 0.1",
    "1.0 + 2.0",
    "0.1 + 0.2",
    "1000000.0 - 0.5",
    "0.0001 - 0.00005",
    "100.1 - 100.0 - 0.099999999999999",
    "1.0000000000000004 - 1.0",
];

/// TIER 1 — the exact decimal reference decides, and the mode must agree with it.
///
/// This is the criterion the roadmap fixes: the snap fires when, and only when, the exact value is
/// zero. Firing more widely would corrupt results the author meant; firing more narrowly would
/// leave the mode not doing its job.
#[test]
fn excel_mode_snaps_exactly_the_chains_whose_exact_value_is_zero() {
    let results = numbers(CHAINS, excel_mode());
    let ieee_results = numbers(CHAINS, ieee_mode());
    for ((chain, result), ieee_result) in CHAINS.iter().zip(results).zip(ieee_results) {
        let (first, rest) = parse_chain(chain);
        let exact = evaluate(first, &rest);
        let result = result.unwrap_or_else(|| panic!("{chain} produced no number"));
        if exact.is_zero() {
            assert_eq!(
                result, 0.0,
                "{chain} is exactly zero but the Excel mode returned {result:e}"
            );
            assert!(
                result.is_sign_positive(),
                "{chain} returned negative zero; calculated values are normalized to +0"
            );
        } else {
            assert_ne!(
                result,
                0.0,
                "{chain} is exactly {} but the Excel mode snapped it to zero",
                exact.to_f64()
            );
            assert_eq!(
                result.to_bits(),
                ieee_result
                    .unwrap_or_else(|| panic!("{chain} produced no IEEE number"))
                    .to_bits(),
                "{chain}: a real nonzero result was changed rather than preserved"
            );
        }
    }
}

/// TIER 1 — the same chains under the opt-in mode keep the residue 0.1.2 produced.
#[test]
fn ieee_mode_keeps_the_residue_for_chains_that_cancel() {
    let results = numbers(CHAINS, ieee_mode());
    let mut residues = 0;
    for (chain, result) in CHAINS.iter().zip(results) {
        let (first, rest) = parse_chain(chain);
        let result = result.unwrap_or_else(|| panic!("{chain} produced no number"));
        if evaluate(first, &rest).is_zero() && result != 0.0 {
            residues += 1;
            assert!(
                result.abs() < 1e-12,
                "{chain} left an implausibly large residue: {result:e}"
            );
        }
    }
    assert!(
        residues > 0,
        "no chain left a residue under IEEE mode, so this suite is not exercising the difference"
    );
}

/// TIER 1 — an inherited residue must be corrected without erasing a nearby real difference.
#[test]
fn inherited_residue_is_corrected_without_erasing_a_real_difference() {
    let inherited = numbers(&["100.1 - 100.0 - 0.1"], excel_mode())[0].expect("number");
    let (first, rest) = parse_chain("100.1 - 100.0 - 0.1");
    assert!(
        evaluate(first, &rest).is_zero(),
        "the chain is exactly zero"
    );
    assert_eq!(inherited, 0.0, "an exact decimal cancellation must snap");

    // The case a wider window would break: a real difference at a comparable relative magnitude.
    let meant = numbers(&["1.0000000000001 - 1.0"], excel_mode())[0].expect("number");
    assert!(
        (meant - 1e-13).abs() < 1e-16,
        "a difference the author meant was snapped away: {meant:e}"
    );
}

/// TIER 1 — the exact decimal trace follows a calculated value across a cell dependency.
#[test]
fn inherited_residue_is_corrected_across_formula_cells() {
    let formulas = ["100.1 - 100.0", "A1 - 0.1"];
    let excel = numbers(&formulas, excel_mode());
    let ieee = numbers(&formulas, ieee_mode());
    assert_eq!(excel[1], Some(0.0));
    assert!(
        ieee[1].is_some_and(|value| value != 0.0),
        "the legacy path must preserve the inherited residue"
    );
}

/// TIER 1 — an untraced function result is never reinterpreted as an exact decimal input.
#[test]
fn calculated_function_results_do_not_create_false_exactness() {
    let direct = "SUM(100.1,-100.0) - 0.099999999999999";
    let direct_excel = numbers(&[direct], excel_mode())[0].expect("Excel-mode number");
    let direct_ieee = numbers(&[direct], ieee_mode())[0].expect("IEEE number");
    assert_ne!(direct_excel, 0.0, "a real difference was snapped to zero");
    assert_eq!(direct_excel.to_bits(), direct_ieee.to_bits());

    let across_cells = ["SUM(100.1,-100.0)", "A1 - 0.099999999999999"];
    let excel = numbers(&across_cells, excel_mode())[1].expect("Excel-mode reference result");
    let ieee = numbers(&across_cells, ieee_mode())[1].expect("IEEE reference result");
    assert_ne!(
        excel, 0.0,
        "a referenced function result gained false exactness"
    );
    assert_eq!(excel.to_bits(), ieee.to_bits());
}

/// TIER 1 — every committed generated arithmetic case is decided by the exact decimal oracle.
#[test]
fn generated_arithmetic_manifest_matches_the_exact_decimal_oracle() {
    let manifest = calculation_modes_manifest();
    assert_eq!(manifest.schema, "cellrune_calculation_modes_v1");
    let formulas: Vec<&str> = manifest
        .arithmetic_cases
        .iter()
        .map(|case| case.formula.as_str())
        .collect();
    let results = numbers(&formulas, excel_mode());
    for (case, result) in manifest.arithmetic_cases.iter().zip(results) {
        assert_eq!(case.oracle, ArithmeticOracle::ExactDecimal);
        let (first, rest) = parse_chain(&case.formula);
        let exact = evaluate(first, &rest);
        let result = result.unwrap_or_else(|| panic!("{} produced no number", case.id));
        assert_eq!(
            result == 0.0,
            exact.is_zero(),
            "{}: result {result:e} disagrees with exact decimal value {}",
            case.id,
            exact.to_f64()
        );
    }
}

/// TIER 2 — metamorphic. The corrected result is either the IEEE result or exactly zero.
///
/// A third value would mean the correction altered a number rather than discarding a residue,
/// which is the failure mode that silently corrupts arithmetic. This needs no expected values, so
/// it covers every chain including the ones no table enumerates.
#[test]
fn the_excel_mode_only_ever_replaces_a_result_with_zero() {
    let excel = numbers(CHAINS, excel_mode());
    let ieee = numbers(CHAINS, ieee_mode());
    for ((chain, excel), ieee) in CHAINS.iter().zip(excel).zip(ieee) {
        let (excel, ieee) = (
            excel.unwrap_or_else(|| panic!("{chain} produced no number")),
            ieee.unwrap_or_else(|| panic!("{chain} produced no number")),
        );
        assert!(
            excel == ieee || excel == 0.0,
            "{chain}: Excel mode returned {excel:e}, which is neither the IEEE result {ieee:e} nor zero"
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

/// TIER 2 — metamorphic, and the reason `excel_sum` had to change at all.
///
/// The operator path and the aggregate path are separate implementations of the same arithmetic.
/// If only one of them is corrected, the release ships a mode in which `=A+B+C` and `=SUM(A,B,C)`
/// disagree, which is not a compatibility setting but a contradiction.
#[test]
fn the_operator_path_and_the_aggregate_path_agree_within_each_mode() {
    const PAIRS: &[(&str, &str)] = &[
        ("0.1 + 0.2 - 0.3", "SUM(0.1,0.2,-0.3)"),
        ("0.7 + 0.1 - 0.8", "SUM(0.7,0.1,-0.8)"),
        ("4.56 - 1.23 - 3.33", "SUM(4.56,-1.23,-3.33)"),
        ("1.1 - 1.0", "SUM(1.1,-1.0)"),
        (
            "100.1 - 100.0 - 0.099999999999999",
            "SUM(100.1,-100.0,-0.099999999999999)",
        ),
        ("1.0000000000000004 - 1.0", "SUM(1.0000000000000004,-1.0)"),
        // `SUMPRODUCT` forms products before summing them, but the sum it forms is the same sum,
        // so it has to reach the same answer. Reading its operands without their decimals leaves
        // `=SUMPRODUCT({0.1,0.2,-0.3})=0` FALSE while `=SUM(0.1,0.2,-0.3)=0` is TRUE.
        ("0.1 + 0.2 - 0.3", "SUMPRODUCT({0.1,0.2,-0.3})"),
        ("0.7 + 0.1 - 0.8", "SUMPRODUCT({0.7,0.1,-0.8})"),
        (
            "100.1 - 100.0 - 0.099999999999999",
            "SUMPRODUCT({100.1,-100.0,-0.099999999999999})",
        ),
    ];
    for options in [excel_mode(), ieee_mode()] {
        for (operators, aggregate) in PAIRS {
            let results = numbers(&[operators, aggregate], options);
            assert_eq!(
                results[0].map(f64::to_bits),
                results[1].map(f64::to_bits),
                "{operators} and {aggregate} disagree",
            );
        }
    }
}

/// TIER 2 — applying neutral arithmetic after a correction cannot change the corrected result.
#[test]
fn near_zero_correction_is_idempotent() {
    const FORMULAS: &[&str] = &[
        "0.1 + 0.2 - 0.3",
        "(0.1 + 0.2 - 0.3) + 0",
        "(0.1 + 0.2 - 0.3) - 0",
        "SUM(0.1,0.2,-0.3,0)",
    ];
    let results = numbers(FORMULAS, excel_mode());
    assert!(
        results.iter().all(|result| *result == Some(0.0)),
        "a corrected zero changed when the same policy was applied again: {results:?}"
    );
}

/// TIER 2 — `NPV` follows the arithmetic axis because it shares the aggregate accumulator.
#[test]
fn npv_follows_the_arithmetic_semantics() {
    for formula in ["NPV(0,0.1,0.2,-0.3)", "NPV(0.1,11,-12.1)"] {
        assert_eq!(numbers(&[formula], excel_mode())[0], Some(0.0), "{formula}");
        let legacy = numbers(&[formula], ieee_mode())[0].expect("IEEE NPV result");
        assert_ne!(legacy, 0.0, "{formula}");
    }

    let nearby = "NPV(0.1,11,-12.099999999999999)";
    let excel = numbers(&[nearby], excel_mode())[0].expect("Excel-mode nearby NPV");
    let ieee = numbers(&[nearby], ieee_mode())[0].expect("IEEE nearby NPV");
    assert_ne!(excel, 0.0, "a real discounted difference was snapped");
    assert_eq!(excel.to_bits(), ieee.to_bits());
}

/// TIER 2 — conditional aggregates share the same streaming accumulator as ordinary aggregates.
#[test]
fn conditional_and_ordinary_aggregates_agree_within_each_mode() {
    for (values, exact_zero) in [
        ([0.1, 0.2, -0.3], true),
        ([100.1, -100.0, -0.099_999_999_999_999], false),
    ] {
        let workbook = conditional_aggregate_workbook(values);
        for options in [excel_mode(), ieee_mode()] {
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
            assert_eq!(
                results[0].to_bits(),
                results[1].to_bits(),
                "SUMIF diverged from SUM"
            );
            assert_eq!(
                results[2].to_bits(),
                results[3].to_bits(),
                "AVERAGEIF diverged from AVERAGE"
            );
            if matches!(
                options.arithmetic_semantics(),
                ArithmeticSemantics::ExcelNearZero
            ) {
                assert_eq!(results[0] == 0.0, exact_zero);
                assert_eq!(results[2] == 0.0, exact_zero);
            }
        }
    }
}

/// TIER 2 — `SUMPRODUCT` multiplies before it adds, and the products are still exact decimals.
///
/// The terms here cancel only after the multiplication: `0.1 x 3` and `-0.3 x 1` are exactly
/// opposite, while their `f64` products are not. Tracing the sum but not the product would leave
/// the kernel unable to see the cancellation it is supposed to correct.
#[test]
fn sumproduct_traces_the_products_it_sums() {
    const PRODUCTS: &[&str] = &[
        "SUMPRODUCT({0.1,-0.3},{3,1})",
        "SUMPRODUCT({0.1,0.2,-0.4},{3,1.5,1.5})",
    ];
    let excel = numbers(PRODUCTS, excel_mode());
    let ieee = numbers(PRODUCTS, ieee_mode());
    let mut residues = 0;
    for ((formula, excel), ieee) in PRODUCTS.iter().zip(excel).zip(ieee) {
        assert_eq!(excel, Some(0.0), "{formula} did not snap an exact zero");
        if ieee.unwrap_or_else(|| panic!("{formula} produced no IEEE number")) != 0.0 {
            residues += 1;
        }
    }
    assert!(
        residues > 0,
        "no product left an IEEE residue, so this test is not exercising the difference"
    );

    // A product sum that is genuinely nonzero must survive untouched in both modes.
    let nearby = "SUMPRODUCT({0.1,-0.30000000000001},{3,1})";
    let excel = numbers(&[nearby], excel_mode())[0].expect("Excel-mode nearby product");
    let ieee = numbers(&[nearby], ieee_mode())[0].expect("IEEE nearby product");
    assert_ne!(excel, 0.0, "a real product difference was snapped");
    assert_eq!(excel.to_bits(), ieee.to_bits());
}

/// TIER 2 — the array path shares `apply_binary` with the scalar path and must share its policy.
#[test]
fn the_array_path_applies_the_same_arithmetic_policy() {
    const ARRAYS: &[&str] = &[
        "SUM({0.1,0.5}+{0.2,0.5}-{0.3,1.0})",
        "SUM({100.1}-{100.0}-{0.1})",
    ];
    let excel = numbers(ARRAYS, excel_mode());
    let ieee = numbers(ARRAYS, ieee_mode());
    for ((formula, excel), ieee) in ARRAYS.iter().zip(excel).zip(ieee) {
        assert_eq!(excel, Some(0.0), "{formula}");
        let ieee = ieee.unwrap_or_else(|| panic!("{formula} produced no number"));
        assert_ne!(ieee, 0.0, "{formula} did not preserve its IEEE residue");
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
