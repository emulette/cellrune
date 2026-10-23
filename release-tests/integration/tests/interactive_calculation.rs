use cellrune::{
    ApplyChangesError, ArithmeticSemantics, CalculationCellId, CalculationCellResult,
    CalculationDecisionReason, CalculationExecutionMode, CalculationHints, CalculationIssueCode,
    CalculationLimits, CalculationMode, CalculationOptions, CalculationTarget, CancellationToken,
    CellAddress, CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope,
    EditBatch, FiniteNumber, FormulaText, NumberFormat, NumberFormatKind, RecalculationMode,
    SessionErrorCode, SessionLimits, SheetId, SheetName, SheetVisibility, TargetCalculationLimits,
    WorkbookCalculationSession, WorkbookChange, calculate_targets, calculate_workbook,
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
fn whole_column_references_include_array_result_areas() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut spilled = WorkbookCalculationSession::create();
    spilled
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("J1"),
                    formula("SEQUENCE(3)"),
                    None,
                )
                .expect("undeclared spill"),
                WorkbookChange::set_cell_formula(sheet_id, address("A1"), formula("SUM(J:J)")),
            ]),
        )
        .expect("spill workbook");
    spilled
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("spill calculation");
    assert_eq!(
        spilled
            .calculation()
            .expect("installed spill calculation")
            .cell(CalculationCellId::new(sheet_id, address("A1"))),
        Some(&CalculationCellResult::Value(number(6.0)))
    );

    let mut declared = WorkbookCalculationSession::create();
    declared
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("L1"),
                    formula("SEQUENCE(3)"),
                    Some(CellRange::new(address("L1"), address("L3")).expect("declared range")),
                )
                .expect("declared spill"),
                WorkbookChange::set_cell_value(sheet_id, address("L10"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("SUM(L:L)")),
            ]),
        )
        .expect("declared spill workbook");
    declared
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("declared spill calculation");
    declared
        .apply_changes(
            declared.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::clear_cell(sheet_id, address("L10"))]),
        )
        .expect("clear the stored cell below the declared spill");
    let delta = declared
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental declared spill recalculation");

    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        declared
            .calculation()
            .expect("installed declared spill calculation")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(6.0)))
    );
}

#[test]
fn explicit_ranges_read_array_results_below_the_last_stored_cell() {
    // J1 and L1 are the only stored cells in their columns; their spills fill the rows below, and
    // explicit ranges far past them read those results and blanks, in full and incrementally.
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(4.0)),
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("J1"),
                    formula("SEQUENCE(C1)"),
                    None,
                )
                .expect("undeclared spill"),
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("L1"),
                    formula("SEQUENCE(3)*C1"),
                    Some(CellRange::new(address("L1"), address("L3")).expect("declared range")),
                )
                .expect("declared spill"),
                WorkbookChange::set_cell_formula(sheet_id, address("A1"), formula("SUM(J1:J5000)")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("A2"),
                    formula("SUMPRODUCT(L1:L5000*2)"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("A3"),
                    formula("COUNTA(L1:L5000)"),
                ),
            ]),
        )
        .expect("spill workbook");
    for (step, value) in [(0, 4.0), (1, 5.0)] {
        if step > 0 {
            session
                .apply_changes(
                    session.workbook().semantic_revision(),
                    EditBatch::new([WorkbookChange::set_cell_value(
                        sheet_id,
                        address("C1"),
                        number(value),
                    )]),
                )
                .expect("input edit");
        }
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("spill calculation");
        let calculation = session.calculation().expect("installed calculation");
        let triangle = value * (value + 1.0) / 2.0;
        for (cell, expected) in [("A1", triangle), ("A2", 12.0 * value), ("A3", 3.0)] {
            assert_eq!(
                calculation.cell(CalculationCellId::new(sheet_id, address(cell))),
                Some(&CalculationCellResult::Value(number(expected))),
                "{cell} at step {step}",
            );
        }
        assert_calculations_equal(
            calculation,
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
}

#[test]
fn declared_spill_followers_read_a_failed_anchor_the_same_way_incrementally() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("A1"),
                    formula("SEQUENCE(2)"),
                    Some(CellRange::new(address("A1"), address("A3")).expect("declared range")),
                )
                .expect("declared spill that does not fit its result"),
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A2+C1")),
            ]),
        )
        .expect("failed declared spill workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full calculation");
    let spill_error = Some(&CalculationCellResult::Value(CellValue::Error(
        cellrune::ExcelError::Spill,
    )));
    assert_eq!(
        session
            .calculation()
            .expect("installed full calculation")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        spill_error
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("C1"),
                number(2.0),
            )]),
        )
        .expect("unrelated input edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental recalculation");

    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        session
            .calculation()
            .expect("installed incremental calculation")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        spill_error
    );
}

#[test]
fn a_range_ending_in_index_follows_its_end_after_edits() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut changes = (2..=20)
        .map(|row| {
            WorkbookChange::set_cell_value(sheet_id, address(&format!("B{row}")), number(1.0))
        })
        .collect::<Vec<_>>();
    changes.push(WorkbookChange::set_cell_value(
        sheet_id,
        address("F1"),
        number(3.0),
    ));
    changes.push(WorkbookChange::set_cell_formula(
        sheet_id,
        address("H1"),
        formula("SUM(B2:INDEX(D2:D20,F1))"),
    ));
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("INDEX range workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    for (address_text, value, expected) in [("F1", 10.0, 10.0), ("B8", 100.0, 109.0)] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet_id,
                    address(address_text),
                    number(value),
                )]),
            )
            .expect("edit");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("recalculation");
        assert_eq!(
            session
                .calculation()
                .expect("installed calculation")
                .cell(CalculationCellId::new(sheet_id, address("H1"))),
            Some(&CalculationCellResult::Value(number(expected))),
            "after editing {address_text}"
        );
    }

    let mut computed_end = WorkbookCalculationSession::create();
    computed_end
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("B2"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B3"), formula("1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("F1"), formula("1+1")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("H1"),
                    formula("SUM(B2:INDEX(D2:D20,F1))"),
                ),
            ]),
        )
        .expect("computed INDEX end workbook");
    computed_end
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full calculation");
    assert_eq!(
        computed_end
            .calculation()
            .expect("installed calculation")
            .cell(CalculationCellId::new(sheet_id, address("H1"))),
        Some(&CalculationCellResult::Value(number(3.0)))
    );
}

