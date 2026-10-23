use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CalculationTargetDto, CellReferenceDto,
    CellValueDto, EditBatchDto, EditBatchV2Dto, InteropErrorKind, RecalculationModeDto,
    TargetCalculationRequestDto, TransactionDetailSectionDto, WorkbookChangeDto,
    WorkbookChangeV2Dto, WorkbookSession, WritableCellValueDto,
};

/// `Sheet1` reads `Data` directly and through a 3-D range; `Data` holds a formula of its own.
fn session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session.add_sheet("Data").expect("Data");
    session.add_sheet("Last").expect("Last");
    for (sheet, address, value) in [
        ("Data", "A1", 2.0),
        ("Last", "A1", 5.0),
        ("Sheet1", "B1", 1.0),
    ] {
        session
            .set_value(sheet, address, WritableCellValueDto::Number { value })
            .expect("value");
    }
    for (sheet, address, formula) in [
        ("Data", "B1", "=A1*10"),
        ("Sheet1", "A1", "=Data!A1+1"),
        ("Sheet1", "A2", "=SUM(Data:Last!A1)"),
        ("Sheet1", "A3", "=B1*3"),
    ] {
        session
            .set_formula(sheet, address, formula, None)
            .expect("formula");
    }
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    session
}

fn cell(sheet_id: u32, sheet_name: &str, address: &str) -> CellReferenceDto {
    CellReferenceDto {
        sheet_id,
        sheet_name: sheet_name.to_owned(),
        address: address.to_owned(),
    }
}

#[test]
fn deleting_a_sheet_rebuilds_and_reports_its_removed_results_by_their_old_name() {
    let mut session = session();
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::DeleteSheet {
                    sheet: "data".to_owned(),
                }],
            },
        )
        .expect("delete Data");
    assert!(receipt.topology_changed);
    assert!(
        receipt
            .changed_cells
            .iter()
            .all(|changed| changed.sheet_name == "Sheet1")
    );
    let names = session
        .summary()
        .sheets
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Sheet1", "Last"]);

    let delta = session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("recalculate");
    assert_eq!(
        (delta.mode.as_str(), delta.reason.as_str()),
        ("full", "topology_changed")
    );
    let changed = |address: &str| {
        delta
            .changed_cells
            .iter()
            .find(|change| change.cell.address == address)
            .map(|change| change.result.clone())
    };
    assert_eq!(
        changed("A1"),
        Some(CalculationResultDto::Value {
            value: CellValueDto::Error {
                value: "#REF!".to_owned()
            }
        })
    );
    assert_eq!(
        changed("A2"),
        Some(CalculationResultDto::Value {
            value: CellValueDto::Number { value: 5.0 }
        })
    );
    assert_eq!(delta.removed_materialized_cells, [cell(2, "Data", "B1")]);
    // The same delta read back later still names the deleted sheet.
    let page = session.changes_since(delta.cursor - 1, 10).expect("page");
    assert_eq!(
        page.deltas[0].removed_materialized_cells,
        [cell(2, "Data", "B1")]
    );

    // A deleted sheet's ID stays taken, and its name is free again.
    assert_eq!(session.add_sheet("Data").expect("re-added"), 4);
}

#[test]
fn a_batch_cannot_reach_a_sheet_it_deleted_and_the_last_visible_sheet_stays() {
    let mut session = session();
    let revision = session.summary().semantic_revision;
    let error = session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: vec![
                    WorkbookChangeDto::DeleteSheet {
                        sheet: "Data".to_owned(),
                    },
                    WorkbookChangeDto::SetValue {
                        sheet: "Data".to_owned(),
                        address: "A1".to_owned(),
                        value: WritableCellValueDto::Blank,
                    },
                ],
            },
        )
        .expect_err("Data is gone for the rest of the batch");
    assert_eq!(error.kind(), InteropErrorKind::Input);
    assert_eq!(session.summary().semantic_revision, revision);

    session.delete_sheet("Data").expect("delete Data");
    session.delete_sheet("Last").expect("delete Last");
    let error = session
        .delete_sheet("Sheet1")
        .expect_err("the last visible sheet stays");
    assert_eq!(error.code(), "validation.last_visible_sheet");
}

#[test]
fn targeted_calculation_and_previews_follow_a_deleted_sheet() {
    let mut session = session();
    session.delete_sheet("Data").expect("delete Data");
    let request = |sheet: &str| TargetCalculationRequestDto {
        targets: vec![CalculationTargetDto {
            sheet: sheet.to_owned(),
            start: "A1".to_owned(),
            end: Some("A2".to_owned()),
        }],
        options: CalculationOptionsDto::default(),
        limits: Default::default(),
    };
    let targeted = session
        .calculate_targets(&request("Sheet1"))
        .expect("targets on a remaining sheet");
    let results = targeted
        .cells
        .iter()
        .map(|result| (result.cell.address.as_str(), result.result.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        results,
        [
            (
                "A1",
                CalculationResultDto::Value {
                    value: CellValueDto::Error {
                        value: "#REF!".to_owned()
                    }
                }
            ),
            (
                "A2",
                CalculationResultDto::Value {
                    value: CellValueDto::Number { value: 5.0 }
                }
            ),
        ]
    );
    let error = session
        .calculate_targets(&request("Data"))
        .expect_err("a deleted sheet has no cells");
    assert_eq!(error.kind(), InteropErrorKind::Input);

    let mut session = self::session();
    let revision = session.summary().semantic_revision;
    let preview = session
        .preview_changes(
            revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::V1(WorkbookChangeDto::DeleteSheet {
                    sheet: "Data".to_owned(),
                })],
            },
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .expect("preview a deletion");
    let page = session
        .preview_changes_page(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            100,
        )
        .expect("preview results");
    let serialized = serde_json::to_string(&page).expect("page JSON");
    assert!(
        serialized.contains(r#""sheet_name":"Data","address":"B1""#),
        "{serialized}"
    );
    let expected = session
        .preview_commit_receipt(preview.preview_id)
        .expect("receipt before commit");
    assert_eq!(
        expected.calculation_delta.removed_materialized_cells,
        [cell(2, "Data", "B1")]
    );
    let receipt = session
        .commit_preview(preview.preview_id)
        .expect("commit the previewed deletion");
    assert!(receipt.edit.topology_changed);
    assert_eq!(
        receipt.calculation_delta.removed_materialized_cells,
        [cell(2, "Data", "B1")]
    );
    assert!(
        session
            .summary()
            .sheets
            .iter()
            .all(|sheet| sheet.name != "Data")
    );
}
