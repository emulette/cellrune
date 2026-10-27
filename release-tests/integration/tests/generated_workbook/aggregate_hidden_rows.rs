use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CancellationToken, CellAddress,
    CellValue, EditBatch, ExcelError, OpenOptions, RecalculationMode, WorkbookCalculationSession,
    WorkbookChange, WorkbookDraft, calculate_workbook, open_xlsx_document_bytes,
};

use crate::support::generated_xlsx::{
    generated_condition_column_filter_workbook, generated_filtered_rows_workbook,
    generated_hidden_rows_workbook,
};

#[derive(Debug, Clone, Copy)]
enum Expected {
    Number(f64),
    Error(ExcelError),
}

#[test]
fn aggregate_skips_hidden_rows_errors_and_nested_totals_by_option() {
    let cases = [
        // Option 5 skips hidden rows only.
        ("_xlfn.AGGREGATE(9,5,A1:A3)", Expected::Number(40.0)),
        ("_xlfn.AGGREGATE(9,4,A1:A3)", Expected::Number(60.0)),
        // Option 6 skips errors but keeps the nested SUBTOTAL and the hidden rows.
        ("_xlfn.AGGREGATE(9,6,A1:A6)", Expected::Number(160.0)),
        // Option 3 skips hidden rows, errors, and nested totals.
        ("_xlfn.AGGREGATE(9,3,A1:A6)", Expected::Number(40.0)),
        (
            "_xlfn.AGGREGATE(9,0,A1:A6)",
            Expected::Error(ExcelError::DivisionByZero),
        ),
        ("_xlfn.AGGREGATE(14,6,A1:A6,2)", Expected::Number(40.0)),
        ("_xlfn.AGGREGATE(14,3,A1:A6,1)", Expected::Number(30.0)),
        // A calculated array keeps its hidden rows and nested totals.
        ("_xlfn.AGGREGATE(14,3,A1:A6*1,1)", Expected::Number(60.0)),
        ("_xlfn.AGGREGATE(12,1,A1:A3)", Expected::Number(20.0)),
        // Excel 16.114 keeps the rows a filter hides under options 0, 2, 4, and 6 and skips them
        // under the odd options (excel-measure RESULTS.md, section C).
        ("_xlfn.AGGREGATE(4,1,C11:C14)", Expected::Number(7.0)),
        ("_xlfn.AGGREGATE(4,4,C11:C14)", Expected::Number(50.0)),
        ("_xlfn.AGGREGATE(9,0,C11:C14)", Expected::Number(65.0)),
        ("_xlfn.AGGREGATE(9,7,C11:C14)", Expected::Number(15.0)),
        (
            "_xlfn.AGGREGATE(9,6,{1,2,3})",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(14,6,A1:A6)",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(20,6,A1:A3)",
            Expected::Error(ExcelError::Value),
        ),
        (
            "_xlfn.AGGREGATE(9,8,A1:A3)",
            Expected::Error(ExcelError::Value),
        ),
        ("_xlfn.AGGREGATE(16,6,A1:A6,0.5)", Expected::Number(30.0)),
        ("_xlfn.AGGREGATE(2,3,A1:A6)", Expected::Number(2.0)),
    ];
    let formulas = cases.map(|(formula, _)| formula);
    let document = open_xlsx_document_bytes(
        &generated_hidden_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let workbook = document.workbook();
    let sheet = &workbook.sheets()[0];
    let calculation = calculate_workbook(workbook, CalculationOptions::default());

    for (offset, (formula, expected)) in cases.iter().enumerate() {
        let cell = CalculationCellId::new(
            sheet.id(),
            CellAddress::from_indices(offset as u32 + 1, 5).expect("valid formula address"),
        );
        match (expected, calculation.cell(cell)) {
            (
                Expected::Number(expected),
                Some(CalculationCellResult::Value(CellValue::Number(actual))),
            ) => {
                assert!(
                    (actual.get() - expected).abs() < 1e-9,
                    "{formula}: {}",
                    actual.get()
                );
            }
            (
                Expected::Error(expected),
                Some(CalculationCellResult::Value(CellValue::Error(actual))),
            ) => {
                assert_eq!(actual, expected, "{formula}");
            }
            (_, actual) => panic!("{formula}: expected {expected:?}, got {actual:?}"),
        }
    }
}

/// The number each formula in E1 downward calculates to in `workbook`.
fn numbers(
    workbook: &cellrune::WorkbookSnapshot,
    calculation: &cellrune::CalculationSnapshot,
    count: usize,
) -> Vec<f64> {
    numbers_in(workbook, calculation, 5, count)
}

/// The number each formula in `column` from row 1 downward calculates to in `workbook`.
fn numbers_in(
    workbook: &cellrune::WorkbookSnapshot,
    calculation: &cellrune::CalculationSnapshot,
    column: u32,
    count: usize,
) -> Vec<f64> {
    let sheet = &workbook.sheets()[0];
    (0..count)
        .map(|offset| {
            let cell = CalculationCellId::new(
                sheet.id(),
                CellAddress::from_indices(offset as u32 + 1, column)
                    .expect("valid formula address"),
            );
            match calculation.cell(cell) {
                Some(CalculationCellResult::Value(CellValue::Number(number))) => number.get(),
                other => panic!("row {}: {other:?}", offset + 1),
            }
        })
        .collect()
}

#[test]
fn subtotal_skips_filtered_rows_and_101_to_111_also_rows_hidden_by_hand() {
    let cases = [
        // Rows 2 and 6 are hidden by hand, outside any filter.
        ("SUBTOTAL(9,A1:A3)", 60.0),
        ("SUBTOTAL(109,A1:A3)", 40.0),
        ("SUBTOTAL(102,A1:A3)", 2.0),
        // Row 12 is hidden inside an auto-filter with a condition, which every function skips.
        ("SUBTOTAL(9,C11:C14)", 15.0),
        ("SUBTOTAL(109,C11:C14)", 15.0),
        ("SUBTOTAL(4,C11:C14)", 7.0),
        ("SUBTOTAL(2,C11:C14)", 3.0),
    ];
    let formulas = cases.map(|(formula, _)| formula);
    let document = open_xlsx_document_bytes(
        &generated_hidden_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let workbook = document.workbook();
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    assert_eq!(
        numbers(workbook, &calculation, cases.len()),
        cases.map(|(_, expected)| expected)
    );
}

#[test]
fn subtotal_takes_a_table_filter_and_not_a_filter_without_a_condition() {
    let cases = [
        // Row 12 is hidden inside a worksheet auto-filter that declares no condition.
        ("SUBTOTAL(9,A11:A14)", 15.0),
        ("SUBTOTAL(109,A11:A14)", 13.0),
        // Row 22 is hidden inside a table whose auto-filter declares a condition.
        ("SUBTOTAL(9,A21:A24)", 13.0),
        ("SUBTOTAL(109,A21:A24)", 13.0),
        ("_xlfn.AGGREGATE(9,6,A21:A24)", 15.0),
        ("_xlfn.AGGREGATE(9,7,A21:A24)", 13.0),
        ("_xlfn.AGGREGATE(9,5,A21:A24)", 13.0),
    ];
    let formulas = cases.map(|(formula, _)| formula);
    let document = open_xlsx_document_bytes(
        &generated_filtered_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let workbook = document.workbook();
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    assert_eq!(
        numbers(workbook, &calculation, cases.len()),
        cases.map(|(_, expected)| expected)
    );
}

#[test]
fn rows_a_removed_filter_hid_stay_hidden_and_count_as_hidden_by_hand() {
    let formulas = ["SUBTOTAL(9,C11:C14)", "SUBTOTAL(109,C11:C14)"];
    let document = open_xlsx_document_bytes(
        &generated_hidden_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let mut session = WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let sheet_id = session.workbook().sheets()[0].id();
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_sheet_auto_filter(sheet_id, None)]),
        )
        .expect("filter removal");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("recalculation");
    let workbook = session.workbook();
    let calculation = session.calculation().expect("installed calculation");
    assert_eq!(numbers(workbook, calculation, 2), [65.0, 15.0]);
}

#[test]
fn every_subtotal_function_skips_filtered_rows_and_101_to_111_rows_hidden_by_hand() {
    // A1:A3 holds 10, 20 (hidden by hand), and 30; C11:C14 holds 5, 50 (filtered), 7, and 3.
    let shown = [10.0, 20.0, 30.0];
    let shown_without_hidden = [10.0, 30.0];
    let filtered = [5.0, 7.0, 3.0];
    let mut cases = Vec::new();
    for function in 1..=11 {
        cases.push((
            format!("SUBTOTAL({function},A1:A3)"),
            expected(function, &shown),
        ));
        cases.push((
            format!("SUBTOTAL({},A1:A3)", function + 100),
            expected(function, &shown_without_hidden),
        ));
        cases.push((
            format!("SUBTOTAL({function},C11:C14)"),
            expected(function, &filtered),
        ));
        cases.push((
            format!("SUBTOTAL({},C11:C14)", function + 100),
            expected(function, &filtered),
        ));
    }
    let formulas = cases
        .iter()
        .map(|(formula, _)| formula.as_str())
        .collect::<Vec<_>>();
    let document = open_xlsx_document_bytes(
        &generated_hidden_rows_workbook(&formulas),
        OpenOptions::default(),
    )
    .expect("generated workbook opens");
    let workbook = document.workbook();
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    for ((formula, expected), actual) in
        cases
            .iter()
            .zip(numbers(workbook, &calculation, cases.len()))
    {
        assert!((actual - expected).abs() < 1e-9, "{formula}: {actual}");
    }
}

/// What `SUBTOTAL` function 1 to 11 returns over `values`.
fn expected(function: u32, values: &[f64]) -> f64 {
    let count = values.len() as f64;
    let mean = values.iter().sum::<f64>() / count;
    let squares = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>();
    match function {
        1 => mean,
        2 | 3 => count,
        4 => values.iter().copied().fold(f64::MIN, f64::max),
        5 => values.iter().copied().fold(f64::MAX, f64::min),
        6 => values.iter().product(),
        7 => (squares / (count - 1.0)).sqrt(),
        8 => (squares / count).sqrt(),
        9 => values.iter().sum(),
        10 => squares / (count - 1.0),
        11 => squares / count,
        _ => unreachable!("functions 1 to 11"),
    }
}

/// Applies `change` to a session over `bytes` after a first calculation, recalculates, and returns
/// the numbers in `column` from row 1 downward alongside those a full calculation of the edited
/// workbook gives.
fn numbers_after(
    bytes: &[u8],
    column: u32,
    count: usize,
    change: impl Fn(&cellrune::WorkbookSnapshot) -> WorkbookChange,
) -> (Vec<f64>, Vec<f64>) {
    let document =
        open_xlsx_document_bytes(bytes, OpenOptions::default()).expect("generated workbook opens");
    let mut session = WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation");
    let edit = change(session.workbook());
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([edit]),
        )
        .expect("edit");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("recalculation");
    let workbook = session.workbook();
    let full = calculate_workbook(workbook, CalculationOptions::default());
    (
        numbers_in(
            workbook,
            session.calculation().expect("installed calculation"),
            column,
            count,
        ),
        numbers_in(workbook, &full, column, count),
    )
}

