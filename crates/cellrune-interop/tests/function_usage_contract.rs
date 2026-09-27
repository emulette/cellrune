use cellrune_interop::{
    EditBatchDto, FunctionUsageEntryDto, FunctionUsageReportDto,
    INTEROP_FUNCTION_USAGE_SCHEMA_VERSION, WorkbookChangeDto, WorkbookSession,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    max_depth: u32,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    formula: String,
    call_count: String,
}

#[test]
fn usage_json_preserves_large_counts_and_read_only_session_state() {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../binding-contract/function-usage-v2.json"
    ))
    .expect("usage corpus");
    assert_eq!(corpus.schema_version, INTEROP_FUNCTION_USAGE_SCHEMA_VERSION);
    let mut session = WorkbookSession::create();
    let changes = (0..=corpus.max_depth)
        .map(|depth| WorkbookChangeDto::SetDefinedName {
            name: format!("Usage_{depth}"),
            scope_sheet: None,
            formula: if depth == 0 {
                "=SUM(1)".to_owned()
            } else {
                format!("=Usage_{}+Usage_{}", depth - 1, depth - 1)
            },
            hidden: false,
        })
        .collect();
    session.apply_changes(0, EditBatchDto { changes }).unwrap();
    for case in corpus.cases {
        session
            .set_formula("Sheet1", "A1", &case.formula, None)
            .unwrap();
        let summary = session.summary();
        let history = session.changes_since(0, 100).unwrap();
        let report = session.function_usage();
        assert_eq!(report.schema_version, corpus.schema_version);
        assert_eq!(report.formula_count, 1);
        assert_eq!(report.parsed_formula_count, 1);
        assert_eq!(report.unparsed_formula_count, 0);
        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.name, "SUM");
        assert!(entry.supported);
        assert_eq!(entry.call_count.to_string(), case.call_count);
        assert_eq!(entry.formula_count, 1);
        assert_eq!(entry.sample_cells.len(), 1);
        assert_eq!(entry.sample_cells[0].address, "A1");
        let serialized = serde_json::to_value(&report).unwrap();
        assert_eq!(serialized["entries"][0]["call_count"], case.call_count);
        assert_eq!(
            serde_json::from_value::<FunctionUsageReportDto>(serialized).unwrap(),
            report
        );
        assert_eq!(session.function_usage(), report);
        assert_eq!(session.summary(), summary);
        assert_eq!(session.changes_since(0, 100).unwrap(), history);
    }
}

#[test]
fn count_codec_rejects_inexact_or_out_of_range_json() {
    for count in [
        json!(1),
        json!(1.5),
        json!("1.5"),
        json!("-1"),
        json!(""),
        json!("18446744073709551616"),
        json!(null),
    ] {
        let entry = json!({
            "name": "SUM", "supported": true, "call_count": count,
            "formula_count": 1, "sample_cells": []
        });
        assert!(serde_json::from_value::<FunctionUsageEntryDto>(entry).is_err());
    }
    let schema = serde_json::to_value(schemars::schema_for!(FunctionUsageEntryDto)).unwrap();
    assert_eq!(schema["properties"]["call_count"]["type"], "string");
}
