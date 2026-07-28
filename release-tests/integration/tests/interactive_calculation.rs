use cellrune::{
    ApplyChangesError, ArithmeticSemantics, CalculationCellId, CalculationCellResult,
    CalculationDecisionReason, CalculationExecutionMode, CalculationHints, CalculationIssueCode,
    CalculationLimits, CalculationMode, CalculationOptions, CancellationToken, CellAddress,
    CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope, EditBatch,
    FiniteNumber, FormulaText, NumberFormat, NumberFormatKind, RecalculationMode, SessionErrorCode,
    SessionLimits, SheetId, SheetName, SheetVisibility, WorkbookCalculationSession, WorkbookChange,
    calculate_workbook,
};

#[test]
fn ten_thousand_cell_batch_commits_once_and_invalid_batches_roll_back() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let changes = (1..=10_000)
        .map(|row| {
            WorkbookChange::set_cell_value(
                sheet_id,
                CellAddress::from_indices(row, 1).expect("valid generated address"),
                number(f64::from(row)),
            )
        })
        .collect::<Vec<_>>();

    let receipt = session
        .apply_changes(0, EditBatch::new(changes))
        .expect("batch commits");

    assert_eq!(receipt.base_revision(), 0);
    assert_eq!(receipt.result_revision(), 1);
    assert_eq!(receipt.applied_change_count(), 10_000);
    assert_eq!(receipt.changed_cells().len(), 10_000);
    assert_eq!(session.workbook().semantic_revision(), 1);
    assert_eq!(
        session
            .workbook()
            .sheet_by_id(sheet_id)
            .map(|sheet| sheet.len()),
        Some(10_000)
    );

    let before = session.workbook().semantic_revision();
    let invalid = EditBatch::new([
        WorkbookChange::set_cell_value(sheet_id, address("B1"), number(1.0)),
        WorkbookChange::set_cell_value(
            SheetId::new(999).expect("valid absent sheet ID"),
            address("A1"),
            number(2.0),
        ),
    ]);
    let error = session
        .apply_changes(before, invalid)
        .expect_err("invalid operation rolls the whole batch back");
    assert!(matches!(error, ApplyChangesError::Validation(_)));
    assert_eq!(session.workbook().semantic_revision(), before);
    assert!(
        session
            .workbook()
            .sheet_by_id(sheet_id)
            .and_then(|sheet| sheet.cell(address("B1")))
            .is_none()
    );
}

#[test]
fn sheet_rename_receipt_includes_every_rewritten_formula_cell() {
    let mut session = WorkbookCalculationSession::create();
    let first = SheetId::new(1).expect("constant sheet ID");
    let second = session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::add_sheet(
                SheetName::new("Second").expect("valid sheet name"),
            )]),
        )
        .expect("second sheet")
        .created_sheet_ids()[0];
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                second,
                address("A1"),
                formula("Sheet1!A1+1"),
            )]),
        )
        .expect("cross-sheet formula");

    let receipt = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::rename_sheet(
                first,
                SheetName::new("Inputs").expect("valid renamed sheet"),
            )]),
        )
        .expect("sheet rename");

    let rewritten = CalculationCellId::new(second, address("A1"));
    assert_eq!(receipt.changed_cells(), &[rewritten]);
    assert_eq!(receipt.calculation_changed_cells(), &[rewritten]);
    let CellContent::Formula(formula) = session
        .workbook()
        .sheet_by_id(second)
        .and_then(|sheet| sheet.cell(address("A1")))
        .expect("rewritten formula cell")
        .content()
    else {
        panic!("rewritten cell must remain a formula");
    };
    assert_eq!(formula.text().map(FormulaText::as_str), Some("Inputs!A1+1"));
}

#[test]
fn batch_sheet_name_validation_uses_unicode_case_insensitive_keys() {
    let mut session = WorkbookCalculationSession::create();
    let unicode = session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::add_sheet(
                SheetName::new("Ä").expect("valid Unicode sheet name"),
            )]),
        )
        .expect("first Unicode spelling is accepted")
        .created_sheet_ids()[0];
    let revision = session.workbook().semantic_revision();

    for change in [
        WorkbookChange::add_sheet(SheetName::new("ä").expect("valid case variant")),
        WorkbookChange::rename_sheet(
            SheetId::new(1).expect("constant sheet ID"),
            SheetName::new("ä").expect("valid case variant"),
        ),
    ] {
        let error = session
            .apply_changes(revision, EditBatch::new([change]))
            .expect_err("Unicode case variant must be rejected");
        assert!(matches!(
            error,
            ApplyChangesError::Validation(cellrune::ValidationError::DuplicateSheetName { .. })
        ));
        assert_eq!(session.workbook().semantic_revision(), revision);
        assert_eq!(
            session
                .workbook()
                .sheet_by_id(unicode)
                .expect("original Unicode sheet remains")
                .name()
                .as_str(),
            "Ä"
        );
    }
}

