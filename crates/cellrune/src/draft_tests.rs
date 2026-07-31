use std::collections::BTreeSet;

use super::WorkbookDraft;
use crate::{
    ApplyChangesError, CalculationHints, CalculationOptions, CellAddress, CellContent, CellRange,
    CellValue, DateSystem, EditBatch, FiniteNumber, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, FrozenPane, NumberFormat, OpenOptions, PhoneticRun, PhoneticTextRange,
    PhoneticWriteOptions, Provenance, RecalculationWriteOptions, RecalculationWritePolicy, Row,
    SavedResult, SessionErrorCode, SessionLimits, Sheet, SheetId, SheetName, SheetVisibility,
    Table, TableColumn, TableColumnId, TableColumnName, TableFormula, TableId, TableName,
    TotalsRowFunction, ValidationError, WorkbookCalculationSession, WorkbookChange,
    WorkbookSnapshot, WorkbookSource, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_bytes,
};

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid cell address")
}

fn formula(value: &str) -> FormulaCell {
    FormulaCell::new(
        FormulaDialect::ExcelA1,
        FormulaText::from_xlsx(value).expect("formula"),
        SavedResult::Missing,
        FormulaMetadata::Normal,
    )
}

fn table_draft() -> WorkbookDraft {
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let item = TableColumn::new(1, "Item", None)
        .expect("item")
        .with_metadata(Some("Total".to_owned()), None, None);
    let amount = TableColumn::new(2, "Amount", Some(TotalsRowFunction::Sum))
        .expect("amount")
        .with_metadata(
            None,
            Some(TableFormula::new(
                FormulaText::from_xlsx("[@Amount]").expect("calculated"),
                false,
            )),
            None,
        );
    let table = Table::new(
        TableId::new(1).expect("table ID"),
        TableName::new("Sales").expect("table name"),
        TableName::new("Sales").expect("display name"),
        CellRange::new(address("A1"), address("B4")).expect("table range"),
        1,
        1,
        vec![item, amount],
    )
    .expect("table");
    sheet.set_tables(vec![table]);
    for (cell, content) in [
        (
            "A1",
            CellContent::Literal(CellValue::Text("Item".to_owned())),
        ),
        (
            "B1",
            CellContent::Literal(CellValue::Text("Amount".to_owned())),
        ),
        ("A2", CellContent::Literal(CellValue::Text("A".to_owned()))),
        ("B2", CellContent::Formula(formula("[@Amount]"))),
        ("A3", CellContent::Literal(CellValue::Text("B".to_owned()))),
        ("B3", CellContent::Formula(formula("[@Amount]"))),
        (
            "A4",
            CellContent::Literal(CellValue::Text("Total".to_owned())),
        ),
        (
            "B4",
            CellContent::Formula(formula("SUBTOTAL(109,[Amount])")),
        ),
        (
            "D1",
            CellContent::Formula(formula("SUM(Sales[Amount])+[@Amount]")),
        ),
    ] {
        sheet.upsert_cell(address(cell), content, NumberFormat::default());
    }
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(crate::ProviderIdentity::writer(), None),
    )
    .expect("snapshot");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn two_table_draft() -> WorkbookDraft {
    let base = table_draft();
    let mut sheets = base.workbook().sheets().to_vec();
    let sheet = &mut sheets[0];
    let mut tables = sheet.tables().to_vec();
    tables.push(
        Table::new(
            TableId::new(2).expect("table ID"),
            TableName::new("Inventory").expect("table name"),
            TableName::new("Inventory").expect("display name"),
            CellRange::new(address("A7"), address("B9")).expect("table range"),
            1,
            0,
            vec![
                TableColumn::new(1, "Sku", None).expect("column"),
                TableColumn::new(2, "Stock", None).expect("column"),
            ],
        )
        .expect("second table"),
    );
    sheet.set_tables(tables);
    for (cell, value) in [("A7", "Sku"), ("B7", "Stock")] {
        sheet.upsert_cell(
            address(cell),
            CellContent::Literal(CellValue::Text(value.to_owned())),
            NumberFormat::default(),
        );
    }
    let workbook = WorkbookSnapshot::new(
        sheets,
        base.workbook().date_system(),
        base.workbook().calculation_hints(),
        base.workbook().source(),
        base.workbook().provenance().clone(),
    )
    .expect("snapshot");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn table_draft_with_row_counts(header_row_count: u32, totals_row_count: u32) -> WorkbookDraft {
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let height = header_row_count + totals_row_count + 2;
    let table = Table::new(
        TableId::new(1).expect("table ID"),
        TableName::new("Sales").expect("table name"),
        TableName::new("Sales").expect("display name"),
        CellRange::new(
            address("A1"),
            CellAddress::from_indices(height, 2).expect("range end"),
        )
        .expect("table range"),
        header_row_count,
        totals_row_count,
        vec![
            TableColumn::new(1, "Item", None).expect("column"),
            TableColumn::new(2, "Amount", None).expect("column"),
        ],
    )
    .expect("table");
    sheet.set_tables(vec![table]);
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(crate::ProviderIdentity::writer(), None),
    )
    .expect("snapshot");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

fn empty_table_draft() -> WorkbookDraft {
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("sheet name"),
        SheetVisibility::Visible,
    );
    let value = TableColumn::new(2, "Value", None)
        .expect("column")
        .with_metadata(
            None,
            Some(TableFormula::new(
                FormulaText::from_xlsx("[@Value]").expect("formula"),
                false,
            )),
            None,
        );
    let table = Table::new(
        TableId::new(1).expect("table ID"),
        TableName::new("EmptyTable").expect("table name"),
        TableName::new("EmptyTable").expect("display name"),
        CellRange::new(address("A1"), address("B1")).expect("table range"),
        1,
        0,
        vec![TableColumn::new(1, "Item", None).expect("column"), value],
    )
    .expect("empty table");
    sheet.set_tables(vec![table]);
    for (cell, value) in [("A1", "Item"), ("B1", "Value")] {
        sheet.upsert_cell(
            address(cell),
            CellContent::Literal(CellValue::Text(value.to_owned())),
            NumberFormat::default(),
        );
    }
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(crate::ProviderIdentity::writer(), None),
    )
    .expect("snapshot");
    WorkbookDraft::from_snapshot_for_test(workbook)
}

