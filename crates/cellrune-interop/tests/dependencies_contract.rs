use cellrune_interop::{CalculationOptionsDto, CellDependencyRequestDto, WorkbookSession};
use serde_json::json;

fn request(address: &str, offset: u64, limit: u32) -> CellDependencyRequestDto {
    CellDependencyRequestDto {
        sheet: "sheet1".to_owned(),
        address: address.to_owned(),
        offset,
        limit,
    }
}

fn session() -> WorkbookSession {
    let mut session = WorkbookSession::create();
    session.add_sheet("Inputs").expect("sheet");
    for (address, formula) in [
        ("B1", "=A1+SUM(Inputs!A1:A3)"),
        ("C1", "=B1*2"),
        ("D1", "=B1+INDIRECT(\"A\"&1)"),
    ] {
        session
            .set_formula("Sheet1", address, formula, None)
            .expect("formula");
    }
    session
}

#[test]
fn precedents_and_dependents_need_the_current_calculation() {
    let mut session = session();
    let error = session
        .precedents(&request("B1", 0, 0))
        .expect_err("no calculation yet");
    assert_eq!(error.code(), "interop.calculation.required");
    let error = session
        .dependents(&request("B1", 0, 0))
        .expect_err("no calculation yet");
    assert_eq!(error.code(), "interop.calculation.required");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");

    assert_eq!(
        serde_json::to_value(
            session
                .precedents(&request("B1", 0, 0))
                .expect("precedents")
        )
        .expect("serialize"),
        json!({
            "schema_version": 1,
            "cell": {"sheet_id": 1, "sheet_name": "Sheet1", "address": "B1"},
            "dynamic": false,
            "total": 2,
            "offset": 0,
            "next_offset": null,
            "entries": [
                {"sheet_id": 1, "sheet_name": "Sheet1", "range": "A1", "kind": "value"},
                {"sheet_id": 2, "sheet_name": "Inputs", "range": "A1:A3", "kind": "value"}
            ]
        })
    );
    let page = session
        .precedents(&request("B1", 1, 1))
        .expect("second page");
    assert_eq!((page.entries.len(), page.next_offset), (1, None));
    let first = session
        .precedents(&request("B1", 0, 1))
        .expect("first page");
    assert_eq!(first.next_offset, Some(1));
    let indirect = session.precedents(&request("D1", 0, 0)).expect("dynamic");
    assert!(indirect.dynamic);

    assert_eq!(
        serde_json::to_value(
            session
                .dependents(&request("B1", 0, 0))
                .expect("dependents")
        )
        .expect("serialize"),
        json!({
            "schema_version": 1,
            "cell": {"sheet_id": 1, "sheet_name": "Sheet1", "address": "B1"},
            "unlisted_dynamic_formulas": 0,
            "total": 2,
            "offset": 0,
            "next_offset": null,
            "entries": [
                {"cell": {"sheet_id": 1, "sheet_name": "Sheet1", "address": "C1"}, "dynamic": false},
                {"cell": {"sheet_id": 1, "sheet_name": "Sheet1", "address": "D1"}, "dynamic": true}
            ]
        })
    );
    let inputs = session
        .dependents(&CellDependencyRequestDto {
            sheet: "Inputs".to_owned(),
            ..request("A2", 0, 0)
        })
        .expect("dependents");
    assert_eq!(inputs.entries.len(), 1);
    assert_eq!(inputs.entries[0].cell.address, "B1");
    assert_eq!(inputs.unlisted_dynamic_formulas, 1);
    let literal = session.precedents(&request("A1", 0, 0)).expect("literal");
    assert_eq!((literal.total, literal.dynamic), (0, false));

    for (result, code) in [
        (
            session.precedents(&request("B1", 3, 0)).map(|_| ()),
            "interop.page.offset_out_of_range",
        ),
        (
            session.dependents(&request("B1", 0, 10_001)).map(|_| ()),
            "interop.page.limit_exceeded",
        ),
        (
            session
                .precedents(&CellDependencyRequestDto {
                    sheet: "Missing".to_owned(),
                    ..request("A1", 0, 0)
                })
                .map(|_| ()),
            "interop.sheet.not_found",
        ),
    ] {
        assert_eq!(result.expect_err("rejected").code(), code);
    }

    session
        .set_formula("Sheet1", "E1", "=C1", None)
        .expect("edit");
    let error = session
        .dependents(&request("C1", 0, 0))
        .expect_err("the edit left the calculation behind");
    assert_eq!(error.code(), "interop.calculation.required");
}

#[test]
fn a_spill_anchor_is_not_its_own_dependent_unless_it_reads_itself() {
    let mut session = WorkbookSession::create();
    session
        .set_formula("Sheet1", "G1", "=SEQUENCE(3)", Some("G1:G3"))
        .expect("spill");
    session
        .set_formula("Sheet1", "H1", "=H1+1", None)
        .expect("cycle");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    let dependents = |address: &str| {
        session
            .dependents(&request(address, 0, 0))
            .expect("dependents")
            .entries
            .into_iter()
            .map(|entry| entry.cell.address)
            .collect::<Vec<_>>()
    };
    assert!(dependents("G1").is_empty());
    assert_eq!(dependents("G2"), ["G1"]);
    assert_eq!(dependents("H1"), ["H1"]);
}
