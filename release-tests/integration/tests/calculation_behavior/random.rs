use std::collections::BTreeSet;

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationOptions,
    CalculationSnapshot, CalculationTarget, CancellationToken, CellAddress, CellValue, ExcelError,
    FormulaText, SheetId, TargetCalculationLimits, WorkbookDraft, WorkbookSnapshot,
    calculate_targets, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, cell_id, workbook_with_formulas};

fn seeded(seed: u32) -> CalculationOptions {
    CalculationOptions::default().with_random_seed(seed)
}

fn number_at(calculation: &CalculationSnapshot, id: CalculationCellId) -> f64 {
    match calculation.materialized_cell(id).map(|cell| cell.result()) {
        Some(CalculationCellResult::Value(CellValue::Number(number))) => number.get(),
        other => panic!("expected a number at {id:?}, got {other:?}"),
    }
}

fn column_number(calculation: &CalculationSnapshot, column: u32) -> f64 {
    number_at(calculation, cell_id(column))
}

fn first_sheet(draft: &WorkbookDraft) -> SheetId {
    draft.workbook().sheets()[0].id()
}

fn spilled_workbook(formulas: &[(&str, &str)]) -> WorkbookSnapshot {
    let mut draft = WorkbookDraft::new();
    let sheet_id = first_sheet(&draft);
    for (address, formula) in formulas {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(*formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }
    draft.workbook().clone()
}

fn area_numbers(
    calculation: &CalculationSnapshot,
    rows: std::ops::RangeInclusive<u32>,
    columns: std::ops::RangeInclusive<u32>,
) -> Vec<f64> {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    rows.flat_map(|row| columns.clone().map(move |column| (row, column)))
        .map(|(row, column)| {
            number_at(
                calculation,
                CalculationCellId::new(
                    sheet_id,
                    CellAddress::from_indices(row, column).expect("valid spill address"),
                ),
            )
        })
        .collect()
}

fn distinct(values: &[f64]) -> usize {
    values
        .iter()
        .map(|value| value.to_bits())
        .collect::<BTreeSet<_>>()
        .len()
}

#[test]
fn random_functions_report_a_missing_seed() {
    let workbook = super::workbook_with_formulas_and_names(
        &[
            (1, 1, "RAND()"),
            (1, 2, "RANDBETWEEN(1,6)"),
            (1, 3, "RANDARRAY(2,2)"),
            (1, 4, "IFERROR(RAND(),0)"),
            (1, 5, "draw"),
            (1, 6, "LET(f,LAMBDA(RAND()),f())"),
            (1, 7, "A1+1"),
        ],
        &[("draw", "RAND()")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=6 {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::VolatileInputMissing,
        );
    }
    assert_issue(&calculation, 7, CalculationIssueCode::BlockedByUpstream);
}

#[test]
fn a_seed_fixes_every_draw_and_a_new_seed_redraws() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "RAND()"),
        (1, 2, "RANDBETWEEN(1,1000000)"),
        (1, 3, "RANDARRAY(1,1,10,20)"),
        (1, 4, "A1*2"),
    ]);
    let first = calculate_workbook(&workbook, seeded(7));
    let again = calculate_workbook(&workbook, seeded(7));
    assert_eq!(
        first.cells().collect::<Vec<_>>(),
        again.cells().collect::<Vec<_>>()
    );
    assert_eq!(column_number(&first, 4), column_number(&first, 1) * 2.0);

    let redrawn = calculate_workbook(&workbook, seeded(8));
    for column in 1..=3 {
        assert_ne!(
            column_number(&first, column),
            column_number(&redrawn, column),
            "column {column} kept its value under a new seed",
        );
    }
}

#[test]
fn draws_depend_on_the_cell_not_on_other_cells_or_their_order() {
    let alone = workbook_with_formulas(&[(1, 3, "RAND()")]);
    let crowded = workbook_with_formulas(&[
        (1, 1, "RAND()"),
        (1, 2, "C1+RAND()"),
        (1, 3, "RAND()"),
        (1, 4, "RAND()+B1"),
    ]);
    let alone = calculate_workbook(&alone, seeded(11));
    let crowded = calculate_workbook(&crowded, seeded(11));
    assert_eq!(column_number(&alone, 3), column_number(&crowded, 3));
    assert_ne!(column_number(&crowded, 1), column_number(&crowded, 3));
}

