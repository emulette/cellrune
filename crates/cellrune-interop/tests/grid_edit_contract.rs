use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellValueDto, EditBatchDto, EditBatchV2Dto,
    RangeRequestDto, RecalculationModeDto, WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookSession,
    WritableCellValueDto,
};

/// `Data` holds values in `A1:A5` and formulas over them; `Sheet1` reads `Data`.
fn session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session.add_sheet("Data").expect("Data");
    for row in 1..=5 {
        session
            .set_value(
                "Data",
                &format!("A{row}"),
                WritableCellValueDto::Number {
                    value: f64::from(row),
                },
            )
            .expect("value");
    }
    for (sheet, address, formula) in [
        ("Data", "B1", "=SUM(A1:A5)"),
        ("Data", "B2", "=A3*10"),
        ("Sheet1", "A1", "=Data!A4"),
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

/// Returns each non-empty cell of `start:end` as `(address, formula or value)`.
fn cells(session: &WorkbookSession, sheet: &str, start: &str, end: &str) -> Vec<(String, String)> {
    session
        .read_range(&RangeRequestDto {
            sheet: sheet.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            offset: 0,
            limit: 0,
        })
        .expect("range")
        .cells
        .into_iter()
        .filter_map(|cell| {
            let text = match (cell.formula, cell.source_value) {
                (Some(formula), _) => formula,
                (None, CellValueDto::Number { value }) => value.to_string(),
                (None, CellValueDto::Blank) => return None,
                (None, other) => format!("{other:?}"),
            };
            Some((cell.address, text))
        })
        .collect()
}

fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(address, text)| ((*address).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn row_insertions_move_cells_and_return_the_changes_that_undo_them() {
    let mut session = session();
    let before = cells(&session, "Data", "A1", "B9");
    let change: WorkbookChangeDto = serde_json::from_value(serde_json::json!({
        "kind": "insert_rows",
        "sheet": "data",
        "row": 2,
        "count": 2,
    }))
    .expect("insert_rows DTO");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: vec![change],
            },
        )
        .expect("insertion");
    assert!(receipt.topology_changed);
    assert_eq!(
        cells(&session, "Data", "B1", "B9"),
        pairs(&[("B1", "=SUM(A1:A7)"), ("B4", "=A5*10")])
    );
    assert_eq!(
        cells(&session, "Sheet1", "A1", "A1"),
        pairs(&[("A1", "=Data!A6")])
    );
    let delta = session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("recalculate");
    assert_eq!(delta.reason, "topology_changed");

    let inverse = receipt
        .inverse
        .expect("structural batches carry an inverse");
    assert!(inverse.unrestored.is_empty());
    assert_eq!(
        serde_json::to_value(&inverse.changes).expect("inverse JSON"),
        serde_json::json!([{"kind": "delete_rows", "sheet": "Data", "first": 2, "last": 3}])
    );
    let revision = session.summary().semantic_revision;
    session
        .apply_changes_v2(
            revision,
            EditBatchV2Dto {
                changes: inverse.changes,
            },
        )
        .expect("undo");
    assert_eq!(cells(&session, "Data", "A1", "B9"), before);
}

#[test]
fn deletions_copies_and_moves_round_trip_through_their_inverse() {
    let batches = [
        serde_json::json!([{"kind": "delete_rows", "sheet": "Data", "first": 3, "last": 4}]),
        serde_json::json!([
            {"kind": "copy_range", "sheet": "Data", "source": "B2", "destination": "C2:C4"},
            {"kind": "copy_range", "sheet": "Data", "source": "B1",
             "destination_sheet": "Sheet1", "destination": "B1"},
        ]),
        serde_json::json!([{"kind": "move_range", "sheet": "Data", "source": "A1:A2",
                            "destination": "D1"}]),
        serde_json::json!([{"kind": "move_columns", "sheet": "Data", "first": 1, "last": 1,
                            "destination": 3}]),
    ];
    for changes in batches {
        let mut session = session();
        let before = (
            cells(&session, "Data", "A1", "E9"),
            cells(&session, "Sheet1", "A1", "B2"),
        );
        let batch: EditBatchV2Dto =
            serde_json::from_value(serde_json::json!({ "changes": changes })).expect("batch");
        let revision = session.summary().semantic_revision;
        let receipt = session.apply_changes_v2(revision, batch).expect("edit");
        let inverse = receipt.receipt.inverse.expect("inverse");
        let revision = session.summary().semantic_revision;
        session
            .apply_changes_v2(
                revision,
                EditBatchV2Dto {
                    changes: inverse.changes,
                },
            )
            .expect("undo");
        assert_eq!(
            (
                cells(&session, "Data", "A1", "E9"),
                cells(&session, "Sheet1", "A1", "B2"),
            ),
            before,
            "{changes}"
        );
    }
}

#[test]
fn deleted_references_read_ref_and_refused_edits_keep_their_core_codes() {
    let mut session = session();
    let revision = session.summary().semantic_revision;
    session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::DeleteRows {
                    sheet: "Data".to_owned(),
                    first: 4,
                    last: 4,
                }],
            },
        )
        .expect("deletion");
    let delta = session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("recalculate");
    let sheet1 = delta
        .changed_cells
        .iter()
        .find(|change| change.cell.sheet_name == "Sheet1")
        .expect("Sheet1 result");
    assert_eq!(
        sheet1.result,
        CalculationResultDto::Value {
            value: CellValueDto::Error {
                value: "#REF!".to_owned()
            }
        }
    );

    let revision = session.summary().semantic_revision;
    session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: vec![WorkbookChangeDto::MoveRange {
                    sheet: "Data".to_owned(),
                    source: "C1".to_owned(),
                    destination: "A2".to_owned(),
                }],
            },
        )
        .expect("pasting over a cell B1 reads");
    let revision = session.summary().semantic_revision;
    let error = session
        .apply_changes_v2(
            revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::V1(WorkbookChangeDto::InsertRows {
                    sheet: "Data".to_owned(),
                    row: 1,
                    count: 0,
                })],
            },
        )
        .expect_err("empty insertion");
    assert_eq!(error.code(), "validation.structural_edit_empty");
}