#[test]
fn literal_blank_and_clear_keep_sparse_state_and_revision_consistent() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let a1 = address("A1");
    let b1 = address("B1");

    draft
        .set_cell_value(
            sheet_id,
            a1,
            CellValue::Number(FiniteNumber::new(3.0).expect("finite")),
        )
        .expect("set literal");
    assert!(matches!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(a1)
            .expect("A1")
            .content(),
        CellContent::Literal(CellValue::Number(value)) if value.get() == 3.0
    ));

    draft
        .set_cell_value(sheet_id, a1, CellValue::Blank)
        .expect("blank clears the cell");
    assert!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(a1)
            .is_none()
    );

    draft
        .set_cell_value(sheet_id, b1, CellValue::Text("temporary".to_owned()))
        .expect("set B1");
    let before_clear = draft.semantic_revision();
    assert!(draft.clear_cell(sheet_id, b1).expect("clear existing B1"));
    assert_eq!(draft.semantic_revision(), before_clear + 1);
    assert!(
        draft
            .workbook()
            .sheet_by_id(sheet_id)
            .expect("sheet")
            .cell(b1)
            .is_none()
    );

    let before_missing_clear = draft.semantic_revision();
    assert!(!draft.clear_cell(sheet_id, b1).expect("clear missing B1"));
    assert_eq!(draft.semantic_revision(), before_missing_clear);
}

#[test]
fn visibility_rules_cover_noop_last_visible_and_hidden_transitions() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();

    let initial_revision = draft.semantic_revision();
    draft
        .set_sheet_visibility(first, SheetVisibility::Visible)
        .expect("visible noop");
    assert_eq!(draft.semantic_revision(), initial_revision);

    assert!(matches!(
        draft.set_sheet_visibility(first, SheetVisibility::Hidden),
        Err(ValidationError::LastVisibleSheet)
    ));

    let second = draft
        .add_sheet(SheetName::new("Second").expect("sheet name"))
        .expect("second sheet");
    draft
        .set_sheet_visibility(first, SheetVisibility::Hidden)
        .expect("hide one of two visible sheets");
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(first)
            .expect("first sheet")
            .visibility(),
        SheetVisibility::Hidden
    );

    draft
        .set_sheet_visibility(first, SheetVisibility::VeryHidden)
        .expect("hidden-to-very-hidden does not affect the visible-sheet invariant");
    assert_eq!(
        draft
            .workbook()
            .sheet_by_id(first)
            .expect("first sheet")
            .visibility(),
        SheetVisibility::VeryHidden
    );
    assert!(matches!(
        draft.set_sheet_visibility(second, SheetVisibility::Hidden),
        Err(ValidationError::LastVisibleSheet)
    ));
}