#[test]
fn calls_in_one_cell_draw_independently_by_position() {
    let workbook = super::workbook_with_formulas_and_names(
        &[
            (1, 1, "RAND()-RAND()"),
            (1, 2, "draw-draw"),
            (1, 3, "LET(f,LAMBDA(RAND()),f()-f())"),
            (1, 4, "LET(x,RAND(),x-x)"),
            (1, 5, "SUM(RAND(),-RAND())"),
            (1, 6, "IF(RAND()<2,RAND())-RAND()"),
            (1, 7, "pick()-pick()"),
        ],
        &[("draw", "RAND()"), ("pick", "LAMBDA(RAND())")],
    );
    let calculation = calculate_workbook(&workbook, seeded(3));
    for column in [1, 2, 3, 5, 6, 7] {
        assert_ne!(column_number(&calculation, column), 0.0, "column {column}");
    }
    assert_eq!(column_number(&calculation, 4), 0.0);
}

#[test]
fn lambda_helpers_draw_once_per_iteration() {
    let workbook = spilled_workbook(&[
        ("A1", "=MAKEARRAY(3,3,LAMBDA(r,c,RAND()))"),
        ("E1", "=MAP(SEQUENCE(4),LAMBDA(v,RAND()))"),
        ("G1", "=BYROW(SEQUENCE(4,2),LAMBDA(r,RAND()))"),
        ("I1", "=BYCOL(SEQUENCE(2,4),LAMBDA(c,RAND()))"),
        ("A6", "=SCAN(0,SEQUENCE(1,4),LAMBDA(a,v,RAND()))"),
        ("A8", "=REDUCE(0,SEQUENCE(4),LAMBDA(a,v,a+RAND()))"),
        ("B8", "=REDUCE(0,SEQUENCE(4),LAMBDA(a,v,RAND()))"),
    ]);
    let calculation = calculate_workbook(&workbook, seeded(5));
    assert_eq!(distinct(&area_numbers(&calculation, 1..=3, 1..=3)), 9);
    assert_eq!(distinct(&area_numbers(&calculation, 1..=4, 5..=5)), 4);
    assert_eq!(distinct(&area_numbers(&calculation, 1..=4, 7..=7)), 4);
    assert_eq!(distinct(&area_numbers(&calculation, 1..=1, 9..=12)), 4);
    assert_eq!(distinct(&area_numbers(&calculation, 6..=6, 1..=4)), 4);
    let total = area_numbers(&calculation, 8..=8, 1..=1)[0];
    let last = area_numbers(&calculation, 8..=8, 2..=2)[0];
    assert!(total > last && total < 4.0, "{total} {last}");
}

#[test]
fn draws_stay_in_their_documented_ranges() {
    let workbook = spilled_workbook(&[
        ("A1", "=RANDARRAY(100,100)"),
        ("CW1", "=RANDARRAY(100,20,-3,3,TRUE)"),
        ("DQ1", "=RANDARRAY(100,1,5,7.5)"),
    ]);
    let calculation = calculate_workbook(&workbook, seeded(2026));

    let uniform = area_numbers(&calculation, 1..=100, 1..=100);
    assert!(uniform.iter().all(|value| (0.0..1.0).contains(value)));
    let mean = uniform.iter().sum::<f64>() / uniform.len() as f64;
    assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
    assert!(uniform.iter().any(|value| *value < 0.01));
    assert!(uniform.iter().any(|value| *value > 0.99));

    let integers = area_numbers(&calculation, 1..=100, 101..=120);
    assert!(
        integers
            .iter()
            .all(|value| value.fract() == 0.0 && (-3.0..=3.0).contains(value))
    );
    assert_eq!(
        integers
            .iter()
            .map(|value| *value as i64)
            .collect::<BTreeSet<_>>(),
        (-3..=3).collect::<BTreeSet<_>>()
    );

    let interval = area_numbers(&calculation, 1..=100, 121..=121);
    assert!(interval.iter().all(|value| (5.0..=7.5).contains(value)));
    assert!(interval.iter().any(|value| value.fract() != 0.0));
}

