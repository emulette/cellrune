//! Public correctness checks for the 0.1.15 snapshot-store changes.

use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
    calculate_workbook,
};

fn sheet() -> SheetId {
    SheetId::new(1).expect("valid default sheet ID")
}

fn address(a1: &str) -> CellAddress {
    CellAddress::from_a1(a1).expect("valid test address")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite test number"))
}

#[test]
fn public_source_and_result_iteration_remain_row_major() {
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("IW1"),
                    FormulaText::from_xlsx("1+1").expect("formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("A2"),
                    FormulaText::from_xlsx("2+2").expect("formula"),
                ),
            ]),
        )
        .expect("sparse workbook");
    assert_eq!(
        session.workbook().sheets()[0]
            .cells()
            .map(|cell| cell.address())
            .collect::<Vec<_>>(),
        vec![address("IW1"), address("A2")]
    );
    let calculation = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_eq!(
        calculation
            .cells()
            .map(|(cell, _)| cell.address())
            .collect::<Vec<_>>(),
        vec![address("IW1"), address("A2")]
    );
}

#[test]
fn full_and_incremental_calculation_agree_after_an_edit() {
    let sheet = sheet();
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("B1"),
                    FormulaText::from_xlsx("A1+1").expect("valid dependent formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("C1"),
                    FormulaText::from_xlsx("B1*2").expect("valid transitive formula"),
                ),
                WorkbookChange::set_cell_formula(
                    sheet,
                    address("D1"),
                    FormulaText::from_xlsx("SEQUENCE(1,3,1,1)").expect("valid spill formula"),
                ),
            ]),
        )
        .expect("workbook");

    session
        .recalculate(
            RecalculationMode::Full,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("full calculation");
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address("A1"),
                number(2.0),
            )]),
        )
        .expect("one-cell edit");

    let incremental = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation");
    let incremental_cells = session
        .calculation()
        .expect("incremental state")
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect::<Vec<_>>();
    let fresh = calculate_workbook(session.workbook(), CalculationOptions::default());
    let fresh_cells = fresh
        .cells()
        .map(|(cell, result)| (cell, result.clone()))
        .collect::<Vec<_>>();

    assert_eq!(incremental_cells, fresh_cells);
    assert!(incremental.evaluated_count() > 0);
    assert!(incremental.evaluated_count() < fresh.len());
}