#[test]
fn semantic_noop_batch_preserves_revision_topology_and_calculation() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let stable_name = defined_name("Stable", "A1");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+1")),
                WorkbookChange::set_defined_name(stable_name.clone()),
            ]),
        )
        .expect("initial semantic state");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let revision = session.workbook().semantic_revision();

    let receipt = session
        .apply_changes(
            revision,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+1")),
                WorkbookChange::clear_cell(sheet_id, address("Z99")),
                WorkbookChange::set_cell_number_format(
                    sheet_id,
                    address("A1"),
                    NumberFormat::default(),
                ),
                WorkbookChange::rename_sheet(
                    sheet_id,
                    SheetName::new("Sheet1").expect("current sheet name"),
                ),
                WorkbookChange::set_sheet_visibility(sheet_id, SheetVisibility::Visible),
                WorkbookChange::set_defined_name(stable_name),
                WorkbookChange::remove_defined_name(DefinedNameScope::Workbook, "Missing"),
                WorkbookChange::set_date_system(DateSystem::Excel1900),
                WorkbookChange::set_calculation_hints(CalculationHints::default()),
            ]),
        )
        .expect("semantic no-op batch");

    assert_eq!(receipt.applied_change_count(), 10);
    assert_eq!(receipt.base_revision(), revision);
    assert_eq!(receipt.result_revision(), revision);
    assert!(receipt.changed_cells().is_empty());
    assert!(receipt.calculation_changed_cells().is_empty());
    assert!(receipt.created_sheet_ids().is_empty());
    assert!(!receipt.topology_changed());
    assert!(!receipt.calculation_metadata_changed());
    assert_eq!(session.workbook().semantic_revision(), revision);
    let calculation = session
        .calculation()
        .expect("no-op batch preserves installed calculation");
    assert_eq!(
        calculation.cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(2.0)))
    );
    let warm = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("no-op batch remains incrementally warm");
    assert_eq!(warm.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(warm.reason(), CalculationDecisionReason::NoDirtyFormulas);
    assert_eq!(warm.evaluated_count(), 0);
}

#[test]
fn index_reference_dependencies_remain_incrementally_safe() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(3.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B1"), number(10.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B2"), number(20.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B3"), number(30.0)),
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("D1"),
                    formula("SUM(INDEX(A1:B3,0,C1))"),
                ),
            ]),
        )
        .expect("INDEX dependency workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial INDEX calculation");

    for (address_text, value, expected) in [("C1", 2.0, 60.0), ("B2", 25.0, 65.0)] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet_id,
                    address(address_text),
                    number(value),
                )]),
            )
            .expect("INDEX dependency edit");
        let delta = session
            .recalculate(
                RecalculationMode::Incremental,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("incremental INDEX recalculation");

        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(
            delta.reason(),
            CalculationDecisionReason::IncrementalRequested
        );
        assert_eq!(delta.evaluated_count(), 1);
        assert_eq!(
            session
                .calculation()
                .expect("installed INDEX calculation")
                .cell(CalculationCellId::new(sheet_id, address("D1"))),
            Some(&CalculationCellResult::Value(number(expected)))
        );
    }
}

#[test]
fn three_d_dependencies_invalidate_incremental_consumers_across_the_span() {
    let mut session = WorkbookCalculationSession::create();
    let first = SheetId::new(1).expect("constant sheet ID");
    let added = session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::add_sheet(
                    SheetName::new("Sheet2").expect("valid second sheet name"),
                ),
                WorkbookChange::add_sheet(
                    SheetName::new("Sheet3").expect("valid third sheet name"),
                ),
            ]),
        )
        .expect("additional sheets")
        .created_sheet_ids()
        .to_vec();
    let fourth_name = SheetName::new("Sheet4").expect("valid fourth sheet name");
    let fourth = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::add_sheet(fourth_name)]),
        )
        .expect("outside sheet")
        .created_sheet_ids()[0];
    let [second, third] = added.as_slice() else {
        panic!("two span sheets must be created");
    };
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([
                WorkbookChange::set_cell_value(first, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(*second, address("A1"), number(2.0)),
                WorkbookChange::set_cell_value(*third, address("A1"), number(3.0)),
                WorkbookChange::set_cell_value(fourth, address("A1"), number(4.0)),
                WorkbookChange::set_cell_formula(
                    first,
                    address("B1"),
                    formula("SUM(Sheet1:Sheet3!A1)"),
                ),
            ]),
        )
        .expect("3-D dependency workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    for (sheet, value, expected, evaluated) in [
        (*second, 20.0, 24.0, 1),
        (first, 10.0, 33.0, 1),
        (*third, 30.0, 60.0, 1),
        (fourth, 40.0, 60.0, 0),
    ] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    address("A1"),
                    number(value),
                )]),
            )
            .expect("dependency edit");
        let delta = session
            .recalculate(
                RecalculationMode::Incremental,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("incremental 3-D recalculation");

        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(delta.evaluated_count(), evaluated);
        assert_eq!(
            session
                .calculation()
                .expect("installed incremental calculation")
                .cell(CalculationCellId::new(first, address("B1"))),
            Some(&CalculationCellResult::Value(number(expected))),
        );
    }
}

