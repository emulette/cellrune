use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CalculationTarget,
    CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber, OpenOptions,
    RecalculationMode, SheetId, TargetCalculationLimits, TargetCalculationResult,
    UncalculablePrecedents, WorkbookCalculationSession, WorkbookChange, WorkbookDraft,
    calculate_targets, open_xlsx_document_bytes,
};

use crate::support::generated_xlsx::generated_saved_precedent_fixture;

fn cell(address: &str) -> CalculationCellId {
    CalculationCellId::new(
        SheetId::new(1).expect("sheet ID"),
        CellAddress::from_a1(address).expect("address"),
    )
}

fn number(value: f64) -> CalculationCellResult {
    CalculationCellResult::Value(CellValue::Number(FiniteNumber::new(value).expect("finite")))
}

fn session(full_calculation_on_load: bool) -> WorkbookCalculationSession {
    let source = generated_saved_precedent_fixture(full_calculation_on_load);
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    WorkbookCalculationSession::new(WorkbookDraft::from_document(&document))
}

fn calculate(
    session: &WorkbookCalculationSession,
    precedents: UncalculablePrecedents,
) -> TargetCalculationResult {
    session
        .calculate_targets_with(
            &["B1", "C1", "D1", "E1"].map(|address| CalculationTarget::cell(cell(address))),
            CalculationOptions::default(),
            TargetCalculationLimits::default(),
            precedents,
            CancellationToken::new(),
        )
        .expect("targeted calculation")
}

fn unavailable(result: &TargetCalculationResult, address: &str) -> bool {
    matches!(
        result.cell(cell(address)),
        Some(CalculationCellResult::Unavailable(_))
    )
}

#[test]
fn an_uncalculable_precedent_gives_its_saved_result_only_when_asked() {
    let session = session(false);
    let without = calculate(&session, UncalculablePrecedents::Unavailable);
    assert!(unavailable(&without, "C1"));
    assert!(without.saved_precedents(cell("C1")).is_empty());

    let with = calculate(&session, UncalculablePrecedents::SavedResult);
    assert_eq!(with.cell(cell("C1")), Some(&number(84.0)));
    assert_eq!(with.saved_precedents(cell("C1")), [cell("B1")]);
    assert_eq!(with.cell(cell("E1")), Some(&number(126.0)));
    assert_eq!(with.saved_precedents(cell("E1")), [cell("B1")]);
    assert!(
        unavailable(&with, "B1"),
        "a requested cell's own formula keeps its result"
    );
    for result in [&without, &with] {
        assert_eq!(result.cell(cell("D1")), Some(&number(6.0)));
        assert!(result.saved_precedents(cell("D1")).is_empty());
    }

    let free = calculate_targets(
        session.workbook(),
        &[CalculationTarget::cell(cell("C1"))],
        CalculationOptions::default(),
        TargetCalculationLimits::default(),
        CancellationToken::new(),
    )
    .expect("free targeted calculation");
    assert!(
        unavailable(&free, "C1"),
        "the free function takes no saved result"
    );
}

#[test]
fn a_formula_reading_a_volatile_one_gives_no_saved_result() {
    let session = session(false);
    let request = |address: &str, options: CalculationOptions| {
        session
            .calculate_targets_with(
                &[CalculationTarget::cell(cell(address))],
                options,
                TargetCalculationLimits::default(),
                UncalculablePrecedents::SavedResult,
                CancellationToken::new(),
            )
            .expect("targeted calculation")
    };
    // G1 reads NOW(), whose saved result belongs to the time the file was saved.
    assert!(unavailable(
        &request("J1", CalculationOptions::default()),
        "J1"
    ));
    let now =
        CalculationOptions::default().with_now_serial(FiniteNumber::new(46000.25).expect("finite"));
    assert!(
        unavailable(&request("L1", now), "L1"),
        "K1 reads NOW() even when the request supplies the time"
    );
}

#[test]
fn a_precedent_over_the_dependency_budget_gives_its_saved_result() {
    let session = session(false);
    let options = CalculationOptions::default().with_limits(
        cellrune::CalculationLimits::default()
            .with_max_dependency_edges(2)
            .expect("limit"),
    );
    let result = session
        .calculate_targets_with(
            &[CalculationTarget::cell(cell("I1"))],
            options,
            TargetCalculationLimits::default(),
            UncalculablePrecedents::SavedResult,
            CancellationToken::new(),
        )
        .expect("targeted calculation");
    assert_eq!(result.cell(cell("I1")), Some(&number(504.0)));
    assert!(result.saved_precedents(cell("I1")).contains(&cell("H1")));
}