#[test]
fn randbetween_includes_both_rounded_bounds() {
    let formulas = (1..=400)
        .map(|column| (1, column, "RANDBETWEEN(1.5,3.5)"))
        .collect::<Vec<_>>();
    let calculation = calculate_workbook(&workbook_with_formulas(&formulas), seeded(9));
    let values = (1..=400)
        .map(|column| column_number(&calculation, column))
        .collect::<Vec<_>>();
    assert_eq!(
        values
            .iter()
            .map(|value| *value as i64)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([2, 3])
    );
    assert!(values.iter().all(|value| value.fract() == 0.0));

    let negative = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "RANDBETWEEN(-1,-1)"), (1, 2, "RANDBETWEEN(4,4.9)")]),
        seeded(9),
    );
    assert_eq!(column_number(&negative, 1), -1.0);
    assert_eq!(column_number(&negative, 2), 4.0);
}

#[test]
fn randarray_shapes_follow_rows_and_columns() {
    let workbook = spilled_workbook(&[
        ("A1", "=RANDARRAY()"),
        ("C1", "=RANDARRAY(3)"),
        ("E1", "=RANDARRAY(,2)"),
        ("H1", "=RANDARRAY(2,4)"),
        ("A10", "=RANDARRAY(2.9,1.2,0,10,TRUE)"),
    ]);
    let calculation = calculate_workbook(&workbook, seeded(4));
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let spill = |address: &str| {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("valid anchor"),
        );
        match calculation.materialized_cell(id).map(|cell| cell.origin()) {
            Some(cellrune::MaterializedResultOrigin::DynamicSpill { range, .. }) => (
                range.end().row().get() - range.start().row().get() + 1,
                range.end().column().get() - range.start().column().get() + 1,
            ),
            other => panic!("expected a spill at {address}, got {other:?}"),
        }
    };
    assert_eq!(spill("A1"), (1, 1));
    assert_eq!(spill("C1"), (3, 1));
    assert_eq!(spill("E1"), (1, 2));
    assert_eq!(spill("H1"), (2, 4));
    assert_eq!(spill("A10"), (2, 1));
    assert_eq!(distinct(&area_numbers(&calculation, 1..=2, 8..=11)), 8);
    assert!(
        area_numbers(&calculation, 10..=11, 1..=1)
            .iter()
            .all(|value| value.fract() == 0.0 && (0.0..=10.0).contains(value))
    );
}

#[test]
fn random_arguments_are_checked_before_the_seed() {
    let cases = [
        ("RAND(1)", ExcelError::Value),
        ("RANDBETWEEN(5,1)", ExcelError::Number),
        ("RANDBETWEEN(2,1.5)", ExcelError::Number),
        ("RANDBETWEEN(-1E+308,1E+308)", ExcelError::Number),
        ("RANDARRAY(1,1,-1E+308,1E+308)", ExcelError::Number),
        ("RANDARRAY(1,1,-1E+308,1E+308,TRUE)", ExcelError::Number),
        ("RANDBETWEEN(\"low\",3)", ExcelError::Value),
        ("RANDBETWEEN(1)", ExcelError::Value),
        ("RANDARRAY(0)", ExcelError::Calculation),
        ("RANDARRAY(0.5)", ExcelError::Calculation),
        ("RANDARRAY(1,0)", ExcelError::Calculation),
        ("RANDARRAY(-0.5)", ExcelError::Value),
        ("RANDARRAY(1,-2)", ExcelError::Value),
        ("RANDARRAY(0,-1)", ExcelError::Value),
        ("RANDARRAY(-1,0)", ExcelError::Value),
        ("RANDARRAY(1,1,5,4.9999999)", ExcelError::Value),
        ("RANDARRAY(1,1,1,1.5,TRUE)", ExcelError::Value),
        ("RANDARRAY(1,1,1.5,3,TRUE)", ExcelError::Value),
        ("RANDARRAY(1,1,-1.5,3,TRUE)", ExcelError::Value),
        ("RANDARRAY(\"x\")", ExcelError::Value),
        ("RANDARRAY(1,1,0,1,\"x\")", ExcelError::Value),
        ("RANDARRAY(1,1,1,2,3,4)", ExcelError::Value),
    ];
    let formulas = cases
        .iter()
        .zip(1..)
        .map(|((formula, _), column)| (1, column, *formula))
        .collect::<Vec<_>>();
    let workbook = workbook_with_formulas(&formulas);
    for options in [CalculationOptions::default(), seeded(1)] {
        let calculation = calculate_workbook(&workbook, options);
        for ((formula, expected), column) in cases.iter().zip(1..) {
            assert_eq!(
                calculation.cell(cell_id(column)),
                Some(&CalculationCellResult::Value(CellValue::Error(*expected))),
                "{formula}"
            );
        }
    }
}