/// A whole-column array must not read cells outside its own columns.
///
/// The materialized height is what makes this an incremental-safety property rather than a
/// cosmetic one: if it came from the sheet-wide used range, writing into any unreferenced column
/// would change the correct answer while leaving every dependency rectangle untouched, so the
/// incremental pass would keep a stale value that a full pass disagrees with.
#[test]
fn whole_column_arrays_agree_between_incremental_and_full_recalculation() {
    let mut session = WorkbookCalculationSession::create();
    let first = SheetId::new(1).expect("constant sheet ID");
    let data = session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::add_sheet(
                SheetName::new("Data").expect("valid sheet name"),
            )]),
        )
        .expect("data sheet")
        .created_sheet_ids()[0];
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([
                WorkbookChange::set_cell_value(data, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(data, address("A2"), number(2.0)),
                WorkbookChange::set_cell_value(data, address("A3"), number(3.0)),
                WorkbookChange::set_cell_value(data, address("B1"), number(10.0)),
                WorkbookChange::set_cell_value(data, address("B2"), number(20.0)),
                WorkbookChange::set_cell_formula(
                    first,
                    address("D1"),
                    formula("COUNT(Data!A:A*Data!B:B)"),
                ),
                WorkbookChange::set_cell_formula(
                    first,
                    address("D2"),
                    formula("AVERAGE(Data!A:A*Data!B:B)"),
                ),
            ]),
        )
        .expect("whole-column workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    // `Z10` is outside every referenced column, so nothing may go dirty and nothing may change.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                data,
                address("Z10"),
                number(999.0),
            )]),
        )
        .expect("unreferenced column edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental recalculation");
    assert_eq!(delta.evaluated_count(), 0);
    let incremental = [
        session
            .calculation()
            .expect("incremental")
            .cell(CalculationCellId::new(first, address("D1"))),
        session
            .calculation()
            .expect("incremental")
            .cell(CalculationCellId::new(first, address("D2"))),
    ]
    .map(|result| result.cloned());

    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full recalculation");
    let full = [
        session
            .calculation()
            .expect("full")
            .cell(CalculationCellId::new(first, address("D1"))),
        session
            .calculation()
            .expect("full")
            .cell(CalculationCellId::new(first, address("D2"))),
    ]
    .map(|result| result.cloned());

    assert_eq!(incremental, full);
    assert_eq!(
        full[0],
        Some(CalculationCellResult::Value(number(3.0))),
        "the count must follow the referenced columns, not the sheet used range",
    );

    // The opposite direction: growth inside a referenced column is covered by that column's
    // dependency rectangle, so it must dirty the consumer and widen the extent.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                data,
                address("A5"),
                number(4.0),
            )]),
        )
        .expect("referenced column edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental recalculation");
    assert_eq!(delta.evaluated_count(), 2);
    let grown = session
        .calculation()
        .expect("incremental")
        .cell(CalculationCellId::new(first, address("D1")))
        .cloned();

    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full recalculation");
    assert_eq!(
        grown,
        session
            .calculation()
            .expect("full")
            .cell(CalculationCellId::new(first, address("D1")))
            .cloned(),
    );
    assert_eq!(
        grown,
        Some(CalculationCellResult::Value(number(5.0))),
        "column A now reaches row 5, so both operands materialize five rows",
    );
}

/// A spill member is not a formula cell, so its exact decimal has to be carried in the snapshot
/// under its own address or an incremental pass cannot restore it.
///
/// `B1:B3` spills `0.1`, `0.2`, `-0.3`, and `D1` sums them to exactly zero before scaling by `E1`.
/// Editing only `E1` leaves the spill clean, so its values are reseeded from the snapshot — and if
/// their decimals are not reseeded with them the sum keeps its `5.55e-17` residue, which the scale
/// then magnifies into a plainly nonzero answer that the full calculation never produced.
#[test]
fn spilled_decimal_traces_survive_an_incremental_recalculation() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let excel =
        CalculationOptions::default().with_arithmetic_semantics(ArithmeticSemantics::ExcelNearZero);
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(0.1)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(0.2)),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(-0.3)),
                // A declared spill range: an undeclared one makes the whole workbook
                // incremental-unsafe, which would take the path under test out of reach.
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("B1"),
                    formula("A1:A3+0"),
                    Some(CellRange::new(address("B1"), address("B3")).expect("valid spill range")),
                )
                .expect("valid dynamic spill change"),
                WorkbookChange::set_cell_value(sheet_id, address("E1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("D1"), formula("SUM(B1:B3)*E1")),
            ]),
        )
        .expect("spill workbook");
    session
        .recalculate(RecalculationMode::Auto, excel, CancellationToken::new())
        .expect("full calculation");
    assert_eq!(
        session
            .calculation()
            .expect("installed full calculation")
            .cell(CalculationCellId::new(sheet_id, address("D1"))),
        Some(&CalculationCellResult::Value(number(0.0))),
        "the full calculation must snap the spilled sum"
    );

    // Dirty only the consumer, and only through a value edit so the pass stays incremental. The
    // spill is untouched, so it is restored from the snapshot rather than recalculated.
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("E1"),
                number(1_000_000.0),
            )]),
        )
        .expect("consumer edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            excel,
            CancellationToken::new(),
        )
        .expect("incremental recalculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        session
            .calculation()
            .expect("installed incremental calculation")
            .cell(CalculationCellId::new(sheet_id, address("D1"))),
        Some(&CalculationCellResult::Value(number(0.0))),
        "the incremental pass disagreed with the full calculation"
    );
}