#[test]
fn a_copied_sheet_gives_no_saved_result() {
    let mut session = session(false);
    let sheet = SheetId::new(1).expect("sheet ID");
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([
                WorkbookChange::set_cell_value(
                    sheet,
                    CellAddress::from_a1("A1").expect("address"),
                    CellValue::Number(FiniteNumber::new(7.0).expect("finite")),
                ),
                WorkbookChange::copy_sheet(sheet, 2),
            ]),
        )
        .expect("edit and copy");
    let copy = session.workbook().sheets()[2].id();
    let target = CalculationCellId::new(copy, CellAddress::from_a1("C1").expect("address"));
    let result = session
        .calculate_targets_with(
            &[CalculationTarget::cell(target)],
            CalculationOptions::default(),
            TargetCalculationLimits::default(),
            UncalculablePrecedents::SavedResult,
            CancellationToken::new(),
        )
        .expect("targeted calculation");
    assert!(
        matches!(
            result.cell(target),
            Some(CalculationCellResult::Unavailable(_))
        ),
        "the copy's B1 was saved with A1 = 5"
    );
}

#[test]
fn an_imported_sheet_gives_no_saved_result() {
    let placeholder = generated_saved_precedent_fixture(true);
    let placeholder =
        open_xlsx_document_bytes(&placeholder, OpenOptions::default()).expect("open placeholder");
    let imported = cellrune::ImportedSheet::from_draft(
        &WorkbookDraft::from_document(&placeholder),
        SheetId::new(1).expect("sheet ID"),
    )
    .expect("importable sheet");
    let mut session = session(false);
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([WorkbookChange::import_sheet(imported, 2)]),
        )
        .expect("import");
    let sheet = session
        .workbook()
        .sheets()
        .last()
        .expect("imported sheet")
        .id();
    let target = CalculationCellId::new(sheet, CellAddress::from_a1("C1").expect("address"));
    let result = session
        .calculate_targets_with(
            &[CalculationTarget::cell(target)],
            CalculationOptions::default(),
            TargetCalculationLimits::default(),
            UncalculablePrecedents::SavedResult,
            CancellationToken::new(),
        )
        .expect("targeted calculation");
    assert!(matches!(
        result.cell(target),
        Some(CalculationCellResult::Unavailable(_))
    ));
    assert_eq!(
        calculate(&session, UncalculablePrecedents::SavedResult).cell(cell("C1")),
        Some(&number(84.0)),
        "the file's own sheet still gives its saved results"
    );
}

#[test]
fn a_file_asking_for_recalculation_on_open_gives_no_saved_result() {
    let with = calculate(&session(true), UncalculablePrecedents::SavedResult);
    assert!(unavailable(&with, "C1"));
}

#[test]
fn an_edit_reaching_the_precedent_withdraws_its_saved_result() {
    let mut session = session(false);
    let sheet = SheetId::new(1).expect("sheet ID");
    let edit = |session: &mut WorkbookCalculationSession, address: &str| {
        let revision = session.workbook().semantic_revision();
        session
            .apply_changes(
                revision,
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    CellAddress::from_a1(address).expect("address"),
                    CellValue::Number(FiniteNumber::new(7.0).expect("finite")),
                )]),
            )
            .expect("edit");
    };
    edit(&mut session, "Z9");
    let unrelated = calculate(&session, UncalculablePrecedents::SavedResult);
    assert_eq!(unrelated.cell(cell("C1")), Some(&number(84.0)));
    edit(&mut session, "A1");
    let reached = calculate(&session, UncalculablePrecedents::SavedResult);
    assert!(unavailable(&reached, "C1"), "B1 reads the edited A1");
    assert_eq!(reached.cell(cell("D1")), Some(&number(8.0)));
}

#[test]
fn a_complete_calculation_is_not_reused_for_saved_results() {
    let mut session = session(false);
    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("complete calculation");
    let without = calculate(&session, UncalculablePrecedents::Unavailable);
    assert!(without.reused_count() > 0);
    assert!(unavailable(&without, "C1"));
    let with = calculate(&session, UncalculablePrecedents::SavedResult);
    assert_eq!(with.reused_count(), 0);
    assert_eq!(with.cell(cell("C1")), Some(&number(84.0)));
}

#[test]
fn calculable_precedents_give_the_same_results_either_way() {
    let session = session(false);
    let target = [CalculationTarget::cell(cell("D1"))];
    let results = [
        UncalculablePrecedents::Unavailable,
        UncalculablePrecedents::SavedResult,
    ]
    .map(|precedents| {
        session
            .calculate_targets_with(
                &target,
                CalculationOptions::default(),
                TargetCalculationLimits::default(),
                precedents,
                CancellationToken::new(),
            )
            .expect("targeted calculation")
            .cells()
            .map(|(cell, result)| (cell, result.clone()))
            .collect::<Vec<_>>()
    });
    assert_eq!(results[0], results[1]);
}

