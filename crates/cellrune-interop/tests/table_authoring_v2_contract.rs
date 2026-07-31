use std::path::PathBuf;

use cellrune_interop::{
    CalculationOptionsDto, CancellationToken, EditBatchV2Dto, RecalculationModeDto,
    TableChangeV2Dto, WorkbookChangeV2Dto, WorkbookSession, WriteOptionsDto,
};

#[test]
fn table_authoring_v2_preserves_v1_and_reopens_with_stable_ids() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../binding-contract/table-authoring-v2.xlsx");
    let mut session = WorkbookSession::open_path(fixture).expect("shared table fixture");
    let revision = session.summary().semantic_revision;
    let receipt = session
        .apply_changes_v2(
            revision,
            EditBatchV2Dto {
                changes: vec![
                    WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTable {
                        table_id: 1,
                        new_display_name: "Orders".to_owned(),
                    }),
                    WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTableColumn {
                        table_id: 1,
                        column_id: 2,
                        new_name: "Gross Amount".to_owned(),
                    }),
                    WorkbookChangeV2Dto::Table(TableChangeV2Dto::ResizeTableRows {
                        table_id: 1,
                        first_data_row: 2,
                        last_data_row: 4,
                    }),
                ],
            },
        )
        .expect("v2 table authoring");
    assert_eq!(receipt.receipt.schema_version, 2);
    assert_eq!(receipt.changed_table_ids, [1]);
    assert_eq!(receipt.receipt.applied_change_count, 3);

    assert_table_summary(&session);
    session
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("recalculate authored workbook");
    let (bytes, _) = session
        .save_bytes(WriteOptionsDto {
            invalidate_unavailable: true,
            ..WriteOptionsDto::default()
        })
        .expect("save authored workbook");
    let reopened = WorkbookSession::open_bytes(&bytes).expect("reopen authored workbook");
    assert_table_summary(&reopened);
}

#[test]
fn table_authoring_v2_cancellation_and_validation_leave_the_session_unchanged() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../binding-contract/table-authoring-v2.xlsx");
    let session = WorkbookSession::open_path(fixture).expect("shared table fixture");
    let revision = session.summary().semantic_revision;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = session
        .prepare_changes_v2_cancellable(
            revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTable {
                    table_id: 1,
                    new_display_name: "Orders".to_owned(),
                })],
            },
            &cancellation,
        )
        .expect_err("pre-cancelled edit");
    assert_eq!(cancelled.code(), "session.cancelled");
    assert_eq!(session.summary().semantic_revision, revision);

    let unknown = session
        .prepare_changes_v2(
            revision,
            EditBatchV2Dto {
                changes: vec![WorkbookChangeV2Dto::Table(
                    TableChangeV2Dto::RenameTableColumn {
                        table_id: 1,
                        column_id: 99,
                        new_name: "Missing".to_owned(),
                    },
                )],
            },
        )
        .expect_err("unknown stable column ID");
    assert_eq!(unknown.code(), "validation.unknown_table_column_id");
    assert_eq!(session.summary().semantic_revision, revision);
}

fn assert_table_summary(session: &WorkbookSession) {
    let summary = session.summary();
    let table = summary.sheets[0]
        .tables
        .iter()
        .find(|table| table.id == 1)
        .expect("edited table");
    assert_eq!(table.id, 1);
    assert_eq!(table.name, "Orders");
    assert_eq!(table.display_name, "Orders");
    assert_eq!(table.range, "A1:C5");
    assert_eq!(table.columns[1].id, 2);
    assert_eq!(table.columns[1].name, "Gross Amount");
    let empty = summary.sheets[0]
        .tables
        .iter()
        .find(|table| table.id == 2)
        .expect("empty table");
    assert_eq!(empty.name, "EmptySales");
    assert_eq!(empty.range, "G1:H1");
    let inspection = session
        .inspect_defined_name(&cellrune_interop::DefinedNameInspectionRequestDto {
            name: "EmptyAmount".to_owned(),
            current_sheet: None,
        })
        .expect("empty defined name");
    assert!(matches!(
        inspection.result,
        cellrune_interop::DefinedNameInspectionResultDto::EmptyReference
    ));
}