#[test]
fn a_range_ending_in_index_with_a_formula_in_its_reach_recalculates_in_full() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut changes = (2..=20)
        .map(|row| {
            WorkbookChange::set_cell_value(sheet_id, address(&format!("B{row}")), number(1.0))
        })
        .collect::<Vec<_>>();
    changes.extend([
        WorkbookChange::set_cell_value(sheet_id, address("E1"), number(50.0)),
        // B8 lies past the range's end until F1 moves it, so calculation order does not place
        // B8 before H1.
        WorkbookChange::set_cell_formula(sheet_id, address("B8"), formula("E1*2")),
        WorkbookChange::set_cell_value(sheet_id, address("F1"), number(3.0)),
        WorkbookChange::set_cell_formula(
            sheet_id,
            address("H1"),
            formula("SUM(B2:INDEX(D2:D20,F1))"),
        ),
    ]);
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("INDEX range workbook");
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
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("F1"), number(10.0)),
                WorkbookChange::set_cell_value(sheet_id, address("E1"), number(100.0)),
            ]),
        )
        .expect("edit");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("recalculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(
        session
            .calculation()
            .expect("installed calculation")
            .cell(CalculationCellId::new(sheet_id, address("H1"))),
        Some(&CalculationCellResult::Value(number(209.0)))
    );
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
fn immediate_lambda_invocation_recalculates_incrementally_with_cell_dependencies() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("LAMBDA(x,x+A1)(1)"),
                ),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("10+1")),
            ]),
        )
        .expect("initial lambda formula");
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    assert_eq!(
        session
            .calculation()
            .expect("installed calculation")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(3.0)))
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(4.0),
            )]),
        )
        .expect("dependency edit");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(delta.reason(), CalculationDecisionReason::DirtySubset);
    assert_eq!(
        session
            .calculation()
            .expect("installed delta")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(5.0)))
    );
}

#[test]
fn grouped_spills_match_full_calculation_after_incremental_value_edits() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let grouped_range = CellRange::new(address("E1"), address("F4")).expect("grouped spill range");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(
                    sheet_id,
                    address("A1"),
                    CellValue::Text("A".to_owned()),
                ),
                WorkbookChange::set_cell_value(
                    sheet_id,
                    address("A2"),
                    CellValue::Text("B".to_owned()),
                ),
                WorkbookChange::set_cell_value(
                    sheet_id,
                    address("A3"),
                    CellValue::Text("A".to_owned()),
                ),
                WorkbookChange::set_cell_value(
                    sheet_id,
                    address("A4"),
                    CellValue::Text("C".to_owned()),
                ),
                WorkbookChange::set_cell_value(sheet_id, address("B1"), number(10.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B2"), number(20.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B3"), number(30.0)),
                WorkbookChange::set_cell_value(sheet_id, address("B4"), number(40.0)),
                WorkbookChange::set_cell_dynamic_formula(
                    sheet_id,
                    address("E1"),
                    formula("GROUPBY(A1:A4,B1:B4,SUM)"),
                    Some(grouped_range),
                )
                .expect("valid grouped spill change"),
            ]),
        )
        .expect("grouped spill workbook");
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial grouped calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("B2"),
                number(25.0),
            )]),
        )
        .expect("grouped value edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental grouped calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        delta.reason(),
        CalculationDecisionReason::IncrementalRequested
    );

    let installed = session
        .calculation()
        .expect("installed grouped calculation");
    let full = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_calculations_equal(installed, &full);
    assert_eq!(
        installed
            .materialized_cell(CalculationCellId::new(sheet_id, address("F2")))
            .map(|cell| cell.result()),
        Some(&CalculationCellResult::Value(number(25.0))),
    );
}

