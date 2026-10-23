use cellrune_interop::{
    CalculationOptionsDto, CsvReadOptionsDto, CsvWriteRequestDto, RangeRequestDto, WorkbookSession,
    WritableCellValueDto,
};
use serde_json::{Value, json};

fn values(session: &WorkbookSession, sheet: &str, end: &str) -> Vec<Value> {
    session
        .read_range(&RangeRequestDto {
            sheet: sheet.to_owned(),
            start: "A1".to_owned(),
            end: end.to_owned(),
            offset: 0,
            limit: 0,
        })
        .expect("range")
        .cells
        .into_iter()
        .map(|cell| serde_json::to_value(cell.source_value).expect("value"))
        .collect()
}

fn write_request(sheet: &str, delimiter: Option<&str>) -> CsvWriteRequestDto {
    CsvWriteRequestDto {
        sheet: sheet.to_owned(),
        delimiter: delimiter.map(str::to_owned),
    }
}

#[test]
fn csv_opens_as_a_workbook_and_exports_its_calculated_values() {
    let mut session = WorkbookSession::open_csv_bytes(
        b"name;amount\r\n\"Widget; large\";2.5\r\n",
        &CsvReadOptionsDto {
            sheet_name: Some("Import".to_owned()),
            delimiter: Some(";".to_owned()),
        },
    )
    .expect("open csv");
    let summary = session.summary();
    assert_eq!(summary.sheets[0].name, "Import");
    assert!(!summary.document_backed);
    assert_eq!(
        values(&session, "Import", "B2"),
        [
            json!({"kind": "text", "value": "name"}),
            json!({"kind": "text", "value": "amount"}),
            json!({"kind": "text", "value": "Widget; large"}),
            json!({"kind": "number", "value": 2.5}),
        ]
    );

    assert_eq!(
        String::from_utf8(
            session
                .save_csv_bytes(&write_request("Import", None))
                .expect("a sheet without formulas needs no calculation")
        )
        .expect("utf8"),
        "name,amount\r\nWidget; large,2.5\r\n"
    );
    session
        .set_formula("Import", "C2", "=B2*2", None)
        .expect("formula");
    assert_eq!(
        session
            .save_csv_bytes(&write_request("Import", None))
            .expect_err("calculation required")
            .code(),
        "interop.calculation.required"
    );
    session
        .set_value(
            "Import",
            "D3",
            WritableCellValueDto::Logical { value: true },
        )
        .expect("value");
    session
        .calculate(CalculationOptionsDto::default())
        .expect("calculate");
    let csv = session
        .save_csv_bytes(&write_request("import", None))
        .expect("export");
    assert_eq!(
        String::from_utf8(csv).expect("utf8"),
        "name,amount,,\r\nWidget; large,2.5,5,\r\n,,,TRUE\r\n"
    );
    let tabs = session
        .save_csv_bytes(&write_request("Import", Some("\t")))
        .expect("tab export");
    assert!(
        String::from_utf8(tabs)
            .expect("utf8")
            .starts_with("name\tamount")
    );

    for (request, code) in [
        (write_request("Missing", None), "interop.sheet.not_found"),
        (write_request("Import", Some("ab")), "csv.invalid_delimiter"),
    ] {
        assert_eq!(
            session
                .save_csv_bytes(&request)
                .expect_err("invalid request")
                .code(),
            code
        );
    }
    let error = WorkbookSession::open_csv_bytes(b"\"open", &CsvReadOptionsDto::default())
        .expect_err("unterminated");
    assert_eq!(error.code(), "csv.read.unterminated_quote");
}
