use cellrune_interop::{
    CalculationOptionsDto, CalculationResultDto, CellValueDto, EditBatchV2Dto, RangeRequestDto,
    RecalculationModeDto, TargetCalculationRequestDto, TransactionDetailItemDto,
    TransactionDetailSectionDto, WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookSession,
    WritableCellValueDto, WriteOptionsDto,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    initial_input: f64,
    edited_input: f64,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    address: String,
    formula: String,
    initial: CellValueDto,
    edited: CellValueDto,
}

fn range() -> RangeRequestDto {
    RangeRequestDto {
        sheet: "Sheet1".to_owned(),
        start: "B1".to_owned(),
        end: "B9".to_owned(),
        offset: 0,
        limit: 100,
    }
}

fn value(value: &CellValueDto) -> CalculationResultDto {
    CalculationResultDto::Value {
        value: value.clone(),
    }
}

#[test]
fn corrected_values_survive_full_targets_auto_preview_and_saved_caches() {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../binding-contract/compatibility-v021.json"
    ))
    .unwrap();
    assert_eq!(corpus.schema_version, 1);
    let mut session = WorkbookSession::create();
    session
        .set_value(
            "Sheet1",
            "A1",
            WritableCellValueDto::Number {
                value: corpus.initial_input,
            },
        )
        .unwrap();
    for case in &corpus.cases {
        session
            .set_formula("Sheet1", &case.address, &case.formula, None)
            .unwrap();
    }
    // Request targets before a full calculation so this exercises evaluation, not cache reuse.
    let targets: TargetCalculationRequestDto = serde_json::from_value(serde_json::json!({
        "targets": [{"sheet":"Sheet1", "start":"B1", "end":"B9"}]
    }))
    .unwrap();
    let partial = session.calculate_targets(&targets).unwrap();
    assert_eq!(partial.evaluated_count, corpus.cases.len() as u64);
    for (cell, case) in partial.cells.iter().zip(&corpus.cases) {
        assert_eq!(cell.result, value(&case.initial), "{}", case.address);
    }
    session
        .recalculate(RecalculationModeDto::Full, CalculationOptionsDto::default())
        .unwrap();
    for (cell, case) in session
        .read_range(&range())
        .unwrap()
        .cells
        .iter()
        .zip(&corpus.cases)
    {
        assert_eq!(
            cell.calculated,
            Some(value(&case.initial)),
            "{}",
            case.address
        );
    }
    let summary = session.summary();
    let history = session.changes_since(0, 100).unwrap();
    let preview = session
        .preview_changes(
            summary.semantic_revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetValue {
                    sheet: "Sheet1".to_owned(),
                    address: "A1".to_owned(),
                    value: WritableCellValueDto::Number {
                        value: corpus.edited_input,
                    },
                })],
            },
            RecalculationModeDto::Auto,
            CalculationOptionsDto::default(),
        )
        .unwrap();
    let page = session
        .preview_changes_page(
            preview.preview_id,
            TransactionDetailSectionDto::PreviewResults,
            None,
            100,
        )
        .unwrap();
    for case in &corpus.cases {
        let previewed = page.items.iter().find_map(|item| match item {
            TransactionDetailItemDto::PreviewResult { cell, result, .. }
                if cell.address == case.address =>
            {
                result.as_ref()
            }
            _ => None,
        });
        if case.initial != case.edited {
            assert_eq!(previewed, Some(&value(&case.edited)), "{}", case.address);
        }
    }
    assert_eq!(session.summary(), summary);
    assert_eq!(session.changes_since(0, 100).unwrap(), history);
    session.discard_preview(preview.preview_id).unwrap();
    session
        .set_value(
            "Sheet1",
            "A1",
            WritableCellValueDto::Number {
                value: corpus.edited_input,
            },
        )
        .unwrap();
    session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .unwrap();
    for (cell, case) in session
        .read_range(&range())
        .unwrap()
        .cells
        .iter()
        .zip(&corpus.cases)
    {
        assert_eq!(
            cell.calculated,
            Some(value(&case.edited)),
            "{}",
            case.address
        );
    }
    let (bytes, report) = session.save_bytes(WriteOptionsDto::default()).unwrap();
    assert!(report.complete);
    let reopened = WorkbookSession::open_bytes(&bytes).unwrap();
    for (cell, case) in reopened
        .read_range(&range())
        .unwrap()
        .cells
        .iter()
        .zip(&corpus.cases)
    {
        assert_eq!(cell.source_value, case.edited, "{}", case.address);
    }
}