#[test]
fn explicit_intersection_index_selection_remains_incrementally_safe() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(3.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B1"), number(10.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B2"), number(20.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B3"), number(30.0)),
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("D2"),
                    formula("@INDEX(A1:B3,0,C1)"),
                ),
            ]),
        )
        .expect("explicit-intersection INDEX workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial explicit-intersection INDEX calculation");

    for (address_text, value, expected) in [("C1", 2.0, 20.0), ("B2", 25.0, 25.0)] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet_id,
                    address(address_text),
                    number(value),
                )]),
            )
            .expect("explicit-intersection INDEX dependency edit");
        let delta = session
            .recalculate(
                RecalculationMode::Incremental,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("incremental explicit-intersection INDEX recalculation");

        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(
            delta.reason(),
            CalculationDecisionReason::IncrementalRequested
        );
        assert_eq!(delta.evaluated_count(), 1);
        assert_eq!(
            session
                .calculation()
                .expect("installed explicit-intersection INDEX calculation")
                .cell(CalculationCellId::new(sheet_id, address("D2"))),
            Some(&CalculationCellResult::Value(number(expected)))
        );
    }
}

#[test]
fn explicit_intersection_range_endpoints_remain_incrementally_safe() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(3.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A4"), number(4.0)),
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("C2"), number(3.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("D2"),
                    formula("@(INDEX(A1:A4,C1):INDEX(A1:A4,C2))"),
                ),
            ]),
        )
        .expect("explicit-intersection range workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial explicit-intersection range calculation");

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("C1"),
                number(3.0),
            )]),
        )
        .expect("explicit-intersection range endpoint edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental explicit-intersection range recalculation");

    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        delta.reason(),
        CalculationDecisionReason::IncrementalRequested
    );
    assert_eq!(delta.evaluated_count(), 1);
    assert_eq!(
        session
            .calculation()
            .expect("installed explicit-intersection range calculation")
            .cell(CalculationCellId::new(sheet_id, address("D2"))),
        Some(&CalculationCellResult::Value(number(3.0)))
    );
}

#[test]
fn explicit_intersection_plain_range_tracks_only_the_selected_cell() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(3.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("D2"), formula("@A:A")),
            ]),
        )
        .expect("plain explicit-intersection workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial plain explicit-intersection calculation");

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A3"),
                number(30.0),
            )]),
        )
        .expect("non-selected explicit-intersection source edit");
    let unchanged = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("non-selected source stays clean");
    assert_eq!(
        unchanged.reason(),
        CalculationDecisionReason::NoDirtyFormulas
    );
    assert_eq!(unchanged.evaluated_count(), 0);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A2"),
                number(20.0),
            )]),
        )
        .expect("selected explicit-intersection source edit");
    let changed = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("selected source recalculates");
    assert_eq!(
        changed.reason(),
        CalculationDecisionReason::IncrementalRequested
    );
    assert_eq!(changed.evaluated_count(), 1);
    assert_eq!(
        session
            .calculation()
            .expect("installed plain explicit-intersection calculation")
            .cell(CalculationCellId::new(sheet_id, address("D2"))),
        Some(&CalculationCellResult::Value(number(20.0)))
    );
}

#[test]
fn incremental_chain_matches_full_oracle_and_reports_only_changed_results() {
    let mut session = chain_session();
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    assert_eq!(initial.parsed_formula_count(), 3);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                SheetId::new(1).expect("constant sheet ID"),
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("literal edit");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");

    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(delta.reason(), CalculationDecisionReason::DirtySubset);
    assert_eq!(delta.dirty_count(), 2);
    assert_eq!(delta.evaluated_count(), 2);
    assert_eq!(delta.parsed_formula_count(), 0);
    assert_eq!(
        delta
            .changed_cells()
            .iter()
            .map(|cell| cell.cell().address().to_string())
            .collect::<Vec<_>>(),
        vec!["B1", "C1"]
    );

    let oracle = calculate_workbook(session.workbook(), CalculationOptions::default());
    let installed = session.calculation().expect("installed calculation");
    assert_calculations_equal(installed, &oracle);

    let warm = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("unchanged warm calculation");
    assert_eq!(warm.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(warm.reason(), CalculationDecisionReason::NoDirtyFormulas);
    assert_eq!(warm.evaluated_count(), 0);
    assert_eq!(warm.parsed_formula_count(), 0);
    assert!(warm.changed_cells().is_empty());
}

#[test]
fn clean_runtime_issues_survive_incremental_reuse_and_match_full() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("A1"),
                    formula("IFERROR(\"ab\"&\"cd\",\"hidden\")"),
                ),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1")),
            ]),
        )
        .expect("resource-limit formulas");
    let limits = CalculationLimits::default()
        .with_max_text_bytes(3)
        .expect("nonzero text limit");
    let options = CalculationOptions::default().with_limits(limits);
    session
        .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
        .expect("initial calculation");
    let before = session
        .calculation()
        .expect("installed initial calculation")
        .cell(CalculationCellId::new(sheet_id, address("A1")))
        .cloned()
        .expect("A1 calculation");
    let CalculationCellResult::Unavailable(issue) = &before else {
        panic!("A1 must retain its resource-limit issue");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_text_bytes"));

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("C1"),
                number(1.0),
            )]),
        )
        .expect("unrelated literal edit");
    let delta = session
        .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
        .expect("warm incremental calculation");

    assert_eq!(delta.evaluated_count(), 0);
    assert!(delta.changed_cells().is_empty());
    assert_eq!(
        session
            .calculation()
            .expect("installed warm calculation")
            .cell(CalculationCellId::new(sheet_id, address("A1"))),
        Some(&before)
    );
    let oracle = calculate_workbook(session.workbook(), options);
    assert_calculations_equal(
        session.calculation().expect("installed warm calculation"),
        &oracle,
    );
}