#[test]
fn degenerate_bounds_follow_excel() {
    let cases = [
        ("RANDBETWEEN(1.5,1.6)", 2.0),
        ("RANDBETWEEN(1.5,1.5)", 2.0),
        ("RANDBETWEEN(2.5,2.6)", 3.0),
        ("RANDBETWEEN(-1.5,-1.2)", -1.0),
        ("RANDBETWEEN(-2.5,-1.5)", -2.0),
        ("RANDBETWEEN(1.2,2.5)", 2.0),
        ("RANDBETWEEN(-2.5,-1.2)", -2.0),
        ("RANDARRAY(1,1,5,5)", 5.0),
        ("RANDARRAY(1,1,0.5,0.5)", 0.5),
        ("RANDARRAY(1,1,1,1,TRUE)", 1.0),
        ("RANDARRAY(1,1,2,2.0,TRUE)", 2.0),
        ("RANDARRAY(1,1,TRUE,TRUE)", 1.0),
    ];
    let formulas = cases
        .iter()
        .zip(1..)
        .map(|((formula, _), column)| (1, column, *formula))
        .collect::<Vec<_>>();
    let workbook = workbook_with_formulas(&formulas);
    for seed in 1..=20 {
        let calculation = calculate_workbook(&workbook, seeded(seed));
        for ((formula, expected), column) in cases.iter().zip(1..) {
            assert_eq!(column_number(&calculation, column), *expected, "{formula}");
        }
    }

    let whole = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "RANDARRAY(1,1,1,3,2)"),
            (1, 2, "RANDARRAY(1,1,TRUE,2)"),
        ]),
        seeded(3),
    );
    let integer = column_number(&whole, 1);
    assert!(integer.fract() == 0.0 && (1.0..=3.0).contains(&integer));
    assert!((1.0..=2.0).contains(&column_number(&whole, 2)));
}

#[test]
fn draws_in_collected_arguments_keep_their_positions() {
    let workbook = super::workbook_with_formulas_and_names(
        &[
            (1, 1, "VAR.P(LET(x,RAND(),x),LET(y,RAND(),y))"),
            (1, 2, "VAR.P((draw),(draw))"),
            (1, 3, "SUM(LET(x,RAND(),x),-LET(y,RAND(),y))"),
        ],
        &[("draw", "RAND()")],
    );
    for seed in 1..=10 {
        let calculation = calculate_workbook(&workbook, seeded(seed));
        for column in 1..=3 {
            assert_ne!(
                column_number(&calculation, column),
                0.0,
                "seed {seed} column {column}"
            );
        }
    }
}

#[test]
fn a_draw_inside_collected_arguments_drives_the_same_dynamic_target_everywhere() {
    let mut cells = vec![(1, 1, "SUM(LET(k,RANDBETWEEN(0,4),OFFSET(B1,k,0)))")];
    cells.extend((1..=5).map(|row| (row, 2, "10*ROW()")));
    let workbook = workbook_with_formulas(&cells);
    let target = CalculationTarget::cell(cell_id(1));
    for seed in 1..=30 {
        let full = calculate_workbook(&workbook, seeded(seed));
        let value = column_number(&full, 1);
        assert!(
            [10.0, 20.0, 30.0, 40.0, 50.0].contains(&value),
            "seed {seed}: {value}"
        );
        let targeted = calculate_targets(
            &workbook,
            &[target],
            seeded(seed),
            TargetCalculationLimits::default(),
            CancellationToken::new(),
        )
        .expect("targeted calculation");
        assert_eq!(
            targeted.cell(cell_id(1)),
            full.cell(cell_id(1)),
            "seed {seed}"
        );
    }
}

