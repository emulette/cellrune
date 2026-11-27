use cellrune::{
    CalculationOptions, CellAddress, CellRange, DataValidation, DataValidationErrorStyle,
    DataValidationOperator, DataValidationType, DefinedName, DefinedNameScope, EditBatch,
    FormulaText, OpenOptions, RecalculationWriteOptions, Row, SheetId, SheetName, ValidationError,
    WorkbookChange, WorkbookDraft, WorkbookSnapshot, WriteOptions, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes, write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support::{archive_parts, part_text};
use crate::support::generated_xlsx::{
    generated_data_validation_edit_fixture, generated_data_validation_workbook,
    generated_lenient_validation_fixture,
};

fn range(value: &str) -> CellRange {
    match value.split_once(':') {
        Some((start, end)) => CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range"),
        None => {
            let cell = CellAddress::from_a1(value).expect("cell");
            CellRange::new(cell, cell).expect("cell range")
        }
    }
}

fn ranges(values: &[&str]) -> Vec<CellRange> {
    values.iter().map(|value| range(value)).collect()
}

fn list(values: &[&str], source: &str) -> DataValidation {
    DataValidation::new(ranges(values), DataValidationType::List).with_formula1(source)
}

/// The sheet's rules in a canonical order: the reader returns the main rules before the
/// extension's, whatever order the model held them in.
fn rules(workbook: &WorkbookSnapshot, sheet: &str) -> Vec<String> {
    let mut rules = workbook
        .sheet_by_name(sheet)
        .expect("sheet")
        .data_validations()
        .iter()
        .map(|validation| format!("{validation:?}"))
        .collect::<Vec<_>>();
    rules.sort();
    rules
}

fn open_draft(source: &[u8]) -> (WorkbookDraft, SheetId) {
    let document = open_xlsx_document_bytes(source, OpenOptions::default()).expect("open");
    let draft = WorkbookDraft::from_document(&document);
    let script = draft
        .workbook()
        .sheet_by_name("Script")
        .expect("Script")
        .id();
    (draft, script)
}

fn minimal_write(draft: &WorkbookDraft) -> Vec<u8> {
    write_xlsx_draft_edits_bytes(draft, WriteOptions::default())
        .expect("minimal write")
        .bytes()
        .to_vec()
}

fn recalculated_write(draft: &WorkbookDraft) -> Vec<u8> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, RecalculationWriteOptions::default())
        .expect("recalculated write")
        .bytes()
        .to_vec()
}

fn reopened(bytes: &[u8]) -> WorkbookSnapshot {
    open_xlsx_document_bytes(bytes, OpenOptions::default())
        .expect("reopen")
        .workbook()
        .clone()
}

fn every_kind_of_rule() -> Vec<DataValidation> {
    vec![
        list(&["A1:A10", "C1"], "\"R&D,Ops\"")
            .with_allow_blank(true)
            .with_input_message(true, Some("Pick"), Some("Choose a team"))
            .with_error_alert(
                true,
                DataValidationErrorStyle::Stop,
                Some("Unknown"),
                Some("Pick from the list"),
            ),
        list(&["B9"], "Roles"),
        DataValidation::new(ranges(&["B1"]), DataValidationType::Whole)
            .with_formula1("1")
            .with_formula2("10"),
        DataValidation::new(ranges(&["B2"]), DataValidationType::Decimal)
            .with_operator(DataValidationOperator::GreaterThanOrEqual)
            .with_formula1("0.5")
            .with_error_alert(true, DataValidationErrorStyle::Warning, None, None),
        DataValidation::new(ranges(&["B3"]), DataValidationType::Date)
            .with_operator(DataValidationOperator::LessThan)
            .with_formula1("DATE(2026,1,1)")
            .with_error_alert(true, DataValidationErrorStyle::Information, None, None),
        DataValidation::new(ranges(&["B4"]), DataValidationType::Time)
            .with_operator(DataValidationOperator::NotBetween)
            .with_formula1("0.25")
            .with_formula2("0.75"),
        DataValidation::new(ranges(&["B5"]), DataValidationType::TextLength)
            .with_operator(DataValidationOperator::LessThanOrEqual)
            .with_formula1("5"),
        DataValidation::new(ranges(&["B6"]), DataValidationType::Custom)
            .with_formula1("ISNUMBER(B6)"),
        DataValidation::new(ranges(&["B7"]), DataValidationType::AnyValue).with_input_message(
            true,
            None,
            Some("Any value"),
        ),
        list(&["B8"], "$D$1:$D$3").with_in_cell_dropdown(false),
        list(&["E1:E1048576"], "Lists!$A$1:$A$3"),
    ]
}

