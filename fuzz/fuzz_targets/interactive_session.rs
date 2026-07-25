#![no_main]

use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
    calculate_workbook,
};
use libfuzzer_sys::fuzz_target;

const MAX_STEPS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let mut session = seed_session();
    install_and_compare(&mut session);

    for (step, operation) in data.iter().copied().take(MAX_STEPS).enumerate() {
        let revision = session.workbook().semantic_revision();
        let batch = batch_for(operation, step);
        let expected_revision = if operation & 0x80 == 0 {
            revision
        } else {
            revision.saturating_sub(1)
        };
        let before_revision = session.workbook().semantic_revision();
        let before_cells = source_cells(session.workbook());
        match session.apply_changes(expected_revision, batch) {
            Ok(_) => install_and_compare(&mut session),
            Err(_) => {
                assert_eq!(session.workbook().semantic_revision(), before_revision);
                assert_eq!(
                    source_cells(session.workbook()),
                    before_cells,
                    "failed atomic edit changed workbook cells"
                );
            }
        }
    }
});

fn seed_session() -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    let sheet_id = sheet_id();
    session
        .apply_changes(
            0,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("Z1"), formula("1+1")),
            ]),
        )
        .expect("fixed seed workbook is valid");
    session
}

fn batch_for(operation: u8, step: usize) -> EditBatch {
    let sheet_id = sheet_id();
    let value = number(f64::from(operation) + step as f64);
    let changes = match operation & 0x0f {
        0..=3 => vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("A1"),
            value,
        )],
        4 => vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("B1"),
            formula("A1+2"),
        )],
        5 => vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("B1"),
            formula("C1+1"),
        )],
        6 => vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("B1"),
            formula("A1+1"),
        )],
        7 => vec![WorkbookChange::clear_cell(sheet_id, address("C1"))],
        8 => vec![WorkbookChange::set_cell_formula(
            sheet_id,
            address("C1"),
            formula("B1+1"),
        )],
        9 => vec![
            WorkbookChange::set_cell_value(sheet_id, address("D1"), value),
            WorkbookChange::set_cell_formula(sheet_id, address("E1"), formula("D1*2")),
        ],
        10 => vec![
            WorkbookChange::set_cell_value(sheet_id, address("D1"), value.clone()),
            WorkbookChange::set_cell_value(
                SheetId::new(999).expect("constant absent sheet ID"),
                address("A1"),
                value,
            ),
        ],
        11 => vec![
            WorkbookChange::set_cell_value(sheet_id, address("F1"), number(2.0)),
            WorkbookChange::set_cell_dynamic_formula(
                sheet_id,
                address("G1"),
                formula("TAKE({1,2,3},,F1)"),
                None,
            )
            .expect("fixed dynamic formula is valid"),
        ],
        12 => vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("F1"),
            number(f64::from((operation % 3) + 1)),
        )],
        _ => vec![WorkbookChange::set_cell_value(
            sheet_id,
            address("D1"),
            value,
        )],
    };
    EditBatch::new(changes)
}

fn source_cells(
    workbook: &cellrune::WorkbookSnapshot,
) -> Vec<(
    SheetId,
    CellAddress,
    cellrune::CellContent,
    cellrune::NumberFormat,
)> {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| {
            sheet.cells().map(|cell| {
                (
                    sheet.id(),
                    cell.address(),
                    cell.content().clone(),
                    cell.number_format().clone(),
                )
            })
        })
        .collect()
}

fn install_and_compare(session: &mut WorkbookCalculationSession) {
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("bounded generated calculation must complete");
    let oracle = calculate_workbook(session.workbook(), CalculationOptions::default());
    let installed = session
        .calculation()
        .expect("successful calculation is installed");
    assert_eq!(
        installed.cells().collect::<Vec<_>>(),
        oracle.cells().collect::<Vec<_>>()
    );
    assert_eq!(
        installed.materialized_cells().collect::<Vec<_>>(),
        oracle.materialized_cells().collect::<Vec<_>>()
    );
}

fn sheet_id() -> SheetId {
    SheetId::new(1).expect("constant sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("constant address")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("constant formula")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("bounded finite number"))
}