#[test]
fn named_lambda_body_dependencies_schedule_and_recalculate_incrementally() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
                WorkbookChange::set_defined_name(defined_name(
                    "Reader",
                    "LAMBDA(offset,C1+offset)",
                )),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("Reader(0)")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("A1*2")),
                WorkbookChange::set_cell_formula(sheet_id, address("D1"), formula("10+1")),
            ]),
        )
        .expect("named lambda dependency workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(
        session
            .calculation()
            .expect("installed calculation")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(4.0)))
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(4.0),
            )]),
        )
        .expect("dependency edit");
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
    assert_eq!(
        session
            .calculation()
            .expect("installed delta")
            .cell(CalculationCellId::new(sheet_id, address("B1"))),
        Some(&CalculationCellResult::Value(number(8.0)))
    );
    assert_calculations_equal(
        session.calculation().expect("installed delta"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn builtin_named_lambda_shadows_preserve_incremental_dependencies() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
                WorkbookChange::set_defined_name(defined_name("MAP", "LAMBDA(value,callback,C1)")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("MAP(0,LAMBDA(x,x))"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B2"),
                    formula("MAP(0,LAMBDA(x,x))+0"),
                ),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("A1*2")),
            ]),
        )
        .expect("builtin-shadowing named lambda dependency workbook");
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
                sheet_id,
                address("A1"),
                number(4.0),
            )]),
        )
        .expect("dependency edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        delta.reason(),
        CalculationDecisionReason::IncrementalRequested
    );
    assert_eq!(delta.dirty_count(), 3);
    assert_eq!(delta.evaluated_count(), 3);
    for address_text in ["B1", "B2"] {
        assert_eq!(
            session
                .calculation()
                .expect("installed delta")
                .cell(CalculationCellId::new(sheet_id, address(address_text))),
            Some(&CalculationCellResult::Value(number(8.0)))
        );
    }
    assert_calculations_equal(
        session.calculation().expect("installed delta"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
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
fn incremental_results_report_capability_issues_as_a_full_calculation_does() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut initial = vec![
        WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
        WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+NB")),
    ];
    for row in 1..=4 {
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 26).expect("valid filler address"),
            formula(&format!("{row}*2")),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(initial))
        .expect("workbook with an undefined name");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    // A value edit dirties the formula with the undefined name; formula edits add an unknown
    // function, a parse failure, and a cycle through an undefined name.
    let edits = [
        vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("A1"),
            number(2.0),
        )],
        vec![
            WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("FOOBAR(A1)")),
            WorkbookChange::set_cell_formula(sheet_id, address("D1"), formula("SUM((")),
            WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("SUM(E1:E3)+NB")),
        ],
    ];
    for changes in edits {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new(changes),
            )
            .expect("edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("automatic recalculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_calculations_equal(
            session.calculation().expect("installed calculation"),
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
    let issue = |a1: &str| match session
        .calculation()
        .expect("installed calculation")
        .cell(CalculationCellId::new(sheet_id, address(a1)))
    {
        Some(CalculationCellResult::Unavailable(issue)) => issue.code(),
        other => panic!("{a1} must be unavailable, found {other:?}"),
    };
    assert_eq!(issue("B1"), CalculationIssueCode::UnsupportedName);
    assert_eq!(issue("C1"), CalculationIssueCode::UnsupportedFunction);
    assert_eq!(issue("D1"), CalculationIssueCode::ParseError);
    assert_eq!(issue("E1"), CalculationIssueCode::UnsupportedName);
}

#[test]
fn incremental_readers_of_a_formula_with_a_capability_issue_read_its_value() {
    // B1 and D1 are unavailable for their capability issues, but a full calculation lets C1 and
    // E1 read the values they evaluate to.
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut initial = vec![
        WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
        WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("NB*3")),
        WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+A1")),
        WorkbookChange::set_cell_formula(sheet_id, address("D1"), formula("IF(TRUE,1,FOOBAR())")),
        WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("D1+A1")),
    ];
    for row in 1..=4 {
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 26).expect("valid filler address"),
            formula(&format!("{row}*2")),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(initial))
        .expect("workbook with capability issues");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    let edits = [
        WorkbookChange::set_cell_value(sheet_id, address("A1"), number(2.0)),
        WorkbookChange::set_cell_formula(sheet_id, address("F1"), formula("B1+D1")),
    ];
    for change in edits {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([change]),
            )
            .expect("edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("automatic recalculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_calculations_equal(
            session.calculation().expect("installed calculation"),
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
    let result = |a1: &str| {
        session
            .calculation()
            .expect("installed calculation")
            .cell(CalculationCellId::new(sheet_id, address(a1)))
            .cloned()
    };
    assert_eq!(
        result("C1"),
        Some(CalculationCellResult::Value(CellValue::Error(
            cellrune::ExcelError::Name
        )))
    );
    assert_eq!(
        result("E1"),
        Some(CalculationCellResult::Value(number(3.0)))
    );
}

#[test]
fn formula_metadata_dependencies_dirty_without_scheduling_source_values() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("ISFORMULA(A1)")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("C1"),
                    formula("FORMULATEXT(A1)"),
                ),
            ]),
        )
        .expect("metadata formulas");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let before_b1 = session
        .calculation()
        .expect("installed initial calculation")
        .cell(CalculationCellId::new(sheet_id, address("B1")))
        .cloned();
    let before_c1 = session
        .calculation()
        .expect("installed initial calculation")
        .cell(CalculationCellId::new(sheet_id, address("C1")))
        .cloned();

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("source content edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental metadata recalculation");

    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(delta.dirty_count(), 2);
    assert_eq!(delta.evaluated_count(), 2);
    assert_eq!(
        session
            .calculation()
            .expect("installed metadata delta")
            .cell(CalculationCellId::new(sheet_id, address("B1")))
            .cloned(),
        before_b1,
    );
    assert_eq!(
        session
            .calculation()
            .expect("installed metadata delta")
            .cell(CalculationCellId::new(sheet_id, address("C1")))
            .cloned(),
        before_c1,
    );
    assert_calculations_equal(
        session.calculation().expect("installed metadata delta"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn formula_edits_recalculate_incrementally_and_forced_incremental_fails_closed() {
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

    // Auto updates the compiled formulas for the edited cell and evaluates it and its dependents.
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto applies the formula edit");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(delta.reason(), CalculationDecisionReason::DirtySubset);
    assert_eq!(delta.parsed_formula_count(), 1);
    assert_calculations_equal(
        session.calculation().expect("installed result"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn formula_edits_evaluate_only_the_edited_formulas_and_their_dependents() {
    // B holds formulas over A, C running totals over B, and G1 sums F, which starts as values.
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut initial = Vec::new();
    for row in 1..=20 {
        initial.push(WorkbookChange::set_cell_value(
            sheet_id,
            CellAddress::from_indices(row, 1).expect("valid input"),
            number(f64::from(row)),
        ));
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 2).expect("valid formula"),
            formula(&format!("A{row}*2")),
        ));
        initial.push(WorkbookChange::set_cell_formula(
            sheet_id,
            CellAddress::from_indices(row, 3).expect("valid total"),
            formula(&format!("SUM($B$1:B{row})")),
        ));
        initial.push(WorkbookChange::set_cell_value(
            sheet_id,
            CellAddress::from_indices(row, 6).expect("valid value"),
            number(1.0),
        ));
    }
    initial.push(WorkbookChange::set_cell_formula(
        sheet_id,
        address("G1"),
        formula("SUM(F1:F20)"),
    ));
    session
        .apply_changes(0, EditBatch::new(initial))
        .expect("generated workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");

    let b7 = CalculationCellId::new(sheet_id, address("B7"));
    for (change, evaluated, removed) in [
        // A replaced formula, and the 16 running totals from its row.
        (
            WorkbookChange::set_cell_formula(sheet_id, address("B5"), formula("A5*3")),
            17,
            None,
        ),
        // A new formula nothing reads.
        (
            WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("SUM(B1:B20)")),
            1,
            None,
        ),
        // A formula inside a range G1 reads, which now waits for it.
        (
            WorkbookChange::set_cell_formula(sheet_id, address("F5"), formula("C20/10")),
            2,
            None,
        ),
        // A formula replaced by a value: its result goes, and the totals from its row, E1, and
        // through C20 F5 and G1 recalculate.
        (
            WorkbookChange::set_cell_value(sheet_id, address("B7"), number(70.0)),
            17,
            Some(b7),
        ),
        // A cleared formula inside G1's range.
        (WorkbookChange::clear_cell(sheet_id, address("F5")), 1, None),
    ] {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([change]),
            )
            .expect("formula edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("formula edit calculation");
        assert_eq!(
            (delta.mode(), delta.reason()),
            (
                CalculationExecutionMode::Incremental,
                CalculationDecisionReason::DirtySubset,
            )
        );
        assert_eq!(delta.evaluated_count(), evaluated);
        let calculation = session.calculation().expect("installed result");
        if let Some(removed) = removed {
            assert!(delta.removed_materialized_cells().contains(&removed));
            assert_eq!(calculation.cell(removed), None);
        }
        assert_calculations_equal(
            calculation,
            &calculate_workbook(session.workbook(), CalculationOptions::default()),
        );
    }
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
    token.cancel();
    let error = session
        .prepare_recalculation(
            RecalculationMode::Full,
            CalculationOptions::default(),
            token,
        )
        .expect_err("pre-cancelled preparation stops before cloning state");
    assert_eq!(error.code(), SessionErrorCode::Cancelled);

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
fn same_revision_older_job_cannot_replace_a_newer_calculation() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::set_cell_formula(
                sheet_id,
                address("A1"),
                formula("TODAY()"),
            )]),
        )
        .expect("TODAY workbook");
    let first_options = CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(1.0).expect("finite first TODAY serial"));
    let second_options = CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(2.0).expect("finite second TODAY serial"));
    session
        .recalculate(
            RecalculationMode::Auto,
            first_options,
            CancellationToken::new(),
        )
        .expect("initial calculation");

    let older = session
        .prepare_recalculation(
            RecalculationMode::Auto,
            first_options,
            CancellationToken::new(),
        )
        .expect("older no-dirty job")
        .run()
        .expect("older job completes");
    let newer = session
        .prepare_recalculation(
            RecalculationMode::Auto,
            second_options,
            CancellationToken::new(),
        )
        .expect("newer options job")
        .run()
        .expect("newer job completes");

    session.install(newer).expect("newer job installs");
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("A1")))),
        Some(&CalculationCellResult::Value(number(2.0))),
    );
    let error = session
        .install(older)
        .expect_err("older same-revision job must be stale");
    assert_eq!(error.code(), SessionErrorCode::StaleResult);
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("A1")))),
        Some(&CalculationCellResult::Value(number(2.0))),
    );
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
                WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("SUM(B1#)")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("F1"),
                    formula("IFERROR(SUM(B1#),99)"),
                ),
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
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(3.0))),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("C1"),
                number(100.0),
            )]),
        )
        .expect("spill child obstruction");
    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("spill child obstruction rejects forced incremental");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);
    let obstructed = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto full spill obstruction calculation");
    assert_eq!(obstructed.mode(), CalculationExecutionMode::Full);
    assert_eq!(
        obstructed.reason(),
        CalculationDecisionReason::DynamicTopology
    );
    assert_eq!(
        obstructed
            .removed_materialized_cells()
            .iter()
            .map(|cell| cell.address().to_string())
            .collect::<Vec<_>>(),
        vec!["C1"],
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("B1")))),
        Some(&CalculationCellResult::Value(CellValue::Error(
            cellrune::ExcelError::Spill
        ))),
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(CellValue::Error(
            cellrune::ExcelError::Spill
        ))),
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("F1")))),
        Some(&CalculationCellResult::Value(number(99.0))),
    );
    assert_calculations_equal(
        session.calculation().expect("installed obstructed spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::clear_cell(sheet_id, address("C1"))]),
        )
        .expect("remove spill child obstruction");
    let recovered = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("auto full spill recovery");
    assert_eq!(recovered.mode(), CalculationExecutionMode::Full);
    assert_eq!(
        recovered.reason(),
        CalculationDecisionReason::DynamicTopology
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(3.0))),
    );
    assert_calculations_equal(
        session.calculation().expect("installed recovered spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

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
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(6.0))),
    );
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
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(1.0))),
    );
    assert_calculations_equal(
        session.calculation().expect("installed shrunk spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
}

#[test]
fn declared_spill_references_recalculate_incrementally_and_remove_stale_children() {
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
                    Some(CellRange::new(address("B1"), address("C1")).expect("declared spill")),
                )
                .expect("valid dynamic formula"),
                WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("SUM(B1#)")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("F1"),
                    formula("IFERROR(SUM(B1#),99)"),
                ),
            ]),
        )
        .expect("initial declared spill batch");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial declared spill calculation");
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(3.0))),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("C1"),
                number(100.0),
            )]),
        )
        .expect("declared spill member edit");
    let member_edit = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("declared spill member invalidates its owner");
    assert_eq!(member_edit.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(member_edit.dirty_count(), 3);
    assert_eq!(member_edit.evaluated_count(), 3);
    assert_calculations_equal(
        session
            .calculation()
            .expect("installed declared member edit"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::clear_cell(sheet_id, address("C1"))]),
        )
        .expect("clear declared spill member source");
    let member_clear = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("declared spill member clear invalidates its owner");
    assert_eq!(member_clear.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(member_clear.dirty_count(), 3);
    assert_eq!(member_clear.evaluated_count(), 3);
    assert_calculations_equal(
        session
            .calculation()
            .expect("installed declared member clear"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(3.0),
            )]),
        )
        .expect("declared shape mismatch input");
    let blocked = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("declared spill remains incrementally safe");
    assert_eq!(blocked.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        blocked
            .removed_materialized_cells()
            .iter()
            .map(|cell| cell.address().to_string())
            .collect::<Vec<_>>(),
        vec!["C1"],
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(CellValue::Error(
            cellrune::ExcelError::Spill
        ))),
    );
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("F1")))),
        Some(&CalculationCellResult::Value(number(99.0))),
    );
    assert_calculations_equal(
        session.calculation().expect("installed blocked spill"),
        &calculate_workbook(session.workbook(), CalculationOptions::default()),
    );

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("restore declared shape");
    let restored = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("restored declared spill");
    assert_eq!(restored.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("E1")))),
        Some(&CalculationCellResult::Value(number(3.0))),
    );
    assert_calculations_equal(
        session.calculation().expect("installed restored spill"),
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
fn dynamic_reference_target_switch_recompiles_and_matches_fresh_full() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(
                    sheet_id,
                    address("A1"),
                    CellValue::Text("C1".to_owned()),
                ),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("INDIRECT(A1)")),
                WorkbookChange::set_cell_value(sheet_id, address("C1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("D1"), formula("C1+1")),
            ]),
        )
        .expect("dynamic-reference workbook");
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
                CellValue::Text("D1".to_owned()),
            )]),
        )
        .expect("switch INDIRECT target");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("dynamic full fallback");

    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::DynamicTopology);
    assert_eq!(
        session
            .calculation()
            .and_then(|snapshot| snapshot.cell(CalculationCellId::new(sheet_id, address("B1")))),
        Some(&CalculationCellResult::Value(number(2.0))),
    );
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
fn let_bindings_preserve_incremental_dependencies_and_shadow_dynamic_names() {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("LET(item,SUM(A1:A2),item+1)"),
                ),
                WorkbookChange::set_defined_name(defined_name("item", "OFFSET(Z1,0,0)")),
            ]),
        )
        .expect("shadowed LET setup");
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
                address("A2"),
                number(4.0),
            )]),
        )
        .expect("LET dependency edit");

    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("local LET binding must keep incremental calculation safe");
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
fn random_static_edit_sequences_match_a_fresh_full_calculation_and_stay_incremental() {
    let modes = random_edit_sequences(EditProfile::Static, 1..=8);
    // Formula edits recompile in place, value edits recalculate their dependents, and both stay
    // incremental.
    assert!(
        modes.contains_key("Incremental/DirtySubset/parsed"),
        "no in-place formula recompilation: {modes:?}"
    );
    assert!(
        modes.contains_key("Incremental/DirtySubset/unparsed"),
        "no incremental value edit: {modes:?}"
    );
}