#[test]
fn kernels_draw_each_argument_once() {
    let mut cells = vec![
        (1, 1, "PPMT(RAND(),1,1,-1)"),
        (1, 2, "SUMIF(OFFSET(Z1,RANDBETWEEN(0,5),0,3,1),\"<=3\")"),
        (1, 3, "AVERAGEIF(OFFSET(Z1,RANDBETWEEN(0,5),0,3,1),\">=0\")"),
        (1, 4, "COUNTIFS(Y1,\"<\"&RAND(),Y1,\">=\"&RAND())"),
    ];
    cells.extend((1..=10).map(|row| (row, 26, "ROW()")));
    let mut workbook_cells = cells.clone();
    workbook_cells.push((1, 25, "0.5"));
    let workbook = workbook_with_formulas(&workbook_cells);
    let mut straddled = false;
    for seed in 1..=40 {
        let calculation = calculate_workbook(&workbook, seeded(seed));
        let principal = column_number(&calculation, 1);
        assert!((principal - 1.0).abs() < 1e-12, "seed {seed}: {principal}");
        let sum = column_number(&calculation, 2);
        assert!([0.0, 3.0, 5.0, 6.0].contains(&sum), "seed {seed}: {sum}");
        let average = column_number(&calculation, 3);
        assert!(
            [2.0, 3.0, 4.0, 5.0, 6.0, 7.0].contains(&average),
            "seed {seed}: {average}"
        );
        straddled |= column_number(&calculation, 4) == 1.0;
    }
    assert!(straddled, "the two COUNTIFS criteria never drew apart");
}

