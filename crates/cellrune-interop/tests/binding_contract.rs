use std::collections::BTreeMap;

use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellValueDto, MAX_PAGE_SIZE, RangeRequestDto,
    WorkbookSession, WritableCellValueDto, WriteOptionsDto,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    operations: Vec<Operation>,
    expected_numbers: Vec<ExpectedNumber>,
    invalid_address: String,
    invalid_address_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Operation {
    #[serde(rename = "set_number")]
    Number {
        sheet: String,
        address: String,
        value: f64,
    },
    #[serde(rename = "set_formula")]
    Formula {
        sheet: String,
        address: String,
        formula: String,
    },
    #[serde(rename = "set_dynamic_formula")]
    DynamicFormula {
        sheet: String,
        address: String,
        formula: String,
        dynamic_range: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedNumber {
    address: String,
    value: f64,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("../../../binding-contract/v1.json"))
        .expect("binding contract corpus must remain valid")
}

fn build_session(corpus: &Corpus) -> WorkbookSession {
    let mut session = WorkbookSession::create();
    for operation in &corpus.operations {
        match operation {
            Operation::Number {
                sheet,
                address,
                value,
            } => session
                .set_value(
                    sheet,
                    address,
                    WritableCellValueDto::Number { value: *value },
                )
                .expect("number operation must succeed"),
            Operation::Formula {
                sheet,
                address,
                formula,
            } => session
                .set_formula(sheet, address, formula, None)
                .expect("formula operation must succeed"),
            Operation::DynamicFormula {
                sheet,
                address,
                formula,
                dynamic_range,
            } => session
                .set_formula(sheet, address, formula, Some(dynamic_range))
                .expect("dynamic formula operation must succeed"),
        }
    }
    session
}

#[test]
fn versioned_corpus_calculates_writes_and_reopens() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    let mut session = build_session(&corpus);
    let report = session
        .calculate(CalculationOptionsDto::default())
        .expect("calculation must succeed");
    assert_eq!(report.formula_count, 3);
    assert_eq!(report.unavailable_count, 0);
    assert_eq!(report.materialized_cell_count, 6);

    let page = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "F2".to_owned(),
            offset: 0,
            limit: 100,
        })
        .expect("range read must succeed");
    let values = page
        .cells
        .into_iter()
        .filter_map(|cell| {
            let result = cell.calculated.unwrap_or(CalculationResultDto::Value {
                value: cell.source_value,
            });
            match result {
                CalculationResultDto::Value {
                    value: CellValueDto::Number { value },
                } => Some((cell.address, value)),
                _ => None,
            }
        })
        .collect::<BTreeMap<_, _>>();
    for expected in &corpus.expected_numbers {
        assert_eq!(values.get(&expected.address), Some(&expected.value));
    }

    let (bytes, write_report) = session
        .save_bytes(WriteOptionsDto::default())
        .expect("verified save must succeed");
    assert!(write_report.complete);
    assert_eq!(write_report.materialized_count, 6);

    let reopened = WorkbookSession::open_bytes(&bytes).expect("written package must reopen");
    let reopened_page = reopened
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "B1".to_owned(),
            end: "F1".to_owned(),
            offset: 0,
            limit: 10,
        })
        .expect("reopened range must be readable");
    let b1 = reopened_page
        .cells
        .iter()
        .find(|cell| cell.address == "B1")
        .expect("B1 must be returned");
    assert_eq!(b1.source_value, CellValueDto::Number { value: 5.0 });
}

#[test]
fn versioned_corpus_error_code_is_stable() {
    let corpus = corpus();
    let mut session = WorkbookSession::create();
    let error = session
        .set_value(
            "Sheet1",
            &corpus.invalid_address,
            WritableCellValueDto::Number { value: 1.0 },
        )
        .expect_err("invalid address must fail");
    assert_eq!(error.code(), corpus.invalid_address_code);
}

#[test]
fn range_reads_are_bounded_and_paginated() {
    let session = WorkbookSession::create();
    let first = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 0,
            limit: 3,
        })
        .expect("first page must succeed");
    assert_eq!(first.cells.len(), 3);
    assert_eq!(first.next_offset, Some(3));

    let second = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 3,
            limit: 3,
        })
        .expect("second page must succeed");
    assert_eq!(second.cells.len(), 1);
    assert_eq!(second.next_offset, None);

    let empty = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 4,
            limit: 0,
        })
        .expect("an offset at the end must return an empty page");
    assert!(empty.cells.is_empty());
    assert_eq!(empty.next_offset, None);

    let offset_error = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 5,
            limit: 1,
        })
        .expect_err("an offset beyond the end must fail");
    assert_eq!(offset_error.code(), "interop.page.offset_out_of_range");

    let limit_error = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 0,
            limit: MAX_PAGE_SIZE + 1,
        })
        .expect_err("a page above the hard limit must fail");
    assert_eq!(limit_error.code(), "interop.page.limit_exceeded");

    let maximum = session
        .read_range(&RangeRequestDto {
            sheet: "Sheet1".to_owned(),
            start: "A1".to_owned(),
            end: "B2".to_owned(),
            offset: 0,
            limit: MAX_PAGE_SIZE,
        })
        .expect("the exact hard limit must remain valid");
    assert_eq!(maximum.cells.len(), 4);
}
