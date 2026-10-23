use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, EditBatch, OpenOptions,
    RecalculationWriteOptions, TableId, WorkbookChange, WorkbookDraft, calculate_workbook,
    open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::table_structure_xlsx::generated_table_structure_fixture;

fn results(
    calculation: &cellrune::CalculationSnapshot,
) -> Vec<(CalculationCellId, CalculationCellResult)> {
    calculation
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect()
}

#[test]
fn converting_a_table_removes_its_part_and_saves_absolute_references() {
    let source = generated_table_structure_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let before = calculate_workbook(document.workbook(), CalculationOptions::default());
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::convert_table_to_range(
            TableId::new(1).expect("table ID"),
        )]))
        .expect("convert Things");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(results(&calculation), results(&before));
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write");
    let parts = support::archive_parts(output.bytes());
    assert!(!parts.contains_key("xl/tables/table1.xml"));
    let text = |part: &str| String::from_utf8(parts[part].clone()).expect("UTF-8");
    let content_types = text("[Content_Types].xml");
    assert!(!content_types.contains("table1.xml"), "{content_types}");
    let s2 = text("xl/worksheets/sheet2.xml");
    assert!(!s2.contains("tablePart"), "{s2}");
    assert!(!text("xl/worksheets/_rels/sheet2.xml.rels").contains("table1.xml"));

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert!(reopened.workbook().table("Things").is_none());
    // Excel 16.114's converted forms, with the calculated column's rows relative.
    for (sheet, address, expected) in [
        ("S1", "A1", "SUM('S2'!$D$2:$D$4)"),
        ("S1", "A2", "SUM('S2'!$D$2:$E$4)"),
        ("S1", "A3", "'S2'!$D$5"),
        ("S1", "A4", "ROWS('S2'!$C$1:$E$5)"),
        ("S2", "E2", "'S2'!$D2*2"),
        ("S2", "E3", "'S2'!$D3*2"),
        ("S2", "E4", "'S2'!$D4*2"),
        ("S2", "D5", "SUBTOTAL(109,'S2'!$D$2:$D$4)"),
    ] {
        let formula = support::formula_at(&reopened, sheet, address);
        assert_eq!(
            formula.text().expect("text").as_str(),
            expected,
            "{sheet}!{address}"
        );
    }
    assert_eq!(
        reopened.workbook().defined_names()[0].formula().as_str(),
        "'S2'!$D$2:$D$4"
    );
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    assert_eq!(results(&fresh), results(&before));
}