#[test]
fn random_dynamic_edit_sequences_match_a_fresh_full_calculation() {
    let modes = random_edit_sequences(EditProfile::Dynamic, 101..=108);
    assert!(
        modes.contains_key("Full/DynamicTopology/parsed")
            || modes.contains_key("Full/DynamicTopology/unparsed"),
        "no recalculation of a workbook with dynamic references: {modes:?}"
    );
}

/// Which formulas a generated edit sequence writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditProfile {
    /// References incremental calculation follows, undefined and changing names, unknown
    /// functions, parse failures, and cycles.
    Static,
    /// The static formulas, `OFFSET` with constant and cell positions, ranges ending in `INDEX`,
    /// spill references, and `SEQUENCE` with and without a declared spill range.
    Dynamic,
}

/// Edits a 12-by-24 grid on two sheets for each seed, in batches of one to three changes, up to
/// four batches before each automatic recalculation, and checks every recalculation against a
/// fresh full calculation. Returns how often each mode, reason, and parsing ran.
fn random_edit_sequences(
    profile: EditProfile,
    seeds: std::ops::RangeInclusive<u64>,
) -> std::collections::BTreeMap<String, usize> {
    let mut modes = std::collections::BTreeMap::new();
    for seed in seeds {
        let mut session = WorkbookCalculationSession::create();
        let first = SheetId::new(1).expect("constant sheet ID");
        let second = session
            .apply_changes(
                0,
                EditBatch::new([WorkbookChange::add_sheet(
                    SheetName::new("Data").expect("valid sheet name"),
                )]),
            )
            .expect("second sheet")
            .created_sheet_ids()[0];
        let mut generator = EditGenerator {
            random: Lcg(seed),
            sheets: [first, second],
            profile,
            nb_defined: true,
        };
        let mut initial = vec![
            WorkbookChange::set_defined_name(defined_name("NA", "Sheet1!$A$1:$A$8")),
            WorkbookChange::set_defined_name(defined_name("NB", "Data!$B$2")),
        ];
        for _ in 0..120 {
            initial.push(generator.cell_change());
        }
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new(initial),
            )
            .expect("generated workbook");
        for step in 0..80 {
            let changes = (0..=generator.random.below(3))
                .map(|_| generator.change())
                .collect::<Vec<_>>();
            session
                .apply_changes(
                    session.workbook().semantic_revision(),
                    EditBatch::new(changes),
                )
                .expect("generated edit");
            if step % 4 != 3 && generator.random.below(3) == 0 {
                continue;
            }
            let delta = session
                .recalculate(
                    RecalculationMode::Auto,
                    CalculationOptions::default(),
                    CancellationToken::new(),
                )
                .unwrap_or_else(|error| panic!("seed {seed}, step {step}: {error}"));
            let parsed = if delta.parsed_formula_count() == 0 {
                "unparsed"
            } else {
                "parsed"
            };
            *modes
                .entry(format!("{:?}/{:?}/{parsed}", delta.mode(), delta.reason()))
                .or_insert(0) += 1;
            let full = calculate_workbook(session.workbook(), CalculationOptions::default());
            let calculation = session.calculation().expect("installed calculation");
            assert_eq!(
                calculation.cells().collect::<Vec<_>>(),
                full.cells().collect::<Vec<_>>(),
                "seed {seed}, step {step}, {:?}",
                delta.reason(),
            );
            assert_eq!(
                calculation.materialized_cells().collect::<Vec<_>>(),
                full.materialized_cells().collect::<Vec<_>>(),
                "seed {seed}, step {step}, {:?}",
                delta.reason(),
            );
        }
    }
    modes
}