#[test]
fn topology_changes_fall_back_to_full_and_forced_incremental_fails_closed() {
    let mut session = chain_session();
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                SheetId::new(1).expect("constant sheet ID"),
                address("B1"),
                formula("A1+10"),
            )]),
        )
        .expect("formula edit");

    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("forced incremental must reject topology change");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);

    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto falls back to full");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::TopologyChanged);
    assert_calculations_equal(
        session.calculation().expect("installed result"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn stale_and_cancelled_jobs_never_replace_current_results() {
    let mut session = chain_session();
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([WorkbookChange::set_cell_value(
                SheetId::new(1).expect("constant sheet ID"),
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("first edit");
    let prepared = session
        .prepare_recalculation(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("prepared calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                SheetId::new(1).expect("constant sheet ID"),
                address("A1"),
                number(3.0),
            )]),
        )
        .expect("newer edit");
    let completed = prepared.run().expect("older calculation can complete");
    let error = session
        .install(completed)
        .expect_err("stale result is not installed");
    assert_eq!(error.code(), SessionErrorCode::StaleResult);

    let token = CancellationToken::new();
    let prepared = session
        .prepare_recalculation(
            RecalculationMode::Full,
            CalculationOptions::default(),
            token.clone(),
        )
        .expect("prepared cancellable calculation");
    token.cancel();
    let error = prepared.run().expect_err("cancelled work stops");
    assert_eq!(error.code(), SessionErrorCode::Cancelled);
}

#[test]
fn cursor_history_is_stable_and_revision_conflicts_are_explicit() {
    let mut session = chain_session();
    let first = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("first calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                SheetId::new(1).expect("constant sheet ID"),
                address("A1"),
                number(5.0),
            )]),
        )
        .expect("edit");
    let second = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("second calculation");

    let first_page = session.changes_since(0, 1).expect("first page");
    assert_eq!(first_page.deltas().len(), 1);
    assert_eq!(first_page.deltas()[0].cursor(), first.cursor());
    assert_eq!(first_page.next_cursor(), Some(first.cursor()));
    let second_page = session
        .changes_since(first.cursor(), 1)
        .expect("second page");
    assert_eq!(second_page.deltas()[0].cursor(), second.cursor());
    assert_eq!(second_page.next_cursor(), None);

    let error = session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::clear_cell(
                SheetId::new(1).expect("constant sheet ID"),
                address("A1"),
            )]),
        )
        .expect_err("stale writer revision rejected");
    match error {
        ApplyChangesError::Session(error) => {
            assert_eq!(error.code(), SessionErrorCode::RevisionMismatch);
        }
        ApplyChangesError::Validation(error) => {
            panic!("unexpected validation error: {error}");
        }
    }
}

