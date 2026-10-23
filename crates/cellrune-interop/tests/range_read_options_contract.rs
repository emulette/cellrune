use cellrune_interop::{
    CalculationOptionsDto, RangeReadOptionsDto, RangeRequestDto, WorkbookSession,
};
use serde_json::{Value, json};

fn request(start: &str, end: &str, offset: u64, limit: u32) -> RangeRequestDto {
    RangeRequestDto {
        sheet: "Sheet1".to_owned(),
        start: start.to_owned(),
        end: end.to_owned(),
        offset,
        limit,
    }
}

fn read(session: &WorkbookSession, request: &RangeRequestDto, options: Value) -> Value {
    let options: RangeReadOptionsDto = serde_json::from_value(options).expect("options");
    serde_json::to_value(
        session
            .read_range_with_options(request, &options)
            .expect("range page"),
    )
    .expect("serialize")
}

fn calculated_session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session
        .set_value(
            "Sheet1",
            "A1",
            cellrune_interop::WritableCellValueDto::Number { value: 1.0 },
        )
        .expect("value");
    session
        .set_formula("Sheet1", "C1", "=A1+1", None)
        .expect("formula");
    session
        .set_formula("Sheet1", "A3", "=SEQUENCE(1,2)", Some("A3:B3"))
        .expect("spill");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    session
}

#[test]
fn default_options_return_the_unchanged_range_page() {
    let session = calculated_session();
    let request = request("A1", "C3", 1, 4);
    let plain = serde_json::to_vec(&session.read_range(&request).expect("plain")).expect("json");
    let defaulted = serde_json::to_vec(
        &session
            .read_range_with_options(&request, &RangeReadOptionsDto::default())
            .expect("defaulted"),
    )
    .expect("json");
    assert_eq!(plain, defaulted);
    let page: Value = serde_json::from_slice(&plain).expect("page");
    assert!(page.get("values").is_none() && page.get("rows").is_none());
    assert_eq!(page["cells"].as_array().map(Vec::len), Some(4));
}

#[test]
fn empty_cells_are_left_out_while_positions_still_advance() {
    let session = calculated_session();
    let page = read(
        &session,
        &request("A1", "C3", 0, 6),
        json!({"omit_empty": true}),
    );
    let addresses = page["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .map(|cell| cell["address"].clone())
        .collect::<Vec<_>>();
    assert_eq!(addresses, [json!("A1"), json!("C1")]);
    assert_eq!(page["next_offset"], 6);
    assert_eq!(page["cells"][1]["formula"], "=A1+1");

    // A spill follower holds no stored cell but does hold a calculated result.
    let rest = read(
        &session,
        &request("A1", "C3", 6, 0),
        json!({"omit_empty": true}),
    );
    assert_eq!(rest["cells"].as_array().map(Vec::len), Some(2));
    assert_eq!(rest["cells"][1]["address"], "B3");
    assert_eq!(rest["next_offset"], Value::Null);
}

#[test]
fn values_only_and_compact_pages_carry_values_without_cell_objects() {
    let session = calculated_session();
    let values = read(
        &session,
        &request("A1", "C1", 0, 0),
        json!({"values_only": true, "omit_empty": true}),
    );
    assert_eq!(values["cells"], json!([]));
    assert_eq!(
        values["values"],
        json!([
            {"address": "A1", "value": {"kind": "number", "value": 1.0}},
            {"address": "C1", "value": {"kind": "number", "value": 2.0}}
        ])
    );
    assert!(values.get("rows").is_none());

    let compact = read(
        &session,
        &request("A1", "C3", 1, 0),
        json!({"compact": true}),
    );
    assert_eq!(compact["cells"], json!([]));
    assert!(compact.get("values").is_none());
    assert_eq!(
        compact["rows"],
        json!([
            [{"kind": "blank"}, {"kind": "number", "value": 2.0}],
            [{"kind": "blank"}, {"kind": "blank"}, {"kind": "blank"}],
            [{"kind": "number", "value": 1.0}, {"kind": "number", "value": 2.0}, {"kind": "blank"}]
        ])
    );
    let trimmed = read(
        &session,
        &request("A1", "C3", 0, 0),
        json!({"compact": true, "omit_empty": true}),
    );
    assert_eq!(
        trimmed["rows"],
        json!([
            [{"kind": "number", "value": 1.0}, {"kind": "blank"}, {"kind": "number", "value": 2.0}],
            [],
            [{"kind": "number", "value": 1.0}, {"kind": "number", "value": 2.0}]
        ])
    );
}

#[test]
fn a_byte_limit_returns_the_longest_fitting_prefix_and_continues_after_it() {
    let session = calculated_session();
    let whole = read(
        &session,
        &request("A1", "C3", 0, 0),
        json!({"values_only": true}),
    );
    let whole_bytes = serde_json::to_vec(&whole).expect("json").len() as u64;
    let unlimited = read(
        &session,
        &request("A1", "C3", 0, 0),
        json!({"values_only": true, "max_bytes": whole_bytes}),
    );
    assert_eq!(unlimited, whole);

    let max_bytes = whole_bytes / 2;
    let mut offset = 0;
    let mut collected = Vec::new();
    loop {
        let page = read(
            &session,
            &request("A1", "C3", offset, 0),
            json!({"values_only": true, "max_bytes": max_bytes}),
        );
        assert!(serde_json::to_vec(&page).expect("json").len() as u64 <= max_bytes);
        collected.extend(page["values"].as_array().expect("values").iter().cloned());
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(Value::Array(collected), whole["values"]);

    let full_cells = read(
        &session,
        &request("A1", "C3", 0, 0),
        json!({"max_bytes": 600}),
    );
    let cells = full_cells["cells"].as_array().expect("cells");
    assert!(!cells.is_empty() && cells.len() < 9);
    assert_eq!(full_cells["next_offset"], cells.len());

    for max_bytes in [10, 200] {
        let options: RangeReadOptionsDto =
            serde_json::from_value(json!({"max_bytes": max_bytes})).expect("options");
        let error = session
            .read_range_with_options(&request("A1", "C3", 0, 0), &options)
            .expect_err("no position fits");
        assert_eq!(error.code(), "interop.page.byte_limit_exceeded");
    }
}