/// A linear congruential generator, so every run edits the same cells.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(bound)) as u32
    }

    fn cell(&mut self) -> String {
        let column = char::from(b'A' + self.below(12) as u8);
        format!("{column}{}", self.below(24) + 1)
    }

    fn column(&mut self) -> char {
        char::from(b'A' + self.below(12) as u8)
    }
}

struct EditGenerator {
    random: Lcg,
    sheets: [SheetId; 2],
    profile: EditProfile,
    /// Whether the workbook defines `NB`, which formulas read as an undefined name otherwise.
    nb_defined: bool,
}

impl EditGenerator {
    /// A cell edit, or now and then a change to the names `NA` and `NB`.
    fn change(&mut self) -> WorkbookChange {
        if self.random.below(30) != 0 {
            return self.cell_change();
        }
        let sheet = ["Sheet1", "Data"][self.random.below(2) as usize];
        let (column, top) = (self.random.column(), self.random.below(16) + 1);
        match self.random.below(3) {
            0 => WorkbookChange::set_defined_name(defined_name(
                "NA",
                &format!("{sheet}!${column}${top}:${column}${}", top + 7),
            )),
            1 if self.nb_defined => {
                self.nb_defined = false;
                WorkbookChange::remove_defined_name(DefinedNameScope::Workbook, "NB")
            }
            _ => {
                self.nb_defined = true;
                WorkbookChange::set_defined_name(defined_name(
                    "NB",
                    &format!("{sheet}!${column}${top}"),
                ))
            }
        }
    }