#[test]
fn dynamic_spill_edits_use_conservative_full_fallback() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("B1"),
                    formula("TAKE({1,2,3},,A1)"),
                    None,
                )
                .expect("valid dynamic formula"),
            ]),
        )
        .expect("initial batch");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial spill calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(3.0),
            )]),
        )
        .expect("spill input edit");

    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("unstable spill topology rejects forced incremental");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto full spill calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::DynamicTopology);
    assert_calculations_equal(
        session.calculation().expect("installed expanded spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(1.0),
            )]),
        )
        .expect("spill shrink input edit");
    let shrink = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto full spill shrink");
    assert_eq!(
        shrink
            .removed_materialized_cells()
            .iter()
            .map(|cell| cell.address().to_string())
            .collect::<Vec<_>>(),
        vec!["C1", "D1"]
    );
    assert_calculations_equal(
        session.calculation().expect("installed shrunk spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn dynamic_references_hidden_in_defined_names_reject_incremental_recalculation() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("SUM(DynamicCell)"),
                ),
                WorkbookChange::set_defined_name(defined_name("DynamicCell", "OFFSET(A1,0,0)")),
            ]),
        )
        .expect("dynamic defined name setup");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("dynamic input edit");

    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("defined-name OFFSET must reject forced incremental calculation");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);

    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto full calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::DynamicTopology);
    assert_calculations_equal(
        session.calculation().expect("installed dynamic result"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn map_parameters_shadow_dynamic_defined_names_during_incremental_analysis() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("SUMPRODUCT(MAP(A1,LAMBDA(item,item+1)))"),
                ),
                WorkbookChange::set_defined_name(defined_name("item", "OFFSET(Z1,0,0)")),
            ]),
        )
        .expect("shadowed defined name setup");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("MAP input edit");

    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("local lambda parameter must not resolve the dynamic workbook name");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_calculations_equal(
        session.calculation().expect("installed incremental result"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn cross_sheet_dependencies_propagate_and_format_only_edits_do_not_recalculate() {
    let mut session = WorkbookCalculationSession::create();
    let first = SheetId::new(1).expect("constant sheet ID");
    let second = SheetId::new(2).expect("predictable second sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::add_sheet(SheetName::new("Inputs").expect("valid sheet name")),
                WorkbookChange::set_cell_value(first, address("A1"), number(2.0)),
                WorkbookChange::set_cell_formula(second, address("B1"), formula("Sheet1!A1*2")),
                WorkbookChange::set_cell_formula(second, address("C1"), formula("B1+1")),
                WorkbookChange::set_cell_formula(second, address("Z1"), formula("1+1")),
            ]),
        )
        .expect("cross-sheet workbook batch");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                first,
                address("A1"),
                number(5.0),
            )]),
        )
        .expect("cross-sheet input edit");
    let incremental = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("cross-sheet incremental calculation");
    assert_eq!(incremental.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(incremental.evaluated_count(), 2);
    assert_calculations_equal(
        session.calculation().expect("installed cross-sheet result"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_number_format(
                first,
                address("A1"),
                NumberFormat::custom(164, "0.00", NumberFormatKind::Number)
                    .expect("valid number format"),
            )]),
        )
        .expect("format-only edit");
    let format_only = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("format-only warm calculation");
    assert_eq!(
        format_only.reason(),
        CalculationDecisionReason::NoDirtyFormulas
    );
    assert_eq!(format_only.evaluated_count(), 0);
    assert!(format_only.changed_cells().is_empty());
}

#[test]
fn generated_edit_sequence_matches_a_fresh_full_oracle_after_every_revision() {
    const ROWS: u32 = 50;
    const EDITS: u32 = 100;
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut initial = Vec::new();
    for row in 1..=ROWS {
        initial.push(WorkbookChange::set_cell_value(
            sheet_id,
            CellAddress::from_indices(row, 1).expect("valid generated input"),
            number(f64::from(row)),
        ));
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 2).expect("valid generated formula"),
            formula(&format!("A{row}*2")),
        ));
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 3).expect("valid generated formula"),
            formula(&format!("B{row}+1")),
        ));
    }
    initial.push(WorkbookChange::set_cell_formula(
        sheet_id,
        address("D1"),
        formula("SUM(C1:C50)"),
    ));
    session
        .apply_changes(0, EditBatch::new(initial))
        .expect("generated workbook batch");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("generated initial calculation");

    for step in 1..=EDITS {
        let row = ((step * 37) % ROWS) + 1;
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet_id,
                    CellAddress::from_indices(row, 1).expect("valid generated edit"),
                    number(10_000.0 + f64::from(step)),
                )]),
            )
            .expect("generated input edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("generated incremental calculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(delta.evaluated_count(), 3);
        assert_calculations_equal(
            session.calculation().expect("installed generated result"),
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
}

#[test]
fn evaluated_and_changed_result_sets_are_reported_independently() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1*0")),
            ]),
        )
        .expect("constant-result workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(99.0),
            )]),
        )
        .expect("input edit");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");
    assert_eq!(delta.dirty_count(), 1);
    assert_eq!(delta.evaluated_count(), 1);
    assert!(delta.changed_cells().is_empty());
}

#[test]
fn defined_name_and_cycle_topology_rebuilds_match_the_full_oracle() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
                WorkbookChange::set_defined_name(defined_name("Factor", "A1*2")),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("Factor+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
            ]),
        )
        .expect("named-formula workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    for changes in [
        vec![WorkbookChange::set_defined_name(defined_name(
            "Factor", "A1*3",
        ))],
        vec![
            WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("C1+1")),
            WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
        ],
        vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("B1"),
            formula("Factor+1"),
        )],
    ] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new(changes),
            )
            .expect("topology edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("topology full calculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Full);
        assert_eq!(delta.reason(), CalculationDecisionReason::TopologyChanged);
        assert_calculations_equal(
            session.calculation().expect("installed topology result"),
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
}

#[test]
fn evaluation_budget_is_rejected_before_a_full_pass_runs() {
    let limits = SessionLimits::new(100, 1, 100, 10, 10).expect("valid session limits");
    let mut session =
        WorkbookCalculationSession::with_limits(cellrune::WorkbookDraft::new(), limits);
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(sheet_id, address("A1"), formula("1")),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("2")),
            ]),
        )
        .expect("formula batch");
    let prepared = session
        .prepare_recalculation(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full calculation preparation");
    let error = prepared
        .run()
        .expect_err("full pass exceeds evaluation budget before execution");
    assert_eq!(error.code(), SessionErrorCode::EvaluationLimitExceeded);
}

