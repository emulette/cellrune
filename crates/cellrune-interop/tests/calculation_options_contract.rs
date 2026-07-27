use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationOptionsDto, CalculationResultDto, CellValueDto,
    FinancialSolverSemanticsDto, RangeRequestDto, WorkbookSession,
};

#[test]
fn omitted_transport_fields_keep_the_new_defaults_and_explicit_modes_round_trip() {
    let omitted: CalculationOptionsDto =
        serde_json::from_str(r#"{"today_serial":45000.0}"#).expect("old payload remains valid");
    assert_eq!(
        omitted.arithmetic_semantics,
        ArithmeticSemanticsDto::ExcelNearZero
    );
    assert_eq!(
        omitted.financial_solver_semantics,
        FinancialSolverSemanticsDto::ExcelIterationBudget
    );

    let explicit = CalculationOptionsDto {
        arithmetic_semantics: ArithmeticSemanticsDto::Ieee754,
        financial_solver_semantics: FinancialSolverSemanticsDto::ExtendedSearch,
        ..CalculationOptionsDto::default()
    };
    let serialized = serde_json::to_value(explicit).expect("options serialize");
    assert_eq!(serialized["arithmetic_semantics"], "ieee_754");
    assert_eq!(serialized["financial_solver_semantics"], "extended_search");
    assert_eq!(
        serde_json::from_value::<CalculationOptionsDto>(serialized).expect("options deserialize"),
        explicit
    );
}

#[test]
fn interop_transports_both_calculation_compatibility_axes() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=0.1+0.2-0.3", None)
        .expect("arithmetic formula");
    session
        .set_formula("Sheet1", "A2", "=IRR({-1,100000})", None)
        .expect("financial formula");

    session
        .calculate(CalculationOptionsDto::default())
        .expect("default calculation");
    let defaults = calculated_results(&session);
    assert_eq!(
        defaults[0],
        CalculationResultDto::Value {
            value: CellValueDto::Number { value: 0.0 }
        }
    );
    assert_eq!(
        defaults[1],
        CalculationResultDto::Value {
            value: CellValueDto::Error {
                value: "#NUM!".to_owned()
            }
        }
    );

    session
        .calculate(CalculationOptionsDto {
            arithmetic_semantics: ArithmeticSemanticsDto::Ieee754,
            financial_solver_semantics: FinancialSolverSemanticsDto::ExtendedSearch,
            ..CalculationOptionsDto::default()
        })
        .expect("legacy-compatible calculation");
    let legacy = calculated_results(&session);
    let CalculationResultDto::Value {
        value: CellValueDto::Number { value: residue },
    } = &legacy[0]
    else {
        panic!("legacy arithmetic mode must produce a number");
    };
    assert_ne!(*residue, 0.0);
    let CalculationResultDto::Value {
        value: CellValueDto::Number { value: rate },
    } = &legacy[1]
    else {
        panic!("extended solver mode must converge");
    };
    assert!((*rate - 99_999.0).abs() <= 99_999.0 * 1e-10);
}

fn calculated_results(session: &WorkbookSession) -> Vec<CalculationResultDto> {
    session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "A2".to_owned(),
            offset: 0,
            limit: 2,
        })
        .expect("calculated range")
        .cells
        .into_iter()
        .map(|cell| cell.calculated.expect("calculated value"))
        .collect()
}