    fn cell_change(&mut self) -> WorkbookChange {
        let random = &mut self.random;
        let sheet = self.sheets[random.below(2) as usize];
        let target = CellAddress::from_a1(&random.cell()).expect("valid generated address");
        let value = match random.below(10) {
            0 => return WorkbookChange::clear_cell(sheet, target),
            1 => CellValue::Text(["k1", "k2", "K1", " 3 "][random.below(4) as usize].to_owned()),
            2 => CellValue::Logical(random.below(2) == 0),
            3 | 4 => number(f64::from(random.below(10))),
            _ => {
                let (a, b, column) = (random.cell(), random.cell(), random.column());
                let (top, bottom) = (random.below(12) + 1, random.below(12) + 13);
                let dynamic = self.profile == EditProfile::Dynamic;
                if dynamic && random.below(15) == 0 {
                    let declared = random.below(2) == 0;
                    let range = declared.then(|| {
                        CellRange::new(
                            target,
                            CellAddress::from_indices(
                                target.row().get() + 2,
                                target.column().get(),
                            )
                            .expect("valid spill end"),
                        )
                        .expect("declared range")
                    });
                    return WorkbookChange::set_cell_dynamic_formula(
                        sheet,
                        target,
                        formula(&format!("SEQUENCE(3,1,{a})")),
                        range,
                    )
                    .expect("dynamic formula");
                }
                let text = match random.below(if dynamic { 23 } else { 19 }) {
                    0 => format!("{a}+{b}"),
                    1 => format!("{a}*2-{b}"),
                    2 => format!("SUM({column}{top}:{column}{bottom})"),
                    3 => format!("SUM(${column}$1:{column}{bottom})"),
                    4 => format!(
                        "SUMIFS({column}1:{column}24,A1:A24,\"k1\",B1:B24,{})",
                        random.below(5)
                    ),
                    5 => format!("COUNTIFS(A1:A24,\"K1\",C1:C24,{})", random.below(5)),
                    6 => format!("IF({a}>4,{b},{a})"),
                    7 => format!("AVERAGE({column}{top}:{column}{bottom})"),
                    8 => format!("IFERROR(VLOOKUP({a},A1:D24,2,FALSE),-1)"),
                    9 => format!("Data!{a}+{b}"),
                    10 => format!("MAX(A{top}:H{bottom})"),
                    11 => format!("COUNTIF(B1:B24,{a})"),
                    12 => format!("NB*3+{a}"),
                    13 => format!("SUM(NA)+COUNT(NA)*{a}"),
                    14 => format!("FOOBAR({a})"),
                    15 => format!("IF({a}>4,{b},FOOBAR())"),
                    16 => format!("SUM(({a}"),
                    17 => format!("ISFORMULA({a})+N({b})"),
                    18 => format!("{a}-1"),
                    19 => format!("OFFSET({a},{},0)", random.below(3)),
                    20 => format!("SUM(A1:INDEX(A1:A24,{}))", random.below(24) + 1),
                    21 => format!("OFFSET(A1,{b},1)"),
                    _ => format!("SUM({column}{top}#)"),
                };
                return WorkbookChange::set_cell_formula(sheet, target, formula(&text));
            }
        };
        WorkbookChange::set_cell_value(sheet, target, value)
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

    // A name change compiles the workbook again. Formula edits, even ones that close or open a
    // cycle, leave the compiled state to the recalculation, which evaluates both formulas in full
    // because together they are every formula in the workbook.
    for (changes, mode, reason) in [
        (
            vec![WorkbookChange::set_defined_name(defined_name(
                "Factor", "A1*3",
            ))],
            CalculationExecutionMode::Full,
            CalculationDecisionReason::TopologyChanged,
        ),
        (
            vec![
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("C1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
            ],
            CalculationExecutionMode::Full,
            CalculationDecisionReason::DirtySetCoversWorkbook,
        ),
        (
            vec![WorkbookChange::set_cell_formula(
                sheet_id,
                address("B1"),
                formula("Factor+1"),
            )],
            CalculationExecutionMode::Full,
            CalculationDecisionReason::DirtySetCoversWorkbook,
        ),
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
            .expect("topology calculation");
        assert_eq!((delta.mode(), delta.reason()), (mode, reason));
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

#[test]
fn random_draws_agree_across_full_targeted_incremental_and_preview_calculation() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let options = CalculationOptions::default().with_random_seed(42);
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("RAND()")),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("C1"),
                    formula("RANDBETWEEN(1,100)+A1"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("D1"),
                    formula("SUM(MAKEARRAY(2,2,LAMBDA(r,c,RAND())))"),
                ),
                WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("B1*2")),
            ]),
        )
        .expect("random workbook");
    let full = session
        .recalculate(RecalculationMode::Full, options, CancellationToken::new())
        .expect("full calculation");
    assert_eq!(full.mode(), CalculationExecutionMode::Full);
    let first = calculate_workbook(session.workbook(), options);
    assert_calculations_equal(session.calculation().expect("installed full"), &first);
    let targeted = calculate_targets(
        session.workbook(),
        &[CalculationTarget::new(
            sheet_id,
            CellRange::new(address("B1"), address("E1")).expect("valid target range"),
        )],
        options,
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("targeted calculation");
    for column in ["B1", "C1", "D1", "E1"] {
        let cell = CalculationCellId::new(sheet_id, address(column));
        assert_eq!(targeted.cell(cell), first.cell(cell), "{column}");
    }

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(5.0),
            )]),
        )
        .expect("unrelated edit");
    let incremental = session
        .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
        .expect("incremental calculation");
    assert_eq!(incremental.reason(), CalculationDecisionReason::DirtySubset);
    assert_eq!(incremental.evaluated_count(), 1);
    let installed = session.calculation().expect("installed incremental");
    assert_calculations_equal(installed, &calculate_workbook(session.workbook(), options));
    for column in ["B1", "D1", "E1"] {
        let cell = CalculationCellId::new(sheet_id, address(column));
        assert_eq!(installed.cell(cell), first.cell(cell), "{column}");
    }

    let mut preview = session
        .prepare_transaction(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                number(9.0),
            )]),
            RecalculationMode::Auto,
            options,
            CancellationToken::new(),
        )
        .expect("preview prepares")
        .run()
        .expect("preview runs");
    session
        .install_transaction(&mut preview)
        .expect("preview installs");
    let installed = session.calculation().expect("installed preview");
    assert_calculations_equal(installed, &calculate_workbook(session.workbook(), options));
    for column in ["B1", "D1", "E1"] {
        let cell = CalculationCellId::new(sheet_id, address(column));
        assert_eq!(installed.cell(cell), first.cell(cell), "{column}");
    }
}