#[test]
fn annotated_text_edits_are_atomic_and_require_explicit_replacement_or_clear() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let cell = address("A1");
    draft
        .set_annotated_text(
            sheet_id,
            cell,
            "明日",
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "あした")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("annotated text");
    let semantic_revision = draft.semantic_revision();
    let presentation_revision = draft.presentation_revision();

    let invalid =
        PhoneticRun::new(PhoneticTextRange::new(1, 3).expect("range"), "invalid").expect("run");
    assert!(matches!(
        draft.set_phonetics(sheet_id, cell, vec![invalid], PhoneticWriteOptions::show()),
        Err(ValidationError::PhoneticRangeOutOfBounds { .. })
    ));
    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(draft.presentation_revision(), presentation_revision);
    assert_eq!(
        draft
            .presentation()
            .cell_phonetics(sheet_id, cell)
            .expect("phonetics")
            .runs()[0]
            .text(),
        "あした"
    );

    assert!(matches!(
        draft.set_cell_value(sheet_id, cell, CellValue::Text("翌日".to_owned())),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));
    assert!(matches!(
        draft.apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id,
            cell,
            CellValue::Text("翌日".to_owned()),
        )])),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));
    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(draft.presentation_revision(), presentation_revision);

    assert!(
        draft
            .clear_cell(sheet_id, cell)
            .expect("explicit clear succeeds")
    );
    assert!(
        draft
            .presentation()
            .cell_phonetics(sheet_id, cell)
            .is_none()
    );
}

#[test]
fn presentation_only_edits_reuse_the_existing_calculation_identity() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let cell = address("A1");
    draft
        .set_cell_value(sheet_id, cell, CellValue::Text("明日".to_owned()))
        .expect("text");
    let semantic_revision = draft.semantic_revision();
    let fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    draft
        .set_phonetics(
            sheet_id,
            cell,
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "あした")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("phonetics");
    draft
        .set_frozen_pane(sheet_id, FrozenPane::new(1, 0).expect("pane"))
        .expect("pane");

    assert_eq!(draft.semantic_revision(), semantic_revision);
    assert_eq!(
        crate::calculation::workbook_fingerprint(draft.workbook()),
        fingerprint
    );
    assert!(calculation.matches_workbook(draft.workbook()));
}

