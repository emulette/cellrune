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
    FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, Provenance, ProviderIdentity,
    SavedResult, Sheet, SheetId, SheetName, SheetVisibility, WorkbookSnapshot, WorkbookSource,
    calculate_workbook,
};
use cellrune_integration_tests::decimal_reference::{evaluate, parse_chain};

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
    // Exact value nonzero: the mode must leave every one of these alone.
    "1.1 - 1.0",
    "0.3 - 0.1",
    "1.0 + 2.0",
    "0.1 + 0.2",
    "1000000.0 - 0.5",
    "0.0001 - 0.00005",
];

/// TIER 1 — the exact decimal reference decides, and the mode must agree with it.
///
/// This is the criterion the roadmap fixes: the snap fires when, and only when, the exact value is
/// zero. Firing more widely would corrupt results the author meant; firing more narrowly would
/// leave the mode not doing its job.
#[test]
fn excel_mode_snaps_exactly_the_chains_whose_exact_value_is_zero() {
    let results = numbers(CHAINS, excel_mode());
    for (chain, result) in CHAINS.iter().zip(results) {
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
            // Still the IEEE answer, not the exact decimal one: the mode corrects cancellation,
            // it does not switch the engine to decimal arithmetic.
            let error = (result - exact.to_f64()).abs();
            assert!(
                error <= exact.to_f64().abs() * 1e-12 + f64::EPSILON,
                "{chain} drifted from its exact value: {result:e} vs {:e}",
                exact.to_f64()
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
                result.abs() < 1e-15,
                "{chain} left an implausibly large residue: {result:e}"
            );
        }
    }
    assert!(
        residues > 0,
        "no chain left a residue under IEEE mode, so this suite is not exercising the difference"
    );
}

/// TIER 1 — the documented boundary of the correction, pinned rather than left to be rediscovered.
///
/// `=100.1-100-0.1` is exactly zero, and the Excel mode does **not** snap it. The residue was
/// created by the first subtraction, where it is a fraction of `100.1`; the correction runs on the
/// second subtraction, whose operands are around `0.1`, and there the same residue is far too
/// large to look like cancellation noise.
///
/// This is not a threshold that wants tuning. `=1.0000000000001-1` is a difference the author
/// meant, sits at almost the same relative magnitude, and is well inside the fifteen significant
/// digits Excel keeps — so any window wide enough to catch the first would corrupt the second.
/// Separating them needs an error term carried through every intermediate, which is a different
/// engine. The test asserts both halves so the trade-off cannot be silently "fixed" later.
#[test]
fn residue_inherited_from_a_larger_intermediate_is_left_alone() {
    let inherited = numbers(&["100.1 - 100.0 - 0.1"], excel_mode())[0].expect("number");
    let (first, rest) = parse_chain("100.1 - 100.0 - 0.1");
    assert!(
        evaluate(first, &rest).is_zero(),
        "the chain is exactly zero"
    );
    assert_ne!(
        inherited, 0.0,
        "the correction reached a residue inherited from a larger intermediate; \
         check that the window was not widened at the cost of the case below"
    );

    // The case a wider window would break: a real difference at a comparable relative magnitude.
    let meant = numbers(&["1.0000000000001 - 1.0"], excel_mode())[0].expect("number");
    assert!(
        (meant - 1e-13).abs() < 1e-16,
        "a difference the author meant was snapped away: {meant:e}"
    );
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

/// TIER 2 — the array path shares `apply_binary` with the scalar path and must share its policy.
#[test]
fn the_array_path_applies_the_same_arithmetic_policy() {
    const ARRAY: &[&str] = &["SUM({0.1,0.5}+{0.2,0.5}-{0.3,1.0})"];
    assert_eq!(numbers(ARRAY, excel_mode())[0], Some(0.0));
    let ieee = numbers(ARRAY, ieee_mode())[0].expect("number");
    assert!(
        ieee != 0.0 && ieee.abs() < 1e-15,
        "unexpected residue {ieee:e}"
    );
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
