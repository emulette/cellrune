use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CellAddress, CellValue,
    ExcelError, FormulaText, WorkbookDraft, calculate_workbook,
};

use super::{assert_materialized_number, materialized_result};

fn spill_workbook(cells: &[(&str, &str)]) -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in cells {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(*formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }
    draft
}

#[test]
fn frequency_counts_numbers_through_each_ascending_bin_as_a_column() {
    let draft = spill_workbook(&[
        ("A1", "={79;85;78;85;50;81;95;88;97}"),
        ("B1", "=FREQUENCY(A1:A9,{70;79;89})"),
        // Horizontal bins still produce one column.
        ("C1", "=FREQUENCY({1,2,2,3,-1},{0,2})"),
        // Text and blanks are ignored in the data, and bins without numbers count everything.
        ("D1", "=FREQUENCY({1,\"x\",3},{\"y\"})"),
        ("E1", "=FREQUENCY(\"text\",{1;2})"),
    ]);
    let sheet_id = draft.workbook().sheets()[0].id();
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("B1", 1.0),
        ("B2", 2.0),
        ("B3", 4.0),
        ("B4", 2.0),
        ("C1", 1.0),
        ("C2", 3.0),
        ("C3", 1.0),
        ("D1", 2.0),
        ("E1", 0.0),
        ("E2", 0.0),
        ("E3", 0.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    assert_eq!(materialized_result(&calculation, sheet_id, "B5"), None);
    assert_eq!(materialized_result(&calculation, sheet_id, "C4"), None);
    assert_eq!(materialized_result(&calculation, sheet_id, "D2"), None);
    // A repeated bin is an empty interval.
    let ascending_with_ties = calculate_workbook(
        spill_workbook(&[("A1", "=FREQUENCY({-1,0,1,2,2,3},{0,2,2})")]).workbook(),
        CalculationOptions::default(),
    );
    for (address, expected) in [("A1", 2.0), ("A2", 3.0), ("A3", 0.0), ("A4", 1.0)] {
        assert_materialized_number(&ascending_with_ties, sheet_id, address, expected);
    }
}

#[test]
fn frequency_follows_excel_for_unordered_and_text_bins_logical_values_and_errors() {
    let draft = spill_workbook(&[
        // Excel 16.114 for Mac keeps the given bin order: 2 numbers at or below 2, none at or
        // below 0, and 1 above.
        ("A1", "=FREQUENCY({1,2,3},{2,0})"),
        // A text bin is left out of the counts and of the result.
        ("B1", "=FREQUENCY({1,2,3},{1,\"x\",3})"),
        // A logical value in a data array constant is ignored.
        ("C1", "=FREQUENCY({1,TRUE,3},{2})"),
        // An error in the data is the whole result.
        ("D1", "=FREQUENCY({1,#N/A,3},{2})"),
        // A blank bin cell is left out too, so three rows for Z1:Z3.
        ("X1", "={1;2;3}"),
        ("Z1", "=1"),
        ("Z2", "=2"),
        ("E1", "=FREQUENCY(X1:X3,Z1:Z3)"),
        // Distinct bins in any order.
        ("F1", "=FREQUENCY({5,1,3,3,9},{4,0,8,2})"),
    ]);
    let sheet_id = draft.workbook().sheets()[0].id();
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 2.0),
        ("A2", 0.0),
        ("A3", 1.0),
        ("B1", 1.0),
        ("B2", 2.0),
        ("B3", 0.0),
        ("C1", 1.0),
        ("C2", 1.0),
        ("E1", 1.0),
        ("E2", 1.0),
        ("E3", 1.0),
        ("F1", 2.0),
        ("F2", 0.0),
        ("F3", 1.0),
        ("F4", 1.0),
        ("F5", 1.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for address in ["A4", "B4", "C3", "D2", "E4", "F6"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            None,
            "{address}"
        );
    }
    assert_eq!(
        materialized_result(&calculation, sheet_id, "D1"),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        ))),
    );
}

#[test]
fn frequency_leaves_unmeasured_inputs_unsupported() {
    for cells in [
        // Equal bins out of ascending order: which row gets the count is unknown.
        &[("A1", "=FREQUENCY({1,2,3},{2,0,2})")][..],
        // Errors in the bins, and different errors in the data.
        &[("A1", "=FREQUENCY({1,2,3},{#N/A})")][..],
        &[("A1", "=FREQUENCY({1,#N/A,#DIV/0!},{2})")][..],
        // Logical values outside a data array constant, and logical bins.
        &[("B1", "=TRUE"), ("A1", "=FREQUENCY(B1:B2,{2})")][..],
        &[("A1", "=FREQUENCY({1,2,3},{TRUE})")][..],
    ] {
        let draft = spill_workbook(cells);
        let sheet_id = draft.workbook().sheets()[0].id();
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        assert!(
            matches!(
                materialized_result(&calculation, sheet_id, "A1"),
                Some(CalculationCellResult::Unavailable(issue))
                    if issue.code() == CalculationIssueCode::UnsupportedExpression
            ),
            "{cells:?}",
        );
    }
}

#[test]
fn mode_mult_spills_every_most_frequent_number_in_first_occurrence_order() {
    let draft = spill_workbook(&[
        // Microsoft's example: 1, 2, and 3 each appear three times.
        ("A1", "=MODE.MULT({1,2,3,4,3,2,1,2,3,5,6,1})"),
        ("B1", "=MODE.MULT({4,4,1,1,2,9})"),
        ("C1", "=MODE.MULT({5,7,7,5},{\"x\",TRUE},8)"),
        ("D1", "=MODE.MULT({1,2,3})"),
        ("E1", "=MODE.SNGL({4,4,1,1,2,9})"),
        // Excel 16.114 for Mac returns 3, then 1.
        ("F1", "=MODE.MULT({3,3,1,1})"),
    ]);
    let sheet_id = draft.workbook().sheets()[0].id();
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 1.0),
        ("A2", 2.0),
        ("A3", 3.0),
        ("B1", 4.0),
        ("B2", 1.0),
        ("C1", 5.0),
        ("C2", 7.0),
        ("E1", 4.0),
        ("F1", 3.0),
        ("F2", 1.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    assert_eq!(materialized_result(&calculation, sheet_id, "A4"), None);
    assert_eq!(materialized_result(&calculation, sheet_id, "F3"), None);
    assert_eq!(materialized_result(&calculation, sheet_id, "B3"), None);
    assert_eq!(
        materialized_result(&calculation, sheet_id, "D1"),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        ))),
    );
}

#[test]
fn mode_counts_negative_zero_as_zero() {
    let draft = spill_workbook(&[
        ("A1", "=MODE.SNGL(0,-0,1)"),
        ("B1", "=MODE.MULT(-0,0,2,2)"),
        ("C1", "=MODE(1,-0,0)"),
    ]);
    let sheet_id = draft.workbook().sheets()[0].id();
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [("A1", 0.0), ("B1", 0.0), ("B2", 2.0), ("C1", 0.0)] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
}
