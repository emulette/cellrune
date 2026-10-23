use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellContent,
    CellValue, ExcelError, OpenOptions, RecalculationWriteOptions, RecalculationWritePolicy,
    SavedResult, WorkbookDraft, XlsxDocument, XlsxWriteErrorCode, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use crate::support::generated_xlsx::generated_package;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/metadata.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><calcPr calcId="191029"/></workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/>
</Relationships>"#;

const METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="dynamic-array"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;

/// B1:B2 doubles A1:A2 = 1, 3 as the legacy array `{=A1:A2*2}`, saved as 2 and 6.
const LEGACY_DOUBLES: &str = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="array" ref="B1:B2">A1:A2*2</f><v>2</v></c></row><row r="2"><c r="A2"><v>3</v></c><c r="B2"><v>6</v></c></row>"#;

/// The same result as a dynamic-array formula spilling over B1:B2.
const SPILLED_DOUBLES: &str = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" cm="1"><f t="array" ref="B1:B2">A1:A2*2</f><v>2</v></c></row><row r="2"><c r="A2"><v>3</v></c><c r="B2"><v>6</v></c></row>"#;

#[test]
fn an_input_edit_saves_new_legacy_array_followers() {
    assert_doubles_follow_an_input_edit(LEGACY_DOUBLES);
}

#[test]
fn an_input_edit_saves_new_dynamic_spill_followers() {
    assert_doubles_follow_an_input_edit(SPILLED_DOUBLES);
}

#[test]
fn legacy_array_results_that_grow_or_shrink_save_every_follower() {
    // B1:B4 = {=SEQUENCE(A1)*10} with A1 = 3; the cell the result does not reach holds #N/A.
    let document = open(
        r#"<row r="1"><c r="A1"><v>3</v></c><c r="B1"><f t="array" ref="B1:B4">_xlfn.SEQUENCE(A1)*10</f><v>10</v></c></row><row r="2"><c r="B2"><v>20</v></c></row><row r="3"><c r="B3"><v>30</v></c></row><row r="4"><c r="B4" t="e"><v>#N/A</v></c></row>"#,
    );
    let not_available = || CellValue::Error(ExcelError::NotAvailable);
    for (rows, followers) in [
        (4.0, [number(20.0), number(30.0), number(40.0)]),
        (2.0, [number(20.0), not_available(), not_available()]),
    ] {
        let reopened = save_calculated(
            &document,
            ("A1", rows),
            RecalculationWritePolicy::RequireComplete,
        )
        .expect("save the resized result");
        for (cell, value) in ["B2", "B3", "B4"].into_iter().zip(followers) {
            assert_eq!(
                content(&reopened, cell),
                Some(CellContent::Literal(value)),
                "A1 = {rows}, {cell}"
            );
        }
    }
}

#[test]
fn unavailable_array_followers_are_removed_when_an_edit_is_saved() {
    let document = open(
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="array" ref="B1:B2">CELLRUNE_UNKNOWN(A1:A2)</f><v>1</v></c></row><row r="2"><c r="A2"><v>3</v></c><c r="B2"><v>3</v></c></row>"#,
    );
    let reopened = save_calculated(
        &document,
        ("A2", 5.0),
        RecalculationWritePolicy::InvalidateUnavailable,
    )
    .expect("save the invalidated array");
    let Some(CellContent::Formula(anchor)) = content(&reopened, "B1") else {
        panic!("the array anchor remains a formula");
    };
    assert_eq!(anchor.saved_result(), &SavedResult::Missing);
    assert_eq!(content(&reopened, "B2"), None);
}

#[test]
fn a_value_typed_into_a_dynamic_spill_is_not_saved_over() {
    // Calculation reads every literal inside a declared spill as one of its saved followers, so
    // the spill still covers the typed B2. The save fails rather than replace the typed value.
    let document = open(SPILLED_DOUBLES);
    assert_eq!(
        save_calculated(
            &document,
            ("B2", 7.0),
            RecalculationWritePolicy::RequireComplete
        )
        .map(|_| ())
        .expect_err("the spill result must not replace a typed value"),
        XlsxWriteErrorCode::OutputVerificationFailed
    );
}