#[test]
fn table_rename_column_rename_and_resize_are_one_stable_atomic_vertical() {
    let mut draft = table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let column_id = TableColumnId::new(2).expect("column ID");
    let receipt = draft
        .apply_changes(EditBatch::new([
            WorkbookChange::rename_table(
                table_id,
                TableName::new("Orders").expect("new table name"),
            ),
            WorkbookChange::rename_table_column(
                table_id,
                column_id,
                TableColumnName::new("Gross.Amount").expect("new column name"),
            ),
            WorkbookChange::resize_table_rows(
                table_id,
                Row::new(2).expect("first data row"),
                Row::new(5).expect("last data row"),
            )
            .expect("resize"),
        ]))
        .expect("table edit");
    assert_eq!(receipt.changed_table_ids(), [table_id]);
    assert!(receipt.topology_changed());
    assert_eq!(receipt.result_revision(), receipt.base_revision() + 1);

    let table = draft.workbook().table_by_id(table_id).expect("table");
    assert_eq!(table.id(), table_id);
    assert_eq!(table.display_name().as_str(), "Orders");
    assert_eq!(table.columns()[1].column_id(), column_id);
    assert_eq!(table.columns()[1].name(), "Gross.Amount");
    assert_eq!(
        table.range(),
        CellRange::new(address("A1"), address("B6")).expect("range")
    );
    assert_eq!(
        table.columns()[1]
            .calculated_column_formula()
            .expect("calculated")
            .text()
            .as_str(),
        "[@[Gross.Amount]]"
    );

    let sheet = draft
        .workbook()
        .sheet_by_id(SheetId::new(1).expect("sheet ID"))
        .expect("sheet");
    assert!(matches!(
        sheet.cell(address("B1")).expect("header").content(),
        CellContent::Literal(CellValue::Text(value)) if value == "Gross.Amount"
    ));
    assert_eq!(
        sheet
            .cell(address("D1"))
            .expect("qualified formula")
            .content()
            .formula_text(),
        Some("SUM(Orders[[Gross.Amount]])+[@Amount]")
    );
    assert_eq!(
        sheet
            .cell(address("B5"))
            .expect("new calculated cell")
            .content()
            .formula_text(),
        Some("[@[Gross.Amount]]")
    );
    assert_eq!(
        sheet
            .cell(address("B6"))
            .expect("new totals cell")
            .content()
            .formula_text(),
        Some("SUBTOTAL(109,[[Gross.Amount]])")
    );
    assert!(matches!(
        sheet.cell(address("A6")).expect("totals label").content(),
        CellContent::Literal(CellValue::Text(value)) if value == "Total"
    ));
    assert_eq!(
        sheet
            .cell(address("B4"))
            .expect("old totals preserved")
            .content()
            .formula_text(),
        Some("SUBTOTAL(109,[[Gross.Amount]])")
    );

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let written = write_xlsx_draft_bytes(
        &draft,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("canonical write");
    let reopened = open_xlsx_document_bytes(written.bytes(), OpenOptions::default())
        .expect("canonical reopen");
    let reopened_table = reopened.workbook().table_by_id(table_id).expect("table");
    assert_eq!(reopened_table.display_name().as_str(), "Orders");
    assert_eq!(reopened_table.columns()[1].name(), "Gross.Amount");
    assert_eq!(
        reopened_table.range(),
        CellRange::new(address("A1"), address("B6")).expect("range")
    );
}

#[test]
fn table_edit_collision_is_atomic() {
    let mut draft = table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let sheet_id = SheetId::new(1).expect("sheet ID");
    draft
        .set_cell_value(
            sheet_id,
            address("A6"),
            CellValue::Text("user content".to_owned()),
        )
        .expect("seed collision");
    let revision = draft.semantic_revision();
    let before_fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());
    let error = draft
        .apply_changes(EditBatch::new([
            WorkbookChange::rename_table(
                table_id,
                TableName::new("Orders").expect("new table name"),
            ),
            WorkbookChange::resize_table_rows(
                table_id,
                Row::new(2).expect("first"),
                Row::new(5).expect("last"),
            )
            .expect("resize"),
        ]))
        .expect_err("collision");
    assert!(matches!(
        error,
        ValidationError::TableMaterializationCollision { .. }
    ));
    assert_eq!(draft.semantic_revision(), revision);
    assert_eq!(
        crate::calculation::workbook_fingerprint(draft.workbook()),
        before_fingerprint
    );
    assert_eq!(
        draft
            .workbook()
            .table_by_id(table_id)
            .expect("unchanged table")
            .display_name()
            .as_str(),
        "Sales"
    );
}

#[test]
fn table_resize_shrinks_without_deleting_cells_outside_the_new_range() {
    let mut draft = table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let sheet_id = SheetId::new(1).expect("sheet ID");
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::clear_cell(sheet_id, address("A3")),
            WorkbookChange::clear_cell(sheet_id, address("B3")),
            WorkbookChange::resize_table_rows(
                table_id,
                Row::new(2).expect("first"),
                Row::new(2).expect("last"),
            )
            .expect("resize"),
        ]))
        .expect("shrink");

    let table = draft.workbook().table_by_id(table_id).expect("table");
    assert_eq!(
        table.range(),
        CellRange::new(address("A1"), address("B3")).expect("range")
    );
    let sheet = draft.workbook().sheet_by_id(sheet_id).expect("sheet");
    assert!(matches!(
        sheet.cell(address("A3")).expect("new total").content(),
        CellContent::Literal(CellValue::Text(value)) if value == "Total"
    ));
    assert_eq!(
        sheet
            .cell(address("B3"))
            .expect("new total")
            .content()
            .formula_text(),
        Some("SUBTOTAL(109,[Amount])")
    );
    assert!(matches!(
        sheet.cell(address("A4")).expect("old total").content(),
        CellContent::Literal(CellValue::Text(value)) if value == "Total"
    ));
    assert_eq!(
        sheet
            .cell(address("B4"))
            .expect("old total")
            .content()
            .formula_text(),
        Some("SUBTOTAL(109,[Amount])")
    );
}