#[test]
fn grouped_aggregations_draw_once_per_group() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = first_sheet(&draft);
    for (row, (key, value)) in (1..).zip([(1.0, 10.0), (2.0, 20.0), (3.0, 30.0), (1.0, 40.0)]) {
        for (column, number) in [(1, key), (2, value)] {
            draft
                .set_cell_value(
                    sheet_id,
                    CellAddress::from_indices(row, column).expect("input address"),
                    CellValue::number(number).expect("finite input"),
                )
                .expect("input value");
        }
    }
    for (address, formula) in [
        ("D1", "=GROUPBY(A1:A4,B1:B4,LAMBDA(v,RAND()),0,0)"),
        ("G1", "=GROUPBY(A1:A4,B1:B4,LAMBDA(v,RAND()),0,0,-2)"),
        ("J1", "=PIVOTBY(A1:A4,A1:A4,B1:B4,LAMBDA(v,RAND()),0,0,,0)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid anchor"),
                FormulaText::from_user_input(formula).expect("valid formula"),
                None,
            )
            .expect("grouped formula");
    }
    let calculation = calculate_workbook(draft.workbook(), seeded(6));
    let groups = area_numbers(&calculation, 1..=3, 5..=5);
    assert_eq!(distinct(&groups), 3);
    // Sorting by the drawn column aggregates each group again; the shown draws must be the ones
    // the sort used.
    let sorted = area_numbers(&calculation, 1..=3, 8..=8);
    assert_eq!(distinct(&sorted), 3);
    assert!(
        sorted.windows(2).all(|pair| pair[0] > pair[1]),
        "{sorted:?}"
    );
    let pivot = (2..=4)
        .flat_map(|row| (11..=13).map(move |column| (row, column)))
        .filter_map(|(row, column)| {
            let id = CalculationCellId::new(
                sheet_id,
                CellAddress::from_indices(row, column).expect("pivot address"),
            );
            match calculation.materialized_cell(id).map(|cell| cell.result()) {
                Some(CalculationCellResult::Value(CellValue::Number(number))) => Some(number.get()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(pivot.len(), 3);
    assert_eq!(distinct(&pivot), 3);
}

#[test]
fn computed_database_criteria_draw_once_per_record() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = first_sheet(&draft);
    let address = |text: &str| CellAddress::from_a1(text).expect("valid address");
    draft
        .set_cell_value(sheet_id, address("A1"), CellValue::Text("v".to_owned()))
        .expect("field header");
    for row in 2..=21 {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_indices(row, 1).expect("record address"),
                CellValue::number(f64::from(row)).expect("finite record"),
            )
            .expect("record value");
    }
    draft
        .set_cell_value(sheet_id, address("D1"), CellValue::Text("pick".to_owned()))
        .expect("criteria label");
    for (cell, text) in [
        ("D2", "=AND(A2>0,RAND()<0.5)"),
        ("F1", "=DCOUNT(A1:A21,\"v\",D1:D2)"),
    ] {
        draft
            .set_cell_formula(
                sheet_id,
                address(cell),
                FormulaText::from_user_input(text).expect("formula"),
            )
            .expect("database formula");
    }
    let mixed = (1..=5).any(|seed| {
        let calculation = calculate_workbook(draft.workbook(), seeded(seed));
        let count = number_at(
            &calculation,
            CalculationCellId::new(sheet_id, address("F1")),
        );
        count > 0.0 && count < 20.0
    });
    assert!(mixed, "every record shared one draw");
}

#[test]
fn authored_randarray_is_stored_with_its_future_prefix() {
    let text = FormulaText::from_user_input("=RANDARRAY(2,2)+RAND()+RANDBETWEEN(1,2)")
        .expect("user formula");
    assert_eq!(
        text.as_str(),
        "_xlfn.RANDARRAY(2,2)+RAND()+RANDBETWEEN(1,2)"
    );

    let stored = workbook_with_formulas(&[(1, 1, "_xlfn.RANDARRAY(1,1,5,6,TRUE)")]);
    let calculation = calculate_workbook(&stored, seeded(1));
    assert!([5.0, 6.0].contains(&column_number(&calculation, 1)));
}

#[test]
fn dynamic_references_follow_the_target_their_draw_selects() {
    let formulas = [
        (1, 1, "SUM(OFFSET(B1,RANDBETWEEN(0,4),0))"),
        (1, 3, "1+INDIRECT(\"B\"&RANDBETWEEN(1,5))"),
    ];
    let targets = (1..=5).map(|row| (row, 2, "10*ROW()"));
    let calculated =
        workbook_with_formulas(&formulas.into_iter().chain(targets).collect::<Vec<_>>());
    let calculated = calculate_workbook(&calculated, seeded(21));

    let mut literal = super::formula_sheet(1, "Sheet1", &formulas);
    for row in 1..=5 {
        super::insert_number(&mut literal, &format!("B{row}"), f64::from(row) * 10.0);
    }
    let literal = calculate_workbook(
        &super::workbook_with_sheets_and_names(vec![literal], &[]),
        seeded(21),
    );
    for column in [1, 3] {
        let value = column_number(&calculated, column);
        assert_eq!(value, column_number(&literal, column), "column {column}");
        assert!(
            (10.0..=51.0).contains(&value) && value % 10.0 <= 1.0,
            "column {column}: {value}"
        );
    }
}

#[test]
fn relative_aggregations_sort_by_the_draws_they_show() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = first_sheet(&draft);
    let data = [
        ("East", "A", 10.0),
        ("West", "B", 20.0),
        ("East", "B", 30.0),
        ("North", "A", 5.0),
        ("West", "A", 15.0),
        ("East", "A", 25.0),
        ("North", "B", 35.0),
        ("West", "B", 45.0),
    ];
    for (row, (region, product, value)) in (1..).zip(data) {
        for (column, cell) in [
            (1, CellValue::Text(region.to_owned())),
            (2, CellValue::Text(product.to_owned())),
            (3, CellValue::number(value).expect("finite input")),
        ] {
            draft
                .set_cell_value(
                    sheet_id,
                    CellAddress::from_indices(row, column).expect("input address"),
                    cell,
                )
                .expect("input value");
        }
    }
    // Sorting compares each total against all rows, while the shown totals are relative to their
    // row; both must use the same draw for the order to match what is shown.
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("E1").expect("valid anchor"),
            FormulaText::from_user_input(
                "=PIVOTBY(A1:A8,B1:B8,C1:C8,LAMBDA(v,a,RAND()),0,1,-2,1,-2,,1)",
            )
            .expect("valid formula"),
            None,
        )
        .expect("pivot formula");
    for seed in 1..=10 {
        let calculation = calculate_workbook(draft.workbook(), seeded(seed));
        let row_totals = area_numbers(&calculation, 2..=4, 8..=8);
        assert!(
            row_totals.windows(2).all(|pair| pair[0] >= pair[1]),
            "seed {seed}: {row_totals:?}"
        );
        let column_totals = area_numbers(&calculation, 5..=5, 6..=7);
        assert!(
            column_totals[0] >= column_totals[1],
            "seed {seed}: {column_totals:?}"
        );
    }
}
