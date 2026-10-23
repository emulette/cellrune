use cellrune::{
    CellRange, DataValidation, DataValidationErrorStyle, DataValidationOperator,
    DataValidationType, EditBatch, OpenOptions, ReadOptions, SheetName, WorkbookChange,
    WorkbookDraft, WriteOptions, open_xlsx_document_bytes, read_xlsx_bytes,
    write_xlsx_draft_edits_bytes,
};

use crate::support::generated_xlsx::{
    generated_data_validation_workbook, generated_sheet_reference_fixture,
};

fn ranges(validation: &DataValidation) -> Vec<String> {
    validation
        .ranges()
        .iter()
        .map(|range: &CellRange| {
            if range.start() == range.end() {
                range.start().to_string()
            } else {
                format!("{}:{}", range.start(), range.end())
            }
        })
        .collect()
}

#[test]
fn reads_every_data_validation_type_and_the_x14_extension() {
    let source = generated_data_validation_workbook();
    let snapshot = read_xlsx_bytes(&source, ReadOptions::default()).expect("read");
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    for workbook in [&snapshot, document.workbook()] {
        let validations = workbook
            .sheet_by_name("Rules")
            .expect("Rules")
            .data_validations();
        let summary = validations
            .iter()
            .map(|validation| {
                (
                    ranges(validation),
                    validation.validation_type(),
                    validation.operator(),
                    validation.formula1(),
                    validation.formula2(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (
                    vec!["A1:A10".to_owned(), "C1".to_owned()],
                    DataValidationType::List,
                    DataValidationOperator::Between,
                    Some("\"R&D,Ops\""),
                    None,
                ),
                (
                    vec!["B1".to_owned()],
                    DataValidationType::Whole,
                    DataValidationOperator::Between,
                    Some("1"),
                    Some("10"),
                ),
                (
                    vec!["B2".to_owned()],
                    DataValidationType::Decimal,
                    DataValidationOperator::GreaterThanOrEqual,
                    Some("0.5"),
                    None,
                ),
                (
                    vec!["B3".to_owned()],
                    DataValidationType::Date,
                    DataValidationOperator::LessThan,
                    Some("DATE(2026,1,1)"),
                    None,
                ),
                (
                    vec!["B4".to_owned()],
                    DataValidationType::Time,
                    DataValidationOperator::NotBetween,
                    Some("0.25"),
                    Some("0.75"),
                ),
                (
                    vec!["B5".to_owned()],
                    DataValidationType::TextLength,
                    DataValidationOperator::LessThanOrEqual,
                    Some("5"),
                    None,
                ),
                (
                    vec!["B6".to_owned()],
                    DataValidationType::Custom,
                    DataValidationOperator::Between,
                    Some("ISNUMBER(B6)"),
                    None,
                ),
                (
                    vec!["B7".to_owned()],
                    DataValidationType::AnyValue,
                    DataValidationOperator::Between,
                    None,
                    None,
                ),
                (
                    vec!["B8".to_owned()],
                    DataValidationType::List,
                    DataValidationOperator::Between,
                    Some("$D$1:$D$3"),
                    None,
                ),
                (
                    vec!["H1".to_owned()],
                    DataValidationType::Custom,
                    DataValidationOperator::Between,
                    Some("Lists!A1:"),
                    None,
                ),
                (
                    vec!["E1:E5".to_owned(), "G2".to_owned()],
                    DataValidationType::List,
                    DataValidationOperator::Between,
                    Some("Lists!$A$1:$A$3"),
                    None,
                ),
            ],
            "the five malformed rules after H1 are left unmodeled"
        );

        let list = &validations[0];
        assert!(list.allow_blank());
        assert!(list.in_cell_dropdown());
        assert!(list.show_input_message());
        assert!(list.show_error_message());
        assert_eq!(list.error_style(), DataValidationErrorStyle::Stop);
        assert_eq!(list.prompt_title(), Some("Pick"));
        assert_eq!(list.prompt_message(), Some("Choose a team"));
        assert_eq!(list.error_title(), Some("Unknown"));
        assert_eq!(list.error_message(), Some("Pick from the list"));

        let whole = &validations[1];
        assert!(!whole.allow_blank());
        assert!(!whole.show_input_message());
        assert!(!whole.show_error_message());
        assert_eq!(whole.prompt_message(), None);
        assert_eq!(
            validations[2].error_style(),
            DataValidationErrorStyle::Warning
        );
        assert_eq!(
            validations[3].error_style(),
            DataValidationErrorStyle::Information
        );
        assert!(validations[7].show_input_message());
        assert_eq!(validations[7].prompt_message(), Some("Any value"));
        assert!(
            !validations[8].in_cell_dropdown(),
            "showDropDown=\"1\" hides the in-cell dropdown"
        );

        assert!(
            workbook
                .sheet_by_name("Lists")
                .expect("Lists")
                .data_validations()
                .is_empty()
        );
    }
}

#[test]
fn sheet_rename_rewrites_data_validation_formulas_as_the_saved_part_does() {
    let source = generated_sheet_reference_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let data = draft.workbook().sheet_by_name("Data").expect("Data").id();
    draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_sheet(
            data,
            SheetName::new("New Data").expect("sheet name"),
        )]))
        .expect("rename");

    let formulas = |validations: &[DataValidation]| {
        validations
            .iter()
            .map(|validation| (ranges(validation), validation.formula1().map(str::to_owned)))
            .collect::<Vec<_>>()
    };
    let renamed = formulas(
        draft
            .workbook()
            .sheet_by_name("Summary")
            .expect("Summary")
            .data_validations(),
    );
    assert_eq!(
        renamed,
        [
            (
                vec!["B1".to_owned()],
                Some("'New Data'!$A$1:$A$3".to_owned())
            ),
            (
                vec!["D1".to_owned()],
                Some("'New Data'!$B$1:$B$3".to_owned())
            ),
        ]
    );

    let saved = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("edits write")
        .into_bytes();
    let reopened = open_xlsx_document_bytes(&saved, OpenOptions::default()).expect("reopen");
    assert_eq!(
        formulas(
            reopened
                .workbook()
                .sheet_by_name("Summary")
                .expect("Summary")
                .data_validations()
        ),
        renamed,
        "the model carries the formulas the saved part holds"
    );
}

