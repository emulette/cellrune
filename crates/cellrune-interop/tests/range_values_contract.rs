use cellrune_interop::{
    CalculationOptionsDto, InteropErrorKind, RangeValuesRequestDto, RangeWriteRequestDto,
    WorkbookSession,
};
use serde_json::{Value, json};

fn request(start: &str, end: &str) -> RangeValuesRequestDto {
    RangeValuesRequestDto {
        sheet: "sheet1".to_owned(),
        start: start.to_owned(),
        end: end.to_owned(),
    }
}

fn write(rows: Value) -> RangeWriteRequestDto {
    serde_json::from_value(json!({"sheet": "Sheet1", "start": "B2", "rows": rows}))
        .expect("write request")
}

fn rows(session: &WorkbookSession, start: &str, end: &str) -> Value {
    serde_json::to_value(session.read_values(&request(start, end)).expect("read"))
        .expect("serialize")["rows"]
        .clone()
}

#[test]
fn a_written_rectangle_reads_back_as_rows_in_one_batch() {
    let mut session = WorkbookSession::create();
    let receipt = session
        .write_values(write(json!([
            [{"kind": "number", "value": 1.5}, {"kind": "text", "value": "a"}],
            [{"kind": "logical", "value": true}, {"kind": "error", "value": "#N/A"}]
        ])))
        .expect("write");
    assert_eq!((receipt.base_revision, receipt.result_revision), (0, 1));
    assert_eq!(receipt.applied_change_count, 4);

    let values = session.read_values(&request("A1", "C3")).expect("read");
    assert_eq!(
        serde_json::to_value(&values).expect("serialize"),
        json!({
            "schema_version": 1,
            "sheet": "Sheet1",
            "start": "A1",
            "end": "C3",
            "rows": [
                [{"kind": "blank"}, {"kind": "blank"}, {"kind": "blank"}],
                [{"kind": "blank"}, {"kind": "number", "value": 1.5}, {"kind": "text", "value": "a"}],
                [{"kind": "blank"}, {"kind": "logical", "value": true}, {"kind": "error", "value": "#N/A"}]
            ]
        })
    );

    // A blank value clears its cell, so a rectangle of blanks erases what it covers.
    session
        .write_values(write(json!([[{"kind": "blank"}, {"kind": "blank"}]])))
        .expect("clear");
    assert_eq!(
        rows(&session, "B2", "C2"),
        json!([[{"kind": "blank"}, {"kind": "blank"}]])
    );
    assert_eq!(session.summary().sheets[0].cell_count, 2);
}

#[test]
fn formulas_read_their_current_result_or_else_their_saved_value() {
    let mut session = WorkbookSession::create();
    session
        .write_values(write(json!([[{"kind": "number", "value": 2.0}]])))
        .expect("write");
    session
        .set_formula("Sheet1", "C2", "=B2*2", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "D2", "=SEQUENCE(2)", Some("D2:D3"))
        .expect("spill");
    session
        .set_formula("Sheet1", "E2", "=NOT_A_FUNCTION(1)", None)
        .expect("unsupported");
    // Without a current calculation a formula holds its saved value, which these never had.
    assert_eq!(
        rows(&session, "C2", "E2"),
        json!([[{"kind": "blank"}, {"kind": "blank"}, {"kind": "blank"}]])
    );

    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    let calculated = rows(&session, "B2", "E3");
    assert_eq!(calculated[0][0], json!({"kind": "number", "value": 2.0}));
    assert_eq!(calculated[0][1], json!({"kind": "number", "value": 4.0}));
    assert_eq!(calculated[0][2], json!({"kind": "number", "value": 1.0}));
    assert_eq!(calculated[1][2], json!({"kind": "number", "value": 2.0}));
    assert_eq!(calculated[0][3]["kind"], "unavailable");
    assert_eq!(calculated[0][3]["code"], "calculation.unsupported_function");
    assert_eq!(calculated[1][3], json!({"kind": "blank"}));

    // An edit leaves the calculation behind, so formulas fall back to their saved values again.
    session
        .write_values(write(json!([[{"kind": "number", "value": 5.0}]])))
        .expect("edit");
    assert_eq!(rows(&session, "C2", "C2"), json!([[{"kind": "blank"}]]));
}

#[test]
fn invalid_rectangles_fail_without_changing_the_workbook() {
    let mut session = WorkbookSession::create();
    for rows in [
        json!([]),
        json!([[]]),
        json!([[{"kind": "blank"}], [{"kind": "blank"}, {"kind": "blank"}]]),
    ] {
        let error = session
            .write_values(write(rows))
            .expect_err("not a rectangle");
        assert_eq!(error.code(), "interop.change.invalid");
    }
    let error = session
        .write_values(
            serde_json::from_value(json!({
                "sheet": "Sheet1",
                "start": "XFD1",
                "rows": [[{"kind": "number", "value": 1.0}, {"kind": "number", "value": 2.0}]]
            }))
            .expect("request"),
        )
        .expect_err("the second column is past the grid");
    assert_eq!(error.kind(), InteropErrorKind::Validation);
    let error = session
        .write_values(
            serde_json::from_value(json!({
                "sheet": "Missing", "start": "A1", "rows": [[{"kind": "blank"}]]
            }))
            .expect("request"),
        )
        .expect_err("unknown sheet");
    assert_eq!(error.code(), "interop.sheet.not_found");
    assert_eq!(session.summary().semantic_revision, 0);

    let error = session
        .read_values(&request("A1", "B1048576"))
        .expect_err("two full columns exceed the read limit");
    assert_eq!(error.code(), "interop.page.limit_exceeded");
    session
        .read_values(&request("A1", "A1048576"))
        .expect("one full column is within the read limit");
    let error = session
        .read_values(&RangeValuesRequestDto {
            sheet: "Missing".to_owned(),
            ..request("A1", "A1")
        })
        .expect_err("unknown sheet");
    assert_eq!(error.code(), "interop.sheet.not_found");
}