#[test]
fn header_only_table_can_expand_and_materialize_calculated_columns() {
    let mut draft = empty_table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let receipt = draft
        .apply_changes(EditBatch::new([WorkbookChange::resize_table_rows(
            table_id,
            Row::new(2).expect("first"),
            Row::new(3).expect("last"),
        )
        .expect("resize")]))
        .expect("expand empty table");
    assert_eq!(receipt.changed_table_ids(), [table_id]);
    let table = draft.workbook().table_by_id(table_id).expect("table");
    assert_eq!(
        table.range(),
        CellRange::new(address("A1"), address("B3")).expect("range")
    );
    let sheet = draft.workbook().sheets().first().expect("sheet");
    for cell in ["B2", "B3"] {
        assert_eq!(
            sheet
                .cell(address(cell))
                .expect("materialized formula")
                .content()
                .formula_text(),
            Some("[@Value]")
        );
    }
}

#[test]
fn table_resize_materialization_limit_fails_before_installation() {
    let draft = empty_table_draft();
    let original_revision = draft.semantic_revision();
    let original_fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());
    let limits = SessionLimits::default()
        .with_table_materialization_limit(1)
        .expect("limits");
    let mut session = WorkbookCalculationSession::with_limits(draft, limits);
    let error = session
        .apply_changes(
            original_revision,
            EditBatch::new([WorkbookChange::resize_table_rows(
                TableId::new(1).expect("table ID"),
                Row::new(2).expect("first"),
                Row::new(3).expect("last"),
            )
            .expect("resize")]),
        )
        .expect_err("second formula cell exceeds the limit");
    assert!(matches!(
        error,
        ApplyChangesError::Session(ref error)
            if error.code() == SessionErrorCode::TableMaterializationLimitExceeded
    ));
    assert_eq!(session.workbook().semantic_revision(), original_revision);
    assert_eq!(
        crate::calculation::workbook_fingerprint(session.workbook()),
        original_fingerprint
    );
}

#[test]
fn table_column_rename_preserves_annotation_and_exact_change_ledger_invariants() {
    let table_id = TableId::new(1).expect("table ID");
    let column_id = TableColumnId::new(2).expect("column ID");
    let sheet_id = SheetId::new(1).expect("sheet ID");

    let mut annotated = table_draft();
    annotated
        .set_phonetics(
            sheet_id,
            address("B1"),
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 6).expect("range"), "アマウント")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("phonetics");
    assert!(matches!(
        annotated.apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
            table_id,
            column_id,
            TableColumnName::new("Gross").expect("name"),
        )])),
        Err(ValidationError::AnnotatedTextReplacementRequired { .. })
    ));
    assert_eq!(
        annotated
            .workbook()
            .table_by_id(table_id)
            .expect("table")
            .columns()[1]
            .name(),
        "Amount"
    );

    let mut already_materialized = table_draft();
    already_materialized
        .set_cell_value(sheet_id, address("B1"), CellValue::Text("Gross".to_owned()))
        .expect("pre-existing target header");
    let receipt = already_materialized
        .apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
            table_id,
            column_id,
            TableColumnName::new("Gross").expect("name"),
        )]))
        .expect("rename");
    assert!(
        !receipt
            .changed_cells()
            .contains(&crate::CalculationCellId::new(sheet_id, address("B1")))
    );
}

#[test]
fn table_authoring_rejects_invalid_targets_names_and_overlap_atomically() {
    let table_id = TableId::new(1).expect("table ID");
    let missing_table_id = TableId::new(99).expect("missing table ID");
    let missing_column_id = TableColumnId::new(99).expect("missing column ID");
    let mut draft = two_table_draft();
    let revision = draft.semantic_revision();
    let fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());

    let cases = [
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::rename_table(
                missing_table_id,
                TableName::new("Missing").expect("name"),
            )])),
            ValidationError::UnknownTableId { value: 99 },
        ),
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
                table_id,
                missing_column_id,
                TableColumnName::new("Missing").expect("column name"),
            )])),
            ValidationError::UnknownTableColumnId {
                table_id: 1,
                column_id: 99,
            },
        ),
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::rename_table(
                table_id,
                TableName::new("A1").expect("compatible core name"),
            )])),
            ValidationError::TableNameReferenceConflict,
        ),
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::rename_table(
                table_id,
                TableName::new("inventory").expect("colliding name"),
            )])),
            ValidationError::DuplicateTableDisplayName {
                name: "Inventory".to_owned(),
            },
        ),
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
                table_id,
                TableColumnId::new(2).expect("column ID"),
                TableColumnName::new("item").expect("duplicate column name"),
            )])),
            ValidationError::DuplicateTableColumnName {
                name: "item".to_owned(),
            },
        ),
        (
            draft.apply_changes(EditBatch::new([WorkbookChange::resize_table_rows(
                table_id,
                Row::new(2).expect("first"),
                Row::new(7).expect("last"),
            )
            .expect("resize")])),
            ValidationError::OverlappingTables {
                sheet_id: 1,
                first_table_id: 1,
                second_table_id: 2,
            },
        ),
    ];

    for (result, expected) in cases {
        assert_eq!(result.expect_err("invalid table edit"), expected);
        assert_eq!(draft.semantic_revision(), revision);
        assert_eq!(
            crate::calculation::workbook_fingerprint(draft.workbook()),
            fingerprint
        );
    }
}