#[test]
fn every_kind_of_rule_reads_back_from_a_new_workbook() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::add_sheet(
            SheetName::new("Lists").expect("sheet name"),
        )]))
        .expect("list sheet");
    let names = DefinedName::new(
        "Roles",
        DefinedNameScope::Workbook,
        FormulaText::from_xlsx("Lists!$A$1:$A$3").expect("formula"),
        false,
    )
    .expect("defined name");
    let mut changes = vec![WorkbookChange::set_defined_name(names)];
    changes.extend(
        every_kind_of_rule()
            .into_iter()
            .map(|validation| WorkbookChange::set_data_validation(sheet, validation)),
    );
    draft.apply_changes(EditBatch::new(changes)).expect("rules");

    let output = recalculated_write(&draft);
    let xml = part_text(&output, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<dataValidations count=\"10\">"),
        "the rules on the sheet stay in the main element: {xml}"
    );
    assert!(
        xml.contains("<x14:dataValidations xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\" count=\"1\">"),
        "a rule naming another sheet goes to the extension: {xml}"
    );
    assert_eq!(
        rules(&reopened(&output), "Sheet1"),
        rules(draft.workbook(), "Sheet1")
    );
}

#[test]
fn edits_read_back_from_both_writes_of_a_document() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_data_validation(script, list(&["G2:G5"], "Roles")),
            WorkbookChange::set_data_validation(script, list(&["H2"], "Lists!$A$1:$A$2")),
            WorkbookChange::clear_data_validations(script, ranges(&["D3:D7", "E5"])),
        ]))
        .expect("edits");
    let expected = rules(draft.workbook(), "Script");
    assert_eq!(expected.len(), 6, "{expected:#?}");

    for output in [minimal_write(&draft), recalculated_write(&draft)] {
        assert_eq!(rules(&reopened(&output), "Script"), expected);
        let xml = part_text(&output, "xl/worksheets/sheet1.xml");
        assert!(
            xml.contains(r#"imeMode="hiragana" sqref="B2:B10""#),
            "an untouched rule keeps its markup: {xml}"
        );
        assert!(
            xml.contains(r#"<dataValidation type="bogus" sqref="F1:F3"/>"#),
            "an unmodeled rule stays: {xml}"
        );
        assert!(xml.contains("<dataValidations count=\"5\">"), "{xml}");
        assert!(xml.contains("<x14:dataValidations count=\"2\">"), "{xml}");
    }
}

#[test]
fn a_minimal_write_of_rule_edits_changes_only_their_worksheet() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
            script,
            list(&["G2:G5"], "Roles"),
        )]))
        .expect("edit");
    let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default()).expect("save");
    let report = output.report();
    assert!(report.affected_formulas().is_empty());
    assert!(!report.requests_host_recalculation());
    assert_eq!(
        report
            .changed_parts()
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>(),
        ["xl/worksheets/sheet1.xml"]
    );
    let before = archive_parts(&source);
    let after = archive_parts(output.bytes());
    for (part, bytes) in &before {
        if part != "xl/worksheets/sheet1.xml" {
            assert_eq!(after.get(part), Some(bytes), "{part}");
        }
    }
    let xml = part_text(output.bytes(), "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<f>A1*2</f><v>4</v>"),
        "the saved result stays: {xml}"
    );
}

#[test]
fn a_rule_cut_by_edits_keeps_the_attributes_the_model_does_not_read() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
            script,
            list(&["B5"], "\"Extra\""),
        )]))
        .expect("first edit");
    draft
        .apply_changes(EditBatch::new([WorkbookChange::insert_rows(
            script,
            Row::new(3).expect("row"),
            2,
        )
        .expect("insert")]))
        .expect("rows");
    draft
        .apply_changes(EditBatch::new([WorkbookChange::clear_data_validations(
            script,
            ranges(&["B12"]),
        )]))
        .expect("second edit");

    let output = minimal_write(&draft);
    assert_eq!(
        rules(&reopened(&output), "Script"),
        rules(draft.workbook(), "Script")
    );
    let xml = part_text(&output, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"imeMode="hiragana" sqref="B2:B6 B8:B11""#),
        "the cut rule keeps imeMode: {xml}"
    );
}

#[test]
fn undoing_each_edit_leaves_the_worksheet_as_it_was() {
    let source = generated_data_validation_edit_fixture();
    let sheet = |bytes: &[u8]| archive_parts(bytes)["xl/worksheets/sheet1.xml"].clone();
    let edits = |script: SheetId| {
        vec![
            WorkbookChange::set_data_validation(script, list(&["B5:D6"], "Roles")),
            WorkbookChange::clear_data_validations(script, ranges(&["D1:E9"])),
            WorkbookChange::remove_data_validation(script, 0),
            WorkbookChange::set_data_validations(script, Vec::new()),
        ]
    };
    let (_, script) = open_draft(&source);
    for edit in edits(script) {
        let (mut draft, _) = open_draft(&source);
        let receipt = draft
            .apply_changes(EditBatch::new([edit.clone()]))
            .expect("edit");
        let inverse = receipt.inverse().expect("inverse").clone();
        assert!(inverse.unrestored().is_empty(), "{edit:?}");
        draft
            .apply_changes(EditBatch::new(inverse.changes().to_vec()))
            .expect("undo");
        assert_eq!(sheet(&minimal_write(&draft)), sheet(&source), "{edit:?}");
    }
}