#[test]
fn converting_a_filtered_table_to_a_range_counts_its_hidden_rows_as_hidden_by_hand() {
    let bytes = generated_filtered_rows_workbook(&[
        "SUBTOTAL(9,A21:A24)",
        "SUBTOTAL(109,A21:A24)",
        "_xlfn.AGGREGATE(9,5,A21:A24)",
    ]);
    let (session, full) = numbers_after(&bytes, 5, 3, |workbook| {
        WorkbookChange::convert_table_to_range(workbook.sheets()[0].tables()[0].id())
    });
    assert_eq!(session, full);
    assert_eq!(session, [15.0, 13.0, 13.0]);
}

#[test]
fn deleting_the_column_with_the_filter_condition_counts_the_hidden_rows_as_hidden_by_hand() {
    let bytes = generated_condition_column_filter_workbook(&["SUBTOTAL(9,B11:B14)"]);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("generated workbook opens");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    assert_eq!(numbers_in(document.workbook(), &calculation, 1, 1), [15.0]);
    let (session, full) = numbers_after(&bytes, 1, 1, |workbook| {
        let column = cellrune::Column::new(3).expect("column C");
        WorkbookChange::delete_columns(workbook.sheets()[0].id(), column, column)
            .expect("one column")
    });
    assert_eq!(session, full);
    assert_eq!(session, [65.0]);
}

