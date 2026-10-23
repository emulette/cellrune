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
fn frequency_leaves_undocumented_inputs_unsupported() {
    for formula in [
        "=FREQUENCY({1,2,3},{2,0})",
        "=FREQUENCY({1,2,3},{1,\"x\",3})",
        "=FREQUENCY({1,TRUE,3},{2})",
        "=FREQUENCY({1,#N/A,3},{2})",
        "=FREQUENCY({1,2,3},{#N/A})",
    ] {
        let draft = spill_workbook(&[("A1", formula)]);
        let sheet_id = draft.workbook().sheets()[0].id();
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        assert!(
            matches!(
                materialized_result(&calculation, sheet_id, "A1"),
                Some(CalculationCellResult::Unavailable(issue))
                    if issue.code() == CalculationIssueCode::UnsupportedExpression
            ),
            "{formula}",
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
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    assert_eq!(materialized_result(&calculation, sheet_id, "A4"), None);
    assert_eq!(materialized_result(&calculation, sheet_id, "B3"), None);
    assert_eq!(
        materialized_result(&calculation, sheet_id, "D1"),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        ))),
    );
}
