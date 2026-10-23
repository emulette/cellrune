use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationLimitOptionsDto, CalculationOptionsDto,
    CalculationResultDto, CellValueDto, FinancialSolverSemanticsDto, LocaleDto, LocaleProfileDto,
    RangeRequestDto, ReadLimitOptionsDto, WorkbookSession,
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

#[test]
fn calculation_and_read_limits_cross_the_transport_boundary() {
    let omitted: CalculationOptionsDto =
        serde_json::from_str("{}").expect("a payload without limits keeps every default");
    assert_eq!(
        omitted.calculation_limits,
        CalculationLimitOptionsDto::default()
    );

    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=SUM(B1:B200)", None)
        .expect("range formula");
    let limited = CalculationOptionsDto {
        calculation_limits: CalculationLimitOptionsDto {
            max_array_cells: Some(100),
            ..CalculationLimitOptionsDto::default()
        },
        ..CalculationOptionsDto::default()
    };
    let report = session.calculate(limited).expect("limited calculation");
    assert_eq!(
        report.unavailable_count, 1,
        "a lowered limit stops the formula"
    );
    let report = session
        .calculate(CalculationOptionsDto::default())
        .expect("default calculation");
    assert_eq!(report.unavailable_count, 0);

    let zero = CalculationOptionsDto {
        calculation_limits: CalculationLimitOptionsDto {
            max_text_bytes: Some(0),
            ..CalculationLimitOptionsDto::default()
        },
        ..CalculationOptionsDto::default()
    };
    let error = session
        .calculate(zero)
        .expect_err("a zero limit is rejected");
    assert_eq!(error.code(), "interop.limit.invalid");
    assert!(
        error
            .details()
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("max_text_bytes"))
    );

    let mut source = WorkbookSession::create();
    source
        .set_value(
            "Sheet1",
            "A1",
            cellrune_interop::WritableCellValueDto::Number { value: 1.0 },
        )
        .expect("value");
    source
        .calculate(CalculationOptionsDto::default())
        .expect("source calculation");
    let (bytes, _) = source
        .save_bytes(cellrune_interop::WriteOptionsDto::default())
        .expect("source package");
    let tight = ReadLimitOptionsDto {
        max_entry_uncompressed_bytes: Some(64),
        ..ReadLimitOptionsDto::default()
    };
    let error = WorkbookSession::open_bytes_with_limits(&bytes, tight)
        .expect_err("a lowered read limit rejects the package");
    assert_eq!(error.code(), "xlsx.entry_too_large");
    WorkbookSession::open_bytes_with_limits(&bytes, ReadLimitOptionsDto::default())
        .expect("default read limits open the package");
    let error = WorkbookSession::open_bytes_with_limits(
        &bytes,
        ReadLimitOptionsDto {
            max_sheets: Some(0),
            ..ReadLimitOptionsDto::default()
        },
    )
    .expect_err("a zero read limit is rejected");
    assert_eq!(error.code(), "interop.limit.invalid");
}