#[test]
fn edits_that_move_or_resize_a_filter_recalculate_subtotal_as_a_full_calculation_does() {
    fn row(value: u32) -> cellrune::Row {
        cellrune::Row::new(value).expect("valid row")
    }
    let sheet_filter =
        generated_hidden_rows_workbook(&["SUBTOTAL(9,C11:C14)", "SUBTOTAL(109,C11:C14)"]);
    let table_filter =
        generated_filtered_rows_workbook(&["SUBTOTAL(9,A21:A24)", "SUBTOTAL(109,A21:A24)"]);
    let edits: [(&[u8], fn(&cellrune::WorkbookSnapshot) -> WorkbookChange); 4] = [
        (&sheet_filter, |workbook| {
            WorkbookChange::insert_rows(workbook.sheets()[0].id(), row(8), 2).expect("insertion")
        }),
        (&sheet_filter, |workbook| {
            WorkbookChange::move_rows(workbook.sheets()[0].id(), row(12), row(12), row(30))
                .expect("row move")
        }),
        (&table_filter, |workbook| {
            WorkbookChange::resize_table_rows(
                workbook.sheets()[0].tables()[0].id(),
                row(21),
                row(23),
            )
            .expect("table resize")
        }),
        (&table_filter, |workbook| {
            WorkbookChange::insert_rows(workbook.sheets()[0].id(), row(22), 1).expect("insertion")
        }),
    ];
    for (index, (bytes, change)) in edits.into_iter().enumerate() {
        let (session, full) = numbers_after(bytes, 5, 2, change);
        assert_eq!(session, full, "edit {index}");
    }
}