#[test]
fn edits_reaching_a_rule_the_reader_could_not_model_are_refused() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    let overlap = ValidationError::DataValidationOverlapsUnmodeledRule {
        sheet_id: script.get(),
    };
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
                script,
                list(&["F3:G3"], "Roles"),
            )]))
            .expect_err("set over the unmodeled rule"),
        overlap
    );
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::clear_data_validations(
                script,
                ranges(&["A1:F1"]),
            )]))
            .expect_err("clear over the unmodeled rule"),
        overlap
    );
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_data_validation(script, list(&["G1:G3"], "Roles")),
            WorkbookChange::remove_data_validation(script, 0),
        ]))
        .expect("edits beside the unmodeled rule");
    let xml = part_text(&minimal_write(&draft), "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"<dataValidation type="bogus" sqref="F1:F3"/>"#),
        "{xml}"
    );

    // Rules whose ranges the reader could not read reach every cell.
    let source = generated_data_validation_workbook();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let rules_sheet = draft.workbook().sheet_by_name("Rules").expect("Rules").id();
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
                rules_sheet,
                list(&["Z100"], "Roles"),
            )]))
            .expect_err("set beside rules of unknown ranges"),
        ValidationError::DataValidationOverlapsUnmodeledRule {
            sheet_id: rules_sheet.get(),
        }
    );
}

#[test]
fn a_copy_of_an_edited_sheet_writes_the_edited_rules() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_data_validation(script, list(&["G2:G5"], "Roles")),
            WorkbookChange::copy_sheet(script, 1),
        ]))
        .expect("edit and copy");
    let output = minimal_write(&draft);
    let workbook = reopened(&output);
    assert_eq!(
        rules(&workbook, "Script (2)"),
        rules(draft.workbook(), "Script (2)")
    );
    assert_eq!(
        rules(&workbook, "Script"),
        rules(draft.workbook(), "Script")
    );
}

#[test]
fn an_unmodeled_rule_moves_with_inserted_rows() {
    let source = generated_data_validation_edit_fixture();
    let (mut draft, script) = open_draft(&source);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::insert_rows(
            script,
            Row::new(1).expect("row"),
            1,
        )
        .expect("insert")]))
        .expect("rows");
    // The bogus rule on F1:F3 now covers F2:F4.
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
            script,
            list(&["F1"], "Roles"),
        )]))
        .expect("the row above the moved rule");
    assert_eq!(
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_data_validation(
                script,
                list(&["F4"], "Roles"),
            )]))
            .expect_err("the moved rule's last row"),
        ValidationError::DataValidationOverlapsUnmodeledRule {
            sheet_id: script.get(),
        }
    );
    let xml = part_text(&minimal_write(&draft), "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"<dataValidation type="bogus" sqref="F2:F4"/>"#),
        "{xml}"
    );
}

#[test]
fn rules_another_tool_wrote_come_back_with_the_inverse() {
    let source = generated_lenient_validation_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let rules_sheet = |draft: &WorkbookDraft| {
        draft
            .workbook()
            .sheet_by_name("Rules")
            .expect("Rules")
            .data_validations()
            .to_vec()
    };
    for edit in [
        |sheet| WorkbookChange::remove_data_validation(sheet, 0),
        |sheet| WorkbookChange::clear_data_validations(sheet, ranges(&["A1:C1"])),
        |sheet| WorkbookChange::set_data_validation(sheet, list(&["Z1"], "\"x\"")),
    ] {
        let mut draft = WorkbookDraft::from_document(&document);
        let original = rules_sheet(&draft);
        assert_eq!(original.len(), 2);
        let sheet = draft.workbook().sheet_by_name("Rules").expect("Rules").id();
        let receipt = draft
            .apply_changes(EditBatch::new([edit(sheet)]))
            .expect("edit");
        let inverse = receipt.inverse().expect("an inverse").clone();
        draft
            .apply_changes(EditBatch::new(inverse.changes().to_vec()))
            .expect("undo");
        assert_eq!(rules_sheet(&draft), original);
        let sheet_part = |bytes: &[u8]| archive_parts(bytes)["xl/worksheets/sheet1.xml"].clone();
        assert_eq!(sheet_part(&minimal_write(&draft)), sheet_part(&source));
    }
}