#[test]
fn sheet_rename_keeps_a_validation_formula_naming_it_that_does_not_parse() {
    let source = generated_data_validation_workbook();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let lists = draft.workbook().sheet_by_name("Lists").expect("Lists").id();
    let rule = |draft: &WorkbookDraft| {
        draft.workbook().sheets()[0].data_validations()[9]
            .formula1()
            .map(str::to_owned)
    };
    let unparsable = rule(&draft);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::rename_sheet(
            lists,
            SheetName::new("Regions").expect("sheet name"),
        )]))
        .expect("a rename succeeds as it did before rules were modeled");
    assert!(draft.workbook().sheet_by_name("Regions").is_some());
    assert_eq!(
        rule(&draft),
        unparsable,
        "the H1 rule names Lists in a formula that does not parse and stays as it is"
    );
    let error = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect_err("saving still refuses the rule it cannot rewrite");
    assert_eq!(
        error.code().as_str(),
        "xlsx.write.unsupported_preservation",
        "{error}"
    );
}

#[test]
fn rule_and_hyperlink_rewrites_are_not_charged_to_the_session_rewrite_budget() {
    let source = generated_sheet_reference_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let draft = WorkbookDraft::from_document(&document);
    let rules = draft
        .workbook()
        .sheets()
        .iter()
        .map(|sheet| sheet.data_validations().len() + sheet.hyperlinks().len())
        .sum::<usize>();
    assert!(rules > 0);
    let cell_formulas = draft
        .workbook()
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter(|cell| matches!(cell.content(), cellrune::CellContent::Formula(_)))
        .count();
    let limits = cellrune::SessionLimits::default()
        .with_formula_rewrite_limits(cell_formulas.max(1), 1 << 24, 1 << 24, 1 << 24)
        .expect("limits");
    let mut session = cellrune::WorkbookCalculationSession::with_limits(draft, limits);
    let data = session.workbook().sheet_by_name("Data").expect("Data").id();
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(
            revision,
            EditBatch::new([WorkbookChange::rename_sheet(
                data,
                SheetName::new("New Data").expect("sheet name"),
            )]),
        )
        .expect("only cell formulas count toward the rewrite budget");
}

#[test]
fn whole_column_and_row_validations_read_and_survive_a_row_insertion() {
    use super::style_record_write::{edit_and_save, one_sheet_package, sheet1};

    const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><dataValidations count="3"><dataValidation type="whole" sqref="C:C"><formula1>1</formula1></dataValidation><dataValidation type="whole" sqref="$2:$3"><formula1>2</formula1></dataValidation><dataValidation type="whole" sqref="A1 B:D"><formula1>3</formula1></dataValidation></dataValidations><extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataValidations count="1" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidation type="list"><x14:formula1><xm:f>$A$1:$A$3</xm:f></x14:formula1><xm:sqref>E:E</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst></worksheet>"#;
    let read = |bytes: &[u8]| {
        let workbook = read_xlsx_bytes(bytes, ReadOptions::default()).expect("read");
        workbook
            .sheet_by_id(sheet1())
            .expect("Sheet1")
            .data_validations()
            .iter()
            .map(ranges)
            .collect::<Vec<_>>()
    };
    let source = one_sheet_package(SHEET, None);
    assert_eq!(
        read(&source),
        [
            vec!["C1:C1048576"],
            vec!["A2:XFD3"],
            vec!["A1", "B1:D1048576"],
            vec!["E1:E1048576"],
        ]
    );
    let output = edit_and_save(
        &source,
        vec![
            WorkbookChange::insert_rows(sheet1(), cellrune::Row::new(1).expect("row"), 1)
                .expect("insert"),
        ],
    );
    let worksheet =
        super::recalculation_write_support::part_text(&output, "xl/worksheets/sheet1.xml");
    assert!(
        worksheet.contains(r#"sqref="C:C""#) && worksheet.contains(r#"sqref="3:4""#),
        "{worksheet}"
    );
    assert_eq!(
        read(&output),
        [
            vec!["C1:C1048576"],
            vec!["A3:XFD4"],
            vec!["A2", "B1:D1048576"],
            vec!["E1:E1048576"],
        ]
    );
}