#[test]
fn random_seed_crosses_the_transport_boundary() {
    let seeded: CalculationOptionsDto =
        serde_json::from_str(r#"{"random_seed":4294967295}"#).expect("largest u32 seed");
    assert_eq!(seeded.random_seed, Some(u32::MAX));
    assert_eq!(
        serde_json::from_str::<CalculationOptionsDto>("{}")
            .expect("omitted seed")
            .random_seed,
        None
    );
    for invalid in ["-1", "1.5", "4294967296", "true", "\"7\""] {
        assert!(
            serde_json::from_str::<CalculationOptionsDto>(&format!(
                r#"{{"random_seed":{invalid}}}"#
            ))
            .is_err(),
            "{invalid}"
        );
    }

    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=RAND()", None)
        .expect("RAND formula");
    session
        .set_formula("Sheet1", "A2", "=RANDBETWEEN(1,1000000)", None)
        .expect("RANDBETWEEN formula");
    let report = session
        .calculate(CalculationOptionsDto::default())
        .expect("calculation without a seed");
    assert_eq!(report.unavailable_count, 2);
    assert!(calculated_results(&session).iter().all(|result| matches!(
        result,
        CalculationResultDto::Unavailable { code, .. }
            if code == "calculation.volatile_input_missing"
    )));

    let seed = |random_seed| CalculationOptionsDto {
        random_seed: Some(random_seed),
        ..CalculationOptionsDto::default()
    };
    session.calculate(seed(7)).expect("seeded calculation");
    let first = calculated_results(&session);
    assert!(
        first
            .iter()
            .all(|result| matches!(result, CalculationResultDto::Value { .. }))
    );
    session.calculate(seed(7)).expect("repeated seed");
    assert_eq!(calculated_results(&session), first);
    session.calculate(seed(8)).expect("new seed");
    assert_ne!(calculated_results(&session), first);

    let targeted = session
        .calculate_targets(
            &serde_json::from_value(serde_json::json!({
                "targets": [{"sheet": "Sheet1", "start": "A1", "end": "A2"}],
                "options": {"random_seed": 7},
            }))
            .expect("targeted request"),
        )
        .expect("targeted calculation");
    assert_eq!(
        targeted
            .cells
            .into_iter()
            .map(|cell| cell.result)
            .collect::<Vec<_>>(),
        first
    );

    let preview = session
        .preview_changes(
            session.summary().semantic_revision,
            cellrune_interop::EditBatchV2Dto {
                changes: vec![cellrune_interop::WorkbookChangeV2Dto::V1(
                    cellrune_interop::WorkbookChangeDto::SetValue {
                        sheet: "Sheet1".to_owned(),
                        address: "B1".to_owned(),
                        value: cellrune_interop::WritableCellValueDto::Number { value: 1.0 },
                    },
                )],
            },
            cellrune_interop::RecalculationModeDto::Auto,
            seed(7),
        )
        .expect("seeded preview");
    assert_eq!(preview.report.calculation_options.random_seed, Some(7));
}

#[test]
fn locale_profile_crosses_the_transport_boundary() {
    let korean: CalculationOptionsDto =
        serde_json::from_str(r#"{"locale_profile":{"locale":"ko-KR"}}"#).expect("Korean locale");
    assert_eq!(
        korean.locale_profile,
        Some(LocaleProfileDto {
            locale: LocaleDto::KoKr,
            display_language: None,
        })
    );
    assert_eq!(
        serde_json::from_str::<CalculationOptionsDto>("{}")
            .expect("omitted profile")
            .locale_profile,
        None
    );
    for invalid in [
        r#"{"locale":"ja-JP"}"#,
        r#"{"locale":"ko-kr"}"#,
        r#"{"display_language":"ko-KR"}"#,
        r#"{"locale":"ko-KR","region":"KR"}"#,
        r#""ko-KR""#,
    ] {
        assert!(
            serde_json::from_str::<CalculationOptionsDto>(&format!(
                r#"{{"locale_profile":{invalid}}}"#
            ))
            .is_err(),
            "{invalid}"
        );
    }
    assert_eq!(
        LocaleDto::from_tag("ko-KR").expect("Korean tag"),
        LocaleDto::KoKr
    );
    assert_eq!(
        LocaleDto::from_tag("ko").expect_err("partial tag").code(),
        "interop.calculation.locale_profile_invalid"
    );

    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "A1", "=DOLLAR(1234.567,2)", None)
        .expect("DOLLAR formula");
    session
        .set_formula("Sheet1", "A2", "=LENB(\"가나\")", None)
        .expect("LENB formula");
    let results = |session: &WorkbookSession| {
        calculated_results(session)
            .into_iter()
            .map(|result| match result {
                CalculationResultDto::Value {
                    value: CellValueDto::Text { value },
                } => value,
                CalculationResultDto::Value {
                    value: CellValueDto::Number { value },
                } => value.to_string(),
                other => panic!("unexpected result {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    session
        .calculate(CalculationOptionsDto::default())
        .expect("default calculation");
    assert_eq!(results(&session), ["$1,234.57", "2"]);
    let profile = |display_language| CalculationOptionsDto {
        locale_profile: Some(LocaleProfileDto {
            locale: LocaleDto::KoKr,
            display_language,
        }),
        ..CalculationOptionsDto::default()
    };
    session
        .calculate(profile(None))
        .expect("Korean calculation");
    assert_eq!(results(&session), ["₩1,234.57", "4"]);

    let targeted = session
        .calculate_targets(
            &serde_json::from_value(serde_json::json!({
                "targets": [{"sheet": "Sheet1", "start": "A1", "end": "A1"}],
                "options": {"locale_profile": {"locale": "ko-KR"}},
            }))
            .expect("targeted request"),
        )
        .expect("targeted calculation");
    assert_eq!(
        targeted.cells[0].result,
        CalculationResultDto::Value {
            value: CellValueDto::Text {
                value: "₩1,234.57".to_owned()
            }
        }
    );

    let preview = session
        .preview_changes(
            session.summary().semantic_revision,
            cellrune_interop::EditBatchV2Dto {
                changes: vec![cellrune_interop::WorkbookChangeV2Dto::V1(
                    cellrune_interop::WorkbookChangeDto::SetValue {
                        sheet: "Sheet1".to_owned(),
                        address: "B1".to_owned(),
                        value: cellrune_interop::WritableCellValueDto::Number { value: 1.0 },
                    },
                )],
            },
            cellrune_interop::RecalculationModeDto::Auto,
            profile(Some(LocaleDto::EnUs)),
        )
        .expect("Korean preview with English labels");
    assert_eq!(
        preview.report.calculation_options.locale_profile,
        LocaleProfileDto {
            locale: LocaleDto::KoKr,
            display_language: Some(LocaleDto::EnUs),
        }
    );
}