fn assert_doubles_follow_an_input_edit(cells: &str) {
    let document = open(cells);
    let reopened = save_calculated(
        &document,
        ("A2", 5.0),
        RecalculationWritePolicy::RequireComplete,
    )
    .expect("save the edited input");
    let Some(CellContent::Formula(anchor)) = content(&reopened, "B1") else {
        panic!("the array anchor remains a formula");
    };
    assert_eq!(anchor.saved_result(), &SavedResult::Present(number(2.0)));
    assert_eq!(
        content(&reopened, "B2"),
        Some(CellContent::Literal(number(10.0)))
    );

    let recalculation = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    let follower = CalculationCellId::new(reopened.workbook().sheets()[0].id(), address("B2"));
    assert_eq!(
        recalculation
            .materialized_cell(follower)
            .map(|cell| cell.result()),
        Some(&CalculationCellResult::Value(number(10.0))),
        "the reopened follower still belongs to its array"
    );
}

/// Opens a one-sheet workbook whose `Data` sheet holds the given rows.
fn open(rows: &str) -> XlsxDocument {
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let bytes = generated_package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELATIONSHIPS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/metadata.xml", METADATA),
    ]);
    open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("open source workbook")
}

/// Sets one value in a draft of `document`, calculates the draft, saves it under `policy`, and
/// reopens the output.
fn save_calculated(
    document: &XlsxDocument,
    (cell, value): (&str, f64),
    policy: RecalculationWritePolicy,
) -> Result<XlsxDocument, XlsxWriteErrorCode> {
    let mut draft = WorkbookDraft::from_document(document);
    draft
        .set_cell_value(
            document.workbook().sheets()[0].id(),
            address(cell),
            number(value),
        )
        .expect("edit the draft");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        &draft,
        &calculation,
        RecalculationWriteOptions::default().with_policy(policy),
    )
    .map_err(|error| error.code())?;
    Ok(open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output"))
}

fn content(document: &XlsxDocument, cell: &str) -> Option<CellContent> {
    document.workbook().sheets()[0]
        .cell(address(cell))
        .map(|cell| cell.content().clone())
}

fn address(text: &str) -> CellAddress {
    CellAddress::from_a1(text).expect("valid address")
}

fn number(value: f64) -> CellValue {
    CellValue::number(value).expect("finite value")
}

#[test]
fn a_canonical_dynamic_array_without_a_declared_range_reopens_with_its_spill() {
    use cellrune::{EditBatch, FormulaText, SheetId, WorkbookChange};

    let sheet = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::new();
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_dynamic_formula(
                sheet,
                CellAddress::from_a1("A1").expect("address"),
                FormulaText::from_xlsx("SEQUENCE(3)").expect("formula"),
                None,
            )
            .expect("dynamic formula"),
            WorkbookChange::set_cell_formula(
                sheet,
                CellAddress::from_a1("C1").expect("address"),
                FormulaText::from_xlsx("SUM(A1#)").expect("formula"),
            ),
        ]))
        .expect("formulas");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let bytes = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("canonical save")
        .into_bytes();
    let worksheet =
        super::recalculation_write_support::part_text(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        worksheet.contains(r#"<f t="array" ref="A1:A3">SEQUENCE(3)</f>"#),
        "{worksheet}"
    );

    let reopened = open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("reopen");
    let recalculated = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    let value = |address: &str| {
        recalculated
            .materialized_cell(CalculationCellId::new(
                sheet,
                CellAddress::from_a1(address).expect("address"),
            ))
            .map(|cell| cell.result().clone())
    };
    for (address, expected) in [("A1", 1.0), ("A2", 2.0), ("A3", 3.0), ("C1", 6.0)] {
        assert_eq!(
            value(address),
            Some(CalculationCellResult::Value(
                CellValue::number(expected).expect("number")
            )),
            "{address}"
        );
    }
    let again = WorkbookDraft::from_document(&reopened);
    write_xlsx_draft_bytes(&again, &recalculated, RecalculationWriteOptions::default())
        .expect("the reopened workbook saves again");
}
