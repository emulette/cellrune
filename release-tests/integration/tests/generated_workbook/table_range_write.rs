use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellRange,
    CellValue, EditBatch, OpenOptions, RecalculationWriteOptions, TableId, WorkbookChange,
    WorkbookDraft, calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

use super::recalculation_write_support as support;
use crate::support::table_structure_xlsx::generated_table_structure_fixture;

fn range(value: &str) -> CellRange {
    let (start, end) = value.split_once(':').expect("range");
    CellRange::new(
        CellAddress::from_a1(start).expect("start"),
        CellAddress::from_a1(end).expect("end"),
    )
    .expect("range")
}

fn changed(target: &str, extra_header: bool) -> (WorkbookDraft, Vec<u8>) {
    let source = generated_table_structure_fixture();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    if extra_header {
        let s2 = draft.workbook().sheet_by_name("S2").expect("S2").id();
        draft
            .set_cell_value(
                s2,
                CellAddress::from_a1("F1").expect("F1"),
                CellValue::Text("extra".to_owned()),
            )
            .expect("header");
    }
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_table_range(
            TableId::new(1).expect("table ID"),
            range(target),
        )]))
        .expect("range change");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write")
        .bytes()
        .to_vec();
    (draft, output)
}

fn results(
    calculation: &cellrune::CalculationSnapshot,
) -> Vec<(CalculationCellId, CalculationCellResult)> {
    calculation
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect()
}

#[test]
fn removing_a_table_column_saves_ref_errors_and_drops_its_table_column() {
    let (draft, output) = changed("C1:D5", false);
    let table = support::part_text(&output, "xl/tables/table1.xml");
    assert!(table.contains(r#"ref="C1:D5""#), "{table}");
    assert!(table.contains(r#"<tableColumns count="2">"#), "{table}");
    assert!(!table.contains(r#"name="dbl""#), "{table}");
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    // Excel 16.114 turns a reference ending on the removed column into `#REF!`.
    for (address, expected) in [
        ("A1", "SUM(Things[val])"),
        ("A2", "SUM(#REF!)"),
        ("A6", "SUM(#REF!)"),
    ] {
        assert_eq!(
            support::formula_at(&reopened, "S1", address)
                .text()
                .expect("text")
                .as_str(),
            expected,
            "S1!{address}"
        );
    }
    let names = reopened
        .workbook()
        .table("Things")
        .expect("Things")
        .columns()
        .iter()
        .map(|column| column.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["key", "val"]);
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    let current = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(results(&fresh), results(&current));
}

#[test]
fn a_table_grows_by_a_column_and_a_row_in_the_saved_part() {
    let (draft, output) = changed("C1:F6", true);
    let table = support::part_text(&output, "xl/tables/table1.xml");
    assert!(table.contains(r#"ref="C1:F6""#), "{table}");
    assert!(table.contains(r#"name="extra""#), "{table}");
    let reopened = open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopen");
    let things = reopened.workbook().table("Things").expect("Things");
    assert_eq!(things.range(), range("C1:F6"));
    let fresh = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    let current = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_eq!(results(&fresh), results(&current));
}