fn volatile_session() -> WorkbookCalculationSession {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut session = WorkbookCalculationSession::create();
    let mut changes = vec![
        WorkbookChange::set_defined_name(defined_name("stamp", "TODAY()+0")),
        WorkbookChange::set_defined_name(defined_name("clock", "LAMBDA(NOW())")),
        WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
    ];
    for (cell, text) in [
        ("B1", "TODAY()"),
        ("C1", "NOW()"),
        ("D1", "RAND()"),
        ("E1", "B1+1"),
        ("F1", "D1*2"),
        ("G1", "A1+1"),
        ("H1", "stamp"),
        ("I1", "LET(f,LAMBDA(RAND()),f())"),
        ("J1", "clock()"),
        ("K1", "SUM(MAKEARRAY(2,2,LAMBDA(r,c,RAND())))"),
    ] {
        changes.push(WorkbookChange::set_cell_formula(
            sheet_id,
            address(cell),
            formula(text),
        ));
    }
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("volatile workbook");
    session
}

fn volatile_options(today: f64, now: f64, seed: u32) -> CalculationOptions {
    CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(today).expect("finite TODAY serial"))
        .with_now_serial(FiniteNumber::new(now).expect("finite NOW serial"))
        .with_random_seed(seed)
}

fn changed_addresses(delta: &cellrune::CalculationDelta) -> Vec<String> {
    delta
        .changed_cells()
        .iter()
        .map(|cell| cell.cell().address().to_string())
        .collect()
}