#[test]
fn batch_change_variants_commit_observable_state_and_preserve_unrelated_metadata() {
    let mut session = WorkbookCalculationSession::create();
    let first = SheetId::new(1).expect("constant sheet ID");
    let second = SheetId::new(2).expect("predictable second sheet ID");
    let initial = session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::add_sheet(SheetName::new("Second").expect("valid sheet name")),
                WorkbookChange::set_cell_value(first, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(first, address("B1"), formula("A1+1")),
                WorkbookChange::set_defined_name(defined_name("Alpha", "A1")),
                WorkbookChange::set_defined_name(defined_name("Beta", "A1+1")),
                WorkbookChange::set_defined_name(
                    DefinedName::new(
                        "Alpha",
                        DefinedNameScope::Sheet(second),
                        formula("Sheet1!A1"),
                        false,
                    )
                    .expect("valid sheet-local name"),
                ),
                WorkbookChange::set_sheet_visibility(second, SheetVisibility::Hidden),
                WorkbookChange::set_date_system(DateSystem::Excel1904),
                WorkbookChange::set_calculation_hints(CalculationHints::new(
                    Some(CalculationMode::Manual),
                    Some(42),
                    Some(true),
                    Some(false),
                )),
            ]),
        )
        .expect("complete metadata batch");
    assert_eq!(initial.created_sheet_ids(), &[second]);
    assert!(initial.topology_changed());
    assert!(initial.calculation_metadata_changed());
    assert_eq!(session.workbook().date_system(), DateSystem::Excel1904);
    assert_eq!(
        session.workbook().calculation_hints().mode(),
        Some(CalculationMode::Manual)
    );
    assert_eq!(
        session
            .workbook()
            .sheet_by_id(second)
            .expect("second sheet")
            .visibility(),
        SheetVisibility::Hidden
    );

    let value_edit = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                first,
                address("A1"),
                number(5.0),
            )]),
        )
        .expect("existing literal edit");
    assert_eq!(value_edit.calculation_changed_cells().len(), 1);
    assert_eq!(literal_number(&session, first, "A1"), 5.0);

    let formula_to_literal = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                first,
                address("B1"),
                number(9.0),
            )]),
        )
        .expect("formula replacement");
    assert!(formula_to_literal.topology_changed());
    assert_eq!(literal_number(&session, first, "B1"), 9.0);
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                first,
                address("B1"),
                formula("A1+1"),
            )]),
        )
        .expect("formula restoration");
    let clear_formula = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::clear_cell(first, address("B1"))]),
        )
        .expect("formula clear");
    assert!(clear_formula.topology_changed());
    assert!(
        session
            .workbook()
            .sheet_by_id(first)
            .and_then(|sheet| sheet.cell(address("B1")))
            .is_none()
    );

    let format =
        NumberFormat::custom(164, "0.00", NumberFormatKind::Number).expect("valid custom format");
    let format_edit = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_number_format(
                first,
                address("A1"),
                format.clone(),
            )]),
        )
        .expect("format edit");
    assert_eq!(format_edit.changed_cells().len(), 1);
    assert!(format_edit.calculation_changed_cells().is_empty());
    assert_eq!(
        session
            .workbook()
            .sheet_by_id(first)
            .and_then(|sheet| sheet.cell(address("A1")))
            .expect("formatted cell")
            .number_format(),
        &format
    );
    let format_noop = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_number_format(
                first,
                address("A1"),
                format,
            )]),
        )
        .expect("idempotent format edit");
    assert!(format_noop.changed_cells().is_empty());
    assert_eq!(format_noop.base_revision(), format_noop.result_revision());
    assert!(!format_noop.topology_changed());

    let replace_name = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([
                WorkbookChange::set_defined_name(defined_name("Alpha", "A1*10")),
                WorkbookChange::remove_defined_name(DefinedNameScope::Workbook, "Alpha"),
            ]),
        )
        .expect("replace and remove workbook name");
    assert!(replace_name.topology_changed());
    assert!(!has_defined_name(
        &session,
        DefinedNameScope::Workbook,
        "Alpha"
    ));
    assert!(has_defined_name(
        &session,
        DefinedNameScope::Workbook,
        "Beta"
    ));
    assert!(has_defined_name(
        &session,
        DefinedNameScope::Sheet(second),
        "Alpha"
    ));

    let third = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::add_sheet(
                SheetName::new("Third").expect("valid third sheet name"),
            )]),
        )
        .expect("third sheet")
        .created_sheet_ids()[0];
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_sheet_visibility(
                third,
                SheetVisibility::Hidden,
            )]),
        )
        .expect("one of two visible sheets may be hidden");
    let before = session.workbook().semantic_revision();
    let error = session
        .apply_changes(
            before,
            EditBatch::new([WorkbookChange::set_sheet_visibility(
                first,
                SheetVisibility::Hidden,
            )]),
        )
        .expect_err("last visible sheet cannot be hidden");
    assert!(matches!(error, ApplyChangesError::Validation(_)));
    assert_eq!(session.workbook().semantic_revision(), before);

    let unknown_scope = DefinedName::new(
        "UnknownScope",
        DefinedNameScope::Sheet(SheetId::new(999).expect("constant absent sheet ID")),
        formula("A1"),
        false,
    )
    .expect("name object validates independently");
    let error = session
        .apply_changes(
            before,
            EditBatch::new([WorkbookChange::set_defined_name(unknown_scope)]),
        )
        .expect_err("unknown sheet scope fails atomically");
    assert!(matches!(error, ApplyChangesError::Validation(_)));
    assert_eq!(session.workbook().semantic_revision(), before);
}