mod reopened {
    use cellrune::{
        CellAddress, EditBatch, FormulaText, OpenOptions, SheetId, UncalculablePrecedents,
        WorkbookCalculationSession, WorkbookChange, WorkbookDraft, WriteOptions,
        open_xlsx_document_bytes, write_xlsx_draft_edits_bytes,
    };

    use super::super::recalculation_write_support::{part_text, replace_part_text};
    use super::{calculate, cell, number, unavailable};
    use crate::support::generated_xlsx::generated_saved_precedent_fixture;

    const MARK_URI: &str = "{6F1B9E52-2C7A-4E0D-9A43-5D8C11E2B7A1}";

    /// Writes a formula on `Other`, which no formula of `Inputs` reads, with a minimal write that
    /// asks the host to recalculate on open.
    fn edited(source: &[u8], mark: bool) -> Vec<u8> {
        edited_with(source, mark, "Inputs!D1*3")
    }

    fn edited_with(source: &[u8], mark: bool, formula: &str) -> Vec<u8> {
        let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
                SheetId::new(2).expect("Other"),
                CellAddress::from_a1("A1").expect("address"),
                FormulaText::from_xlsx(formula).expect("formula"),
            )]))
            .expect("edit");
        let output = write_xlsx_draft_edits_bytes(
            &draft,
            WriteOptions::default().with_saved_result_mark(mark),
        )
        .expect("minimal write");
        assert!(output.report().requests_host_recalculation());
        output.bytes().to_vec()
    }

    fn session(bytes: &[u8]) -> WorkbookCalculationSession {
        let document = open_xlsx_document_bytes(bytes, OpenOptions::default()).expect("reopen");
        WorkbookCalculationSession::new(WorkbookDraft::from_document(&document))
    }

    #[test]
    fn a_marked_minimal_write_keeps_its_saved_results_current() {
        let marked = edited(&generated_saved_precedent_fixture(false), true);
        assert!(part_text(&marked, "xl/workbook.xml").contains(MARK_URI));
        let result = calculate(&session(&marked), UncalculablePrecedents::SavedResult);
        assert_eq!(result.cell(cell("C1")), Some(&number(84.0)));
        assert_eq!(result.saved_precedents(cell("C1")), [cell("B1")]);

        let again = edited_with(&marked, true, "Inputs!D1*4");
        let result = calculate(&session(&again), UncalculablePrecedents::SavedResult);
        assert_eq!(
            result.cell(cell("C1")),
            Some(&number(84.0)),
            "a marked file written again keeps the trust"
        );
    }

    #[test]
    fn an_unmarked_or_changed_file_gives_no_saved_result() {
        let unmarked = edited(&generated_saved_precedent_fixture(false), false);
        assert!(
            !part_text(&unmarked, "xl/workbook.xml").contains(MARK_URI),
            "the default minimal write leaves no mark"
        );
        assert!(unavailable(
            &calculate(&session(&unmarked), UncalculablePrecedents::SavedResult),
            "C1"
        ));

        let marked = edited(&generated_saved_precedent_fixture(false), true);
        let changed =
            replace_part_text(&marked, "xl/worksheets/sheet1.xml", "<v>5</v>", "<v>6</v>");
        assert!(unavailable(
            &calculate(&session(&changed), UncalculablePrecedents::SavedResult),
            "C1"
        ));
    }

    #[test]
    fn a_minimal_write_with_a_copied_sheet_is_never_marked() {
        let source = generated_saved_precedent_fixture(false);
        let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new([
                WorkbookChange::set_cell_formula(
                    SheetId::new(2).expect("Other"),
                    CellAddress::from_a1("A1").expect("address"),
                    FormulaText::from_xlsx("Inputs!D1*3").expect("formula"),
                ),
                WorkbookChange::copy_sheet(SheetId::new(1).expect("Inputs"), 2),
            ]))
            .expect("edit and copy");
        let output = write_xlsx_draft_edits_bytes(
            &draft,
            WriteOptions::default().with_saved_result_mark(true),
        )
        .expect("minimal write");
        assert!(!part_text(output.bytes(), "xl/workbook.xml").contains(MARK_URI));
    }

    #[test]
    fn a_placeholder_source_is_never_marked() {
        let marked = edited(&generated_saved_precedent_fixture(true), true);
        assert!(!part_text(&marked, "xl/workbook.xml").contains(MARK_URI));
        assert!(unavailable(
            &calculate(&session(&marked), UncalculablePrecedents::SavedResult),
            "C1"
        ));
    }
}