#[test]
fn a_volatile_input_change_recalculates_only_the_formulas_that_read_it() {
    let mut session = volatile_session();
    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            volatile_options(45_000.0, 45_000.5, 1),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);

    for (options, expected) in [
        (
            volatile_options(45_001.0, 45_000.5, 1),
            vec!["B1", "E1", "H1"],
        ),
        (volatile_options(45_001.0, 45_001.5, 1), vec!["C1", "J1"]),
        (
            volatile_options(45_001.0, 45_001.5, 2),
            vec!["D1", "F1", "I1", "K1"],
        ),
    ] {
        let delta = session
            .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
            .expect("volatile input recalculation");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(delta.reason(), CalculationDecisionReason::DirtySubset);
        assert_eq!(delta.dirty_count(), expected.len());
        assert_eq!(delta.evaluated_count(), expected.len());
        assert_eq!(delta.parsed_formula_count(), 0);
        assert_eq!(changed_addresses(&delta), expected);
        assert_calculations_equal(
            session.calculation().expect("installed calculation"),
            &calculate_workbook(session.workbook(), options),
        );
    }

    let seed_three = volatile_options(45_001.0, 45_001.5, 3);
    let forced = session
        .recalculate(
            RecalculationMode::Incremental,
            seed_three,
            CancellationToken::new(),
        )
        .expect("a seed change is incrementally safe");
    assert_eq!(forced.mode(), CalculationExecutionMode::Incremental);
    assert_eq!(
        forced.reason(),
        CalculationDecisionReason::IncrementalRequested
    );
    assert_eq!(forced.evaluated_count(), 4);

    let without_seed = CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(45_001.0).expect("finite TODAY serial"))
        .with_now_serial(FiniteNumber::new(45_001.5).expect("finite NOW serial"));
    let removed = session
        .recalculate(
            RecalculationMode::Auto,
            without_seed,
            CancellationToken::new(),
        )
        .expect("removing the seed");
    assert_eq!(removed.reason(), CalculationDecisionReason::DirtySubset);
    let installed = session.calculation().expect("installed calculation");
    assert_calculations_equal(
        installed,
        &calculate_workbook(session.workbook(), without_seed),
    );
    for (cell, code) in [
        ("D1", CalculationIssueCode::VolatileInputMissing),
        ("F1", CalculationIssueCode::BlockedByUpstream),
    ] {
        let Some(CalculationCellResult::Unavailable(issue)) = installed.cell(
            CalculationCellId::new(SheetId::new(1).expect("sheet"), address(cell)),
        ) else {
            panic!("{cell} must be unavailable without a seed");
        };
        assert_eq!(issue.code(), code, "{cell}");
    }

    let other_option = seed_three.with_arithmetic_semantics(ArithmeticSemantics::Ieee754);
    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            other_option,
            CancellationToken::new(),
        )
        .expect_err("another option change is not incrementally safe");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);
    let full = session
        .recalculate(
            RecalculationMode::Auto,
            other_option,
            CancellationToken::new(),
        )
        .expect("another option change recalculates in full");
    assert_eq!(full.mode(), CalculationExecutionMode::Full);
    assert_eq!(full.reason(), CalculationDecisionReason::OptionsChanged);
}

#[test]
fn a_volatile_input_change_without_readers_needs_no_evaluation() {
    let mut session = chain_session();
    let today = |serial: f64| {
        CalculationOptions::default()
            .with_today_serial(FiniteNumber::new(serial).expect("finite TODAY serial"))
    };
    session
        .recalculate(
            RecalculationMode::Auto,
            today(1.0),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    for options in [today(2.0), today(2.0).with_random_seed(9)] {
        let delta = session
            .recalculate(RecalculationMode::Auto, options, CancellationToken::new())
            .expect("volatile input change");
        assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
        assert_eq!(delta.reason(), CalculationDecisionReason::NoDirtyFormulas);
        assert_eq!(delta.evaluated_count(), 0);
        assert!(delta.changed_cells().is_empty());
        assert_eq!(
            session
                .calculation()
                .expect("installed calculation")
                .options(),
            options
        );
    }
}

#[test]
fn a_volatile_input_change_under_dynamic_references_recalculates_in_full() {
    let sheet_id = SheetId::new(1).expect("constant sheet ID");
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_value(sheet_id, address("A2"), number(2.0)),
                WorkbookChange::set_cell_formula(
                    sheet_id,
                    address("B1"),
                    formula("OFFSET(A1,RANDBETWEEN(0,1),0)"),
                ),
            ]),
        )
        .expect("dynamic random workbook");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default().with_random_seed(1),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default().with_random_seed(2),
            CancellationToken::new(),
        )
        .expect("seed change");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::DynamicTopology);
    assert_calculations_equal(
        session.calculation().expect("installed calculation"),
        &calculate_workbook(
            session.workbook(),
            CalculationOptions::default().with_random_seed(2),
        ),
    );
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