#[test]
fn session_batch_history_and_delta_limits_accept_exact_boundaries_and_reject_excess() {
    for invalid in [
        (0, 1, 1, 1, 1),
        (1, 0, 1, 1, 1),
        (1, 1, 0, 1, 1),
        (1, 1, 1, 0, 1),
        (1, 1, 1, 1, 0),
    ] {
        let error = SessionLimits::new(invalid.0, invalid.1, invalid.2, invalid.3, invalid.4)
            .expect_err("every zero session limit must fail");
        assert_eq!(error.code(), SessionErrorCode::InvalidLimits);
    }

    let batch_limits = SessionLimits::new(2, 100, 100, 2, 10).expect("valid batch limits");
    let mut batch_session =
        WorkbookCalculationSession::with_limits(cellrune::WorkbookDraft::new(), batch_limits);
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    batch_session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
            ]),
        )
        .expect("exact batch limit is accepted");
    let error = batch_session
        .apply_changes(
            batch_session.workbook().semantic_revision(),
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(3.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A4"), number(4.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A5"), number(5.0)),
            ]),
        )
        .expect_err("batch above limit is rejected");
    assert!(matches!(
        error,
        ApplyChangesError::Session(error)
            if error.code() == SessionErrorCode::BatchLimitExceeded
    ));

    let delta_limits = SessionLimits::new(10, 100, 2, 1, 10).expect("valid delta limits");
    let mut exact =
        WorkbookCalculationSession::with_limits(cellrune::WorkbookDraft::new(), delta_limits);
    exact
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(sheet_id, address("A1"), formula("1")),
                WorkbookChange::set_cell_formula(sheet_id, address("A2"), formula("2")),
            ]),
        )
        .expect("two-formula workbook");
    let first = exact
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("delta exactly at cell limit");
    assert_eq!(first.changed_cells().len(), 2);
    exact
        .apply_changes(
            exact.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_formula(
                sheet_id,
                address("A1"),
                formula("10"),
            )]),
        )
        .expect("second revision");
    let second = exact
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("second retained delta");
    let retained = exact.changes_since(0, 10).expect("retained history");
    assert_eq!(retained.deltas().len(), 1);
    assert_eq!(retained.deltas()[0].cursor(), second.cursor());

    let mut excess =
        WorkbookCalculationSession::with_limits(cellrune::WorkbookDraft::new(), delta_limits);
    excess
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(sheet_id, address("A1"), formula("1")),
                WorkbookChange::set_cell_formula(sheet_id, address("A2"), formula("2")),
                WorkbookChange::set_cell_formula(sheet_id, address("A3"), formula("3")),
            ]),
        )
        .expect("three-formula workbook");
    let error = excess
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("delta above limit is rejected");
    assert_eq!(error.code(), SessionErrorCode::DeltaLimitExceeded);
}

fn chain_session() -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("Z1"), formula("1+1")),
            ]),
        )
        .expect("valid chain batch");
    session
}

fn assert_calculations_equal(
    left: &cellrune::CalculationSnapshot,
    right: &cellrune::CalculationSnapshot,
) {
    assert_eq!(
        left.cells().collect::<Vec<_>>(),
        right.cells().collect::<Vec<_>>()
    );
    assert_eq!(
        left.materialized_cells().collect::<Vec<_>>(),
        right.materialized_cells().collect::<Vec<_>>()
    );
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid test address")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("valid test formula")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite test number"))
}

fn literal_number(
    session: &WorkbookCalculationSession,
    sheet_id: SheetId,
    address_text: &str,
) -> f64 {
    let content = session
        .workbook()
        .sheet_by_id(sheet_id)
        .and_then(|sheet| sheet.cell(address(address_text)))
        .expect("test cell")
        .content();
    let CellContent::Literal(CellValue::Number(value)) = content else {
        panic!("test cell must contain a literal number");
    };
    value.get()
}

fn has_defined_name(
    session: &WorkbookCalculationSession,
    scope: DefinedNameScope,
    name: &str,
) -> bool {
    session
        .workbook()
        .defined_names()
        .iter()
        .any(|defined_name| {
            defined_name.scope() == scope && defined_name.name().eq_ignore_ascii_case(name)
        })
}

fn defined_name(name: &str, formula_text: &str) -> DefinedName {
    DefinedName::new(
        name,
        DefinedNameScope::Workbook,
        formula(formula_text),
        false,
    )
    .expect("valid test defined name")
}
