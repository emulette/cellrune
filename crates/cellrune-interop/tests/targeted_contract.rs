use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CancellationToken, CellValueDto, EditBatchV2Dto,
    RecalculationModeDto, TargetCalculationRequestDto, TargetCalculationResultDto,
    WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookSession, WritableCellValueDto, WriteOptionsDto,
};

fn request() -> TargetCalculationRequestDto {
    serde_json::from_value(
        serde_json::json!({"targets": [{"sheet":"sheet1", "start":"B1", "end":"C1"}]}),
    )
    .expect("typed request")
}

#[test]
fn package_backed_structured_references_match_full_without_saved_cache_trust() {
    use cellrune::{
        CalculationCellId, CalculationOptions, CalculationTarget, CancellationToken, CellContent,
        ReadOptions, TargetCalculationLimits, calculate_targets, calculate_workbook,
        read_xlsx_path,
    };
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../binding-contract/table-authoring-v2.xlsx");
    let workbook = read_xlsx_path(path, ReadOptions::default()).expect("table fixture");
    let full = calculate_workbook(&workbook, CalculationOptions::default());
    let mut checked = 0;
    for sheet in workbook.sheets() {
        for cell in sheet
            .cells()
            .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
        {
            let id = CalculationCellId::new(sheet.id(), cell.address());
            let result = calculate_targets(
                &workbook,
                &[CalculationTarget::cell(id)],
                CalculationOptions::default(),
                TargetCalculationLimits::default(),
                CancellationToken::new(),
            )
            .expect("single formula scope");
            assert_eq!(
                result.cell(id),
                full.cell(id),
                "{}!{}",
                sheet.name().as_str(),
                cell.address()
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
}
fn session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 1.0 })
        .expect("literal");
    session
        .set_formula("Sheet1", "B1", "=A1+1", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "C1", "=B1+1", None)
        .expect("formula");
    session
}

#[test]
fn first_partial_response_is_typed_without_enabling_save_or_delta_history() {
    let mut session = session();
    let revision = session.summary().semantic_revision;
    let result = session
        .calculate_targets(&request())
        .expect("partial response");
    assert_eq!(result.semantic_revision, revision);
    assert_eq!(result.evaluated_count, 2);
    assert_eq!(
        result.cells[1].result,
        CalculationResultDto::Value {
            value: CellValueDto::Number { value: 3.0 }
        }
    );
    assert_eq!(result.cells[0].cell.sheet_name, "Sheet1");
    assert_eq!(session.summary().semantic_revision, revision);
    assert!(
        session
            .changes_since(0, 10)
            .expect("history")
            .deltas
            .is_empty()
    );
    assert_eq!(
        session
            .save_bytes(WriteOptionsDto::default())
            .expect_err("partial cannot save")
            .code(),
        "interop.calculation.required"
    );
    let encoded = serde_json::to_string(&result).expect("serialize");
    assert_eq!(
        serde_json::from_str::<TargetCalculationResultDto>(&encoded).expect("roundtrip"),
        result
    );
    assert!(!session.calculation_active());
}

#[test]
fn partial_cache_reuse_preserves_a_published_preview_and_installed_delta() {
    let mut session = session();
    session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("full");
    let history = session.changes_since(0, 10).expect("history");
    let preview = session
        .preview_changes(
            session.summary().semantic_revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetValue {
                    sheet: "Sheet1".to_owned(),
                    address: "A1".to_owned(),
                    value: WritableCellValueDto::Number { value: 5.0 },
                })],
            },
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("preview");
    let result = session
        .calculate_targets(&request())
        .expect("cached partial");
    assert_eq!(result.reused_count, 2);
    assert_eq!(result.evaluated_count, 0);
    assert_eq!(
        session.changes_since(0, 10).expect("unchanged history"),
        history
    );
    session
        .commit_preview(preview.preview_id)
        .expect("preview remains committable");
}

#[test]
fn stale_cancelled_superseded_and_invalid_requests_preserve_session_state() {
    let mut session = session();
    let prepared = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("prepare");
    let mut invalid = request();
    invalid.targets.clear();
    assert!(
        session
            .prepare_target_calculation(&invalid, CancellationToken::new())
            .is_err()
    );
    session
        .finish_target_calculation(prepared.run().expect("invalid request did not supersede"))
        .expect("finish");
    let first = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("first");
    let second = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("second");
    assert_eq!(
        first.run().expect_err("superseded").code(),
        "calculation.target.cancelled"
    );
    let completed = second.run().expect("second completes");
    session
        .set_value("Sheet1", "A1", WritableCellValueDto::Number { value: 8.0 })
        .expect("edit");
    assert_eq!(
        session
            .finish_target_calculation(completed)
            .expect_err("stale")
            .code(),
        "calculation.target.stale_result"
    );
    assert!(!session.calculation_active());
    let cancelled = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("prepare cancellation");
    let id = cancelled.request_id();
    assert!(session.cancel_calculation());
    assert_eq!(
        cancelled.run().expect_err("cancelled").code(),
        "calculation.target.cancelled"
    );
    session.abandon_recalculation(id);
    assert!(!session.calculation_active());
}

#[test]
fn cancellation_after_run_clears_only_its_own_active_request() {
    let mut session = session();
    let completed = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("prepare")
        .run()
        .expect("run");
    assert!(session.cancel_calculation());
    assert_eq!(
        session
            .finish_target_calculation(completed)
            .expect_err("cancelled after run")
            .code(),
        "session.cancelled"
    );
    assert!(!session.calculation_active());
    let old = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("prepare")
        .run()
        .expect("run");
    let current = session
        .prepare_target_calculation(&request(), CancellationToken::new())
        .expect("new request");
    assert!(session.finish_target_calculation(old).is_err());
    assert!(session.calculation_active());
    session
        .finish_target_calculation(current.run().expect("current run"))
        .expect("current finish");
}
