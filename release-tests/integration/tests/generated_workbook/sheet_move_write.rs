use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellValue, OpenOptions,
    RecalculationWriteOptions, SavedResult, WorkbookDraft, WriteOptions, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes, write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support as support;
use crate::support::sheet_structure_xlsx::generated_sheet_structure_fixture;

const WORKBOOK_PART: &str = "xl/workbook.xml";

fn moved(source: &[u8], moves: &[(&str, usize)]) -> WorkbookDraft {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    for (name, position) in moves {
        let sheet_id = draft
            .workbook()
            .sheet_by_name(name)
            .expect("moved sheet")
            .id();
        draft.move_sheet(sheet_id, *position).expect("move sheet");
    }
    draft
}

fn write(draft: &WorkbookDraft) -> Vec<u8> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, RecalculationWriteOptions::default())
        .expect("write")
        .bytes()
        .to_vec()
}

fn formula(document: &cellrune::XlsxDocument, address: &str) -> (String, SavedResult) {
    let formula = support::formula_at(document, "S1", address);
    (
        formula.text().expect("formula text").as_str().to_owned(),
        formula.saved_result().clone(),
    )
}

fn number(value: f64) -> SavedResult {
    SavedResult::Present(CellValue::number(value).expect("number"))
}

#[test]
fn moving_a_sheet_reorders_tabs_and_keeps_ids_scopes_and_the_active_sheet() {
    let source = generated_sheet_structure_fixture();
    let output = write(&moved(&source, &[("S3", 5)]));
    let workbook = support::part_text(&output, WORKBOOK_PART);
    // Excel 16.114: S3 after S6 keeps every sheet ID, renumbers `localSheetId`, and turns
    // `'S3:S4'` into `'S4'`. The active sheet stays S3, now the last tab.
    for expected in [
        r#"<workbookView xWindow="0" yWindow="0" firstSheet="1" activeTab="5"/>"#,
        r#"<sheets><sheet r:id="rId1" name="S1" sheetId="1"/><sheet r:id="rId2" name="S2" sheetId="2"/><sheet r:id="rId4" name="S4" sheetId="4"/><sheet r:id="rId5" name="S5" sheetId="5"/><sheet r:id="rId6" name="S6" sheetId="6"/><sheet r:id="rId3" name="S3" sheetId="3"/></sheets>"#,
        r#"<definedName name="Local" localSheetId="5">'S3'!$A$1</definedName>"#,
        r#"<definedName name="Span">'S4'!$A$1</definedName>"#,
    ] {
        assert!(workbook.contains(expected), "{expected}: {workbook}");
    }
    // Other parts keep their bytes, and the conditional format's single-sheet reference stays.
    let source_parts = support::archive_parts(&source);
    let parts = support::archive_parts(&output);
    for part in ["xl/worksheets/sheet3.xml", "xl/_rels/workbook.xml.rels"] {
        assert_eq!(source_parts[part], parts[part], "{part}");
    }

    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    for (address, text, saved) in [
        ("B1", "SUM('S2:S5'!A1)", number(11.0)),
        ("B2", "SUM('S4'!A1)", number(4.0)),
        ("B3", "SUM('S2:S3'!A1)", number(2.0 + 4.0 + 5.0 + 6.0 + 3.0)),
        ("B4", "SHEET('S5'!A1)", number(4.0)),
        ("B5", "SUM(Span)", number(4.0)),
        ("B7", "'S6'!A1*2", number(12.0)),
    ] {
        assert_eq!(
            formula(&reopened, address),
            (text.to_owned(), saved),
            "S1!{address}"
        );
    }
}

/// Moving sheets and recalculating gives the results that opening the saved file and calculating
/// it gives, for single moves and for a move after a deletion.
#[test]
fn move_then_full_recalculation_equals_the_saved_file_calculated_fresh() {
    let source = generated_sheet_structure_fixture();
    for moves in [
        &[("S3", 5)][..],
        &[("S6", 3)],
        &[("S5", 1)],
        &[("S2", 4), ("S6", 0)],
        &[("S1", 5), ("S5", 1)],
    ] {
        let mut draft = moved(&source, moves);
        if moves.len() > 1 {
            let s4 = draft.workbook().sheet_by_name("S4").expect("S4").id();
            draft.delete_sheet(s4).expect("delete S4");
            let s2 = draft.workbook().sheet_by_name("S2").expect("S2").id();
            draft.move_sheet(s2, 0).expect("move S2 first");
        }
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .unwrap_or_else(|error| panic!("{moves:?}: {error}"));
        let reopened =
            open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
        let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
        let results = |calculation: &cellrune::CalculationSnapshot| {
            calculation
                .cells()
                .map(|(cell, result)| (cell, result.clone()))
                .collect::<Vec<(CalculationCellId, CalculationCellResult)>>()
        };
        assert_eq!(results(&calculation), results(&fresh), "{moves:?}");
        let order = |workbook: &cellrune::WorkbookSnapshot| {
            workbook
                .sheets()
                .iter()
                .map(|sheet| (sheet.id(), sheet.name().as_str().to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            order(draft.workbook()),
            order(reopened.workbook()),
            "{moves:?}"
        );
        assert_eq!(
            draft.workbook().defined_names(),
            reopened.workbook().defined_names(),
            "{moves:?}"
        );
    }
}

#[test]
fn edit_only_write_clears_the_results_a_move_changes() {
    let source = generated_sheet_structure_fixture();
    let output =
        write_xlsx_draft_edits_bytes(&moved(&source, &[("S6", 3)]), WriteOptions::default())
            .expect("edits");
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("open");
    // The 3-D sums and the sheet function read the tab order; a plain reference does not.
    for address in ["B1", "B2", "B3", "B4"] {
        assert_eq!(
            formula(&reopened, address).1,
            SavedResult::Missing,
            "S1!{address}"
        );
    }
    assert_eq!(formula(&reopened, "B7").1, number(12.0));
}

#[test]
fn canonical_draft_saves_moved_sheets_in_their_new_order() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();
    let name = |name: &str| cellrune::SheetName::new(name).expect("sheet name");
    let second = draft.add_sheet(name("Second")).expect("second");
    let third = draft.add_sheet(name("Third")).expect("third");
    draft
        .set_defined_name(
            cellrune::DefinedName::new(
                "Here",
                cellrune::DefinedNameScope::Sheet(third),
                cellrune::FormulaText::from_xlsx("Third!$A$1").expect("formula"),
                false,
            )
            .expect("name"),
        )
        .expect("scoped name");
    draft.move_sheet(third, 0).expect("move Third first");
    let reopened = open_xlsx_document_bytes(&write(&draft), OpenOptions::default()).expect("open");
    let ids = reopened
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.id())
        .collect::<Vec<_>>();
    assert_eq!(ids, [third, first, second]);
    assert_eq!(
        reopened.workbook().defined_names()[0].scope(),
        cellrune::DefinedNameScope::Sheet(third)
    );
}
