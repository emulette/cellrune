use cellrune::{
    CalculationOptions, DefinedName, DefinedNameScope, FormulaText, RecalculationWriteOptions,
    SheetName, WorkbookDraft, XlsxDocument, calculate_workbook, open_xlsx_document_bytes,
    write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;

const WORKBOOK_PART: &str = "xl/workbook.xml";
const FILTER_DATABASE: &str = r#"<definedName name="_xlnm._FilterDatabase" localSheetId="1" hidden="1">Data!$A$1:$B$3</definedName>"#;
const TAX_RATE: &str = r#"<definedName name="TaxRate" comment="Statutory rate" description="Applied to sales" help="See policy" statusBar="Tax rate">Data!$B$1</definedName>"#;
const MACRO_RUN: &str = r#"<definedName name="MacroRun" function="1" vbProcedure="1" shortcutKey="m" customMenu="Run macro" functionGroupId="14" publishToServer="1" workbookParameter="1">Data!$A$1</definedName>"#;
const LEGACY_MACRO: &str =
    r#"<definedName name="LegacyMacro" function="1" xlm="1">Data!$A$2</definedName>"#;
const PLAIN: &str = r#"<definedName  name='Plain' >Sheet1!$A$1</definedName>"#;

#[test]
fn changing_one_defined_name_formula_keeps_every_source_element_and_attribute() {
    let document = source_document();
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_defined_name(workbook_name("TaxRate", "Data!$B$2"))
        .expect("change TaxRate");

    let output = write(&draft);

    assert_eq!(
        defined_names_block(&output),
        defined_names(&[
            FILTER_DATABASE,
            &TAX_RATE.replace("Data!$B$1", "Data!$B$2"),
            MACRO_RUN,
            LEGACY_MACRO,
            PLAIN,
        ])
    );
    assert_reopened_names(&output, &draft);
}

#[test]
fn changing_a_hidden_flag_rewrites_only_that_attribute() {
    let document = source_document();
    let data = sheet_id(&document, "Data");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_defined_name(
            DefinedName::new(
                "_xlnm._FilterDatabase",
                DefinedNameScope::Sheet(data),
                FormulaText::from_xlsx("Data!$A$1:$B$3").expect("formula"),
                false,
            )
            .expect("defined name"),
        )
        .expect("unhide the filter database name");
    draft
        .set_defined_name(
            DefinedName::new(
                "TaxRate",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("Data!$B$1").expect("formula"),
                true,
            )
            .expect("defined name"),
        )
        .expect("hide TaxRate");

    let output = write(&draft);

    assert_eq!(
        defined_names_block(&output),
        defined_names(&[
            &FILTER_DATABASE.replace(r#" hidden="1""#, ""),
            &TAX_RATE.replace(
                r#"statusBar="Tax rate""#,
                r#"statusBar="Tax rate" hidden="1""#
            ),
            MACRO_RUN,
            LEGACY_MACRO,
            PLAIN,
        ])
    );
    assert_reopened_names(&output, &draft);
}

#[test]
fn removing_and_adding_defined_names_keeps_the_remaining_source_elements() {
    let document = source_document();
    let mut draft = WorkbookDraft::from_document(&document);
    assert!(
        draft
            .remove_defined_name(DefinedNameScope::Workbook, "legacymacro")
            .expect("remove LegacyMacro")
    );
    draft
        .set_defined_name(workbook_name("Added", "Sheet1!$B$1"))
        .expect("add a name");

    let output = write(&draft);

    assert_eq!(
        defined_names_block(&output),
        defined_names(&[
            FILTER_DATABASE,
            TAX_RATE,
            MACRO_RUN,
            PLAIN,
            r#"<definedName name="Added">Sheet1!$B$1</definedName>"#,
        ])
    );
    assert_reopened_names(&output, &draft);
}

#[test]
fn renaming_a_referenced_sheet_rewrites_name_text_and_keeps_name_attributes() {
    let document = source_document();
    let data = sheet_id(&document, "Data");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .rename_sheet(data, SheetName::new("Ledger").expect("sheet name"))
        .expect("rename Data");

    let output = write(&draft);

    assert_eq!(
        defined_names_block(&output),
        defined_names(&[
            &FILTER_DATABASE.replace("Data!", "Ledger!"),
            &TAX_RATE.replace("Data!", "Ledger!"),
            &MACRO_RUN.replace("Data!", "Ledger!"),
            &LEGACY_MACRO.replace("Data!", "Ledger!"),
            PLAIN,
        ])
    );
    assert_reopened_names(&output, &draft);
}

fn source_document() -> XlsxDocument {
    let mut seed = WorkbookDraft::new();
    seed.add_sheet(SheetName::new("Data").expect("sheet name"))
        .expect("add Data");
    let calculation = calculate_workbook(seed.workbook(), CalculationOptions::default());
    let seed_output =
        write_xlsx_draft_bytes(&seed, &calculation, RecalculationWriteOptions::default())
            .expect("seed output");
    let block = defined_names(&[FILTER_DATABASE, TAX_RATE, MACRO_RUN, LEGACY_MACRO, PLAIN]);
    let source_bytes = support::replace_part_text(
        seed_output.bytes(),
        WORKBOOK_PART,
        "</sheets>",
        &format!("</sheets>{block}"),
    );
    open_xlsx_document_bytes(&source_bytes, cellrune::OpenOptions::default())
        .expect("open source document")
}

fn sheet_id(document: &XlsxDocument, name: &str) -> cellrune::SheetId {
    document.workbook().sheet_by_name(name).expect("sheet").id()
}

fn workbook_name(name: &str, formula: &str) -> DefinedName {
    DefinedName::new(
        name,
        DefinedNameScope::Workbook,
        FormulaText::from_xlsx(formula).expect("formula"),
        false,
    )
    .expect("defined name")
}

fn write(draft: &WorkbookDraft) -> Vec<u8> {
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    write_xlsx_draft_bytes(draft, &calculation, RecalculationWriteOptions::default())
        .expect("document draft output")
        .bytes()
        .to_vec()
}

fn defined_names(elements: &[&str]) -> String {
    format!("<definedNames>{}</definedNames>", elements.concat())
}

fn defined_names_block(output: &[u8]) -> String {
    let workbook = support::part_text(output, WORKBOOK_PART);
    let start = workbook.find("<definedNames>").expect("definedNames start");
    let end = workbook.find("</definedNames>").expect("definedNames end");
    workbook[start..end + "</definedNames>".len()].to_owned()
}

fn assert_reopened_names(output: &[u8], draft: &WorkbookDraft) {
    let reopened =
        open_xlsx_document_bytes(output, cellrune::OpenOptions::default()).expect("reopen output");
    assert_eq!(
        reopened.workbook().defined_names(),
        draft.workbook().defined_names()
    );
}