#[test]
fn table_authoring_fails_closed_for_unsupported_multirow_materialization() {
    let table_id = TableId::new(1).expect("table ID");
    let mut multi_header = table_draft_with_row_counts(2, 0);
    assert!(matches!(
        multi_header.apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
            table_id,
            TableColumnId::new(1).expect("column ID"),
            TableColumnName::new("Product").expect("column name"),
        )])),
        Err(ValidationError::UnsupportedTableAuthoringMetadata { table_id: 1 })
    ));
    assert!(matches!(
        multi_header.apply_changes(EditBatch::new([WorkbookChange::resize_table_rows(
            table_id,
            Row::new(3).expect("first"),
            Row::new(5).expect("last"),
        )
        .expect("resize")])),
        Err(ValidationError::UnsupportedTableAuthoringMetadata { table_id: 1 })
    ));

    let mut multi_totals = table_draft_with_row_counts(1, 2);
    assert!(matches!(
        multi_totals.apply_changes(EditBatch::new([WorkbookChange::resize_table_rows(
            table_id,
            Row::new(2).expect("first"),
            Row::new(4).expect("last"),
        )
        .expect("resize")])),
        Err(ValidationError::UnsupportedTableAuthoringMetadata { table_id: 1 })
    ));
}

#[test]
fn related_malformed_formula_aborts_table_rename_atomically() {
    let mut draft = table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let sheet_id = SheetId::new(1).expect("sheet ID");
    draft
        .set_cell_formula(
            sheet_id,
            address("E1"),
            FormulaText::from_xlsx("SUM(Sales[[Amount])").expect("stored formula"),
        )
        .expect("seed malformed formula");
    let revision = draft.semantic_revision();
    let fingerprint = crate::calculation::workbook_fingerprint(draft.workbook());
    let error = draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_table(
            table_id,
            TableName::new("Orders").expect("new name"),
        )]))
        .expect_err("related malformed formula must stop the rename");
    assert!(matches!(
        error,
        ValidationError::FormulaRewriteParseFailed {
            owner: Some(ref owner),
            ..
        } if owner == "cell:sheet_id=1,address=E1"
    ));
    assert_eq!(draft.semantic_revision(), revision);
    assert_eq!(
        crate::calculation::workbook_fingerprint(draft.workbook()),
        fingerprint
    );
    assert_eq!(
        draft
            .workbook()
            .table_by_id(table_id)
            .expect("table")
            .display_name()
            .as_str(),
        "Sales"
    );
}

#[test]
fn table_receipt_reports_only_tables_changed_by_the_current_batch() {
    let mut draft = table_draft();
    let table_id = TableId::new(1).expect("table ID");
    let sheet_id = SheetId::new(1).expect("sheet ID");
    let first = draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_table(
            table_id,
            TableName::new("Orders").expect("new table name"),
        )]))
        .expect("table rename");
    assert_eq!(first.changed_table_ids(), [table_id]);

    let unrelated = draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id,
            address("E1"),
            CellValue::Number(FiniteNumber::new(1.0).expect("finite")),
        )]))
        .expect("unrelated cell edit");
    assert!(unrelated.changed_table_ids().is_empty());
    assert_eq!(
        draft.changed_table_ids(),
        &BTreeSet::from([table_id]),
        "writer metadata must retain the cumulative table patch set"
    );

    let second = draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_table_column(
            table_id,
            TableColumnId::new(2).expect("column ID"),
            TableColumnName::new("Gross").expect("column name"),
        )]))
        .expect("second table edit");
    assert_eq!(second.changed_table_ids(), [table_id]);
}

trait CellContentFormulaText {
    fn formula_text(&self) -> Option<&str>;
}

impl CellContentFormulaText for CellContent {
    fn formula_text(&self) -> Option<&str> {
        match self {
            Self::Formula(formula) => formula.text().map(FormulaText::as_str),
            Self::Literal(_) => None,
        }
    }
}
