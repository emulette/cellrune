#![no_main]

use std::collections::BTreeMap;

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CancellationToken, CellAddress,
    CellContent, CellRange, CellValue, Column, EditBatch, FiniteNumber, FormulaText,
    RecalculationMode, Row, SheetId, SheetName, WorkbookCalculationSession, WorkbookChange,
    calculate_workbook,
};
use libfuzzer_sys::fuzz_target;

const MAX_STEPS: usize = 64;
/// The second sheet, which reads the first, holds an `OFFSET` of a cell position whose position
/// edits move, and is deleted and restored.
const DATA: &str = "Data";

fuzz_target!(|data: &[u8]| {
    let mut session = seed_session();
    install_and_compare(&mut session);

    for (step, operation) in data.iter().copied().take(MAX_STEPS).enumerate() {
        if operation & 0x0f >= 13 {
            structural_edit(&mut session, operation);
            continue;
        }
        let revision = session.workbook().semantic_revision();
        let batch = batch_for(&session, operation, step);
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
    let data = session
        .apply_changes(
            0,
            EditBatch::new([WorkbookChange::add_sheet(
                SheetName::new(DATA).expect("constant sheet name"),
            )]),
        )
        .expect("fixed second sheet is valid")
        .created_sheet_ids()[0];
    session
        .apply_changes(
            1,
            EditBatch::new([
                WorkbookChange::set_cell_value(sheet_id, address("A1"), number(1.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B1"), formula("A1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("C1"), formula("B1+1")),
                WorkbookChange::set_cell_formula(sheet_id, address("Z1"), formula("1+1")),
                WorkbookChange::set_cell_value(sheet_id, address("A3"), number(5.0)),
                WorkbookChange::set_cell_formula(sheet_id, address("B3"), formula("SUM(A1:A3)")),
                WorkbookChange::set_cell_formula(sheet_id, address("D3"), formula("Data!A1+1")),
                WorkbookChange::set_cell_formula(data, address("A1"), formula("Sheet1!A1*10")),
                WorkbookChange::set_cell_value(data, address("B1"), number(3.0)),
                WorkbookChange::set_cell_formula(data, address("C1"), formula("OFFSET(A1,B1-3,0)")),
            ]),
        )
        .expect("fixed seed workbook is valid");
    session
}

/// A structural edit of the first sheet, and where each of its cells moves when the edit keeps
/// every reference to them, so that their values stay.
struct StructuralEdit {
    change: WorkbookChange,
    moves: Option<fn(u8, CellAddress) -> Option<CellAddress>>,
}

/// Inserting a row or column at 1 to 4, or moving A1:B3 below every referenced cell, keeps each
/// reference; deleting a row, a column, or the second sheet may not.
fn structural_change(
    session: &WorkbookCalculationSession,
    operation: u8,
) -> Option<StructuralEdit> {
    let sheet_id = sheet_id();
    let position = structural_position(operation);
    let rows = operation & 0x10 == 0;
    let edit = match operation & 0x0f {
        13 if rows => StructuralEdit {
            change: WorkbookChange::insert_rows(sheet_id, Row::new(position).ok()?, 1).ok()?,
            moves: Some(|operation, cell| {
                let row = cell.row().get();
                let shift = u32::from(row >= structural_position(operation));
                CellAddress::from_indices(row + shift, cell.column().get()).ok()
            }),
        },
        13 => StructuralEdit {
            change: WorkbookChange::insert_columns(sheet_id, Column::new(position).ok()?, 1)
                .ok()?,
            moves: Some(|operation, cell| {
                let column = cell.column().get();
                let shift = u32::from(column >= structural_position(operation));
                CellAddress::from_indices(cell.row().get(), column + shift).ok()
            }),
        },
        14 if rows => StructuralEdit {
            change: WorkbookChange::delete_rows(
                sheet_id,
                Row::new(position).ok()?,
                Row::new(position).ok()?,
            )
            .ok()?,
            moves: None,
        },
        14 => StructuralEdit {
            change: WorkbookChange::delete_columns(
                sheet_id,
                Column::new(position).ok()?,
                Column::new(position).ok()?,
            )
            .ok()?,
            moves: None,
        },
        _ if rows => StructuralEdit {
            change: WorkbookChange::move_range(
                sheet_id,
                CellRange::new(address("A1"), address("B3")).ok()?,
                CellAddress::from_indices(40 + position, position).ok()?,
            )
            .ok()?,
            moves: Some(|operation, cell| {
                let position = structural_position(operation);
                let (row, column) = (cell.row().get(), cell.column().get());
                if row <= 3 && column <= 2 {
                    CellAddress::from_indices(row + 39 + position, column + position - 1).ok()
                } else {
                    Some(cell)
                }
            }),
        },
        _ => StructuralEdit {
            change: WorkbookChange::delete_sheet(session.workbook().sheet_by_name(DATA)?.id()),
            moves: None,
        },
    };
    Some(edit)
}

fn structural_position(operation: u8) -> u32 {
    u32::from((operation >> 5) & 0x03) + 1
}

/// Applies a structural edit, compares its results with a full calculation and the values it
/// keeps with their values before it, and checks that its inverse brings every cell back.
fn structural_edit(session: &mut WorkbookCalculationSession, operation: u8) {
    let Some(edit) = structural_change(session, operation) else {
        return;
    };
    let before_values = stored_values(session);
    let before_cells = named_cells(session.workbook());
    let revision = session.workbook().semantic_revision();
    let Ok(receipt) = session.apply_changes(revision, EditBatch::new([edit.change])) else {
        assert_eq!(session.workbook().semantic_revision(), revision);
        assert_eq!(
            named_cells(session.workbook()),
            before_cells,
            "failed structural edit changed workbook cells"
        );
        return;
    };
    install_and_compare(session);
    if let Some(moves) = edit.moves {
        let after_values = stored_values(session);
        for ((sheet, cell), value) in &before_values {
            let moved = if sheet == "Sheet1" {
                moves(operation, *cell)
            } else {
                Some(*cell)
            };
            if let Some(moved) = moved {
                assert_eq!(
                    after_values.get(&(sheet.clone(), moved)),
                    Some(value),
                    "{sheet}!{cell} moved to {moved}"
                );
            }
        }
    }
    let Some(inverse) = receipt
        .inverse()
        .filter(|inverse| inverse.unrestored().is_empty())
    else {
        return;
    };
    let revision = session.workbook().semantic_revision();
    session
        .apply_changes(revision, EditBatch::new(inverse.changes().to_vec()))
        .expect("the inverse of an applied edit applies");
    install_and_compare(session);
    assert_eq!(
        named_cells(session.workbook()),
        before_cells,
        "the inverse restores every cell"
    );
    assert_eq!(stored_values(session), before_values);
}

/// The value of every stored cell by sheet name: its literal, or its formula's result.
fn stored_values(
    session: &WorkbookCalculationSession,
) -> BTreeMap<(String, CellAddress), CalculationCellResult> {
    let calculation = session
        .calculation()
        .expect("a calculation is installed after every edit");
    let mut values = BTreeMap::new();
    for sheet in session.workbook().sheets() {
        for cell in sheet.cells() {
            let value = match cell.content() {
                CellContent::Literal(value) => CalculationCellResult::Value(value.clone()),
                CellContent::Formula(_) => calculation
                    .cell(CalculationCellId::new(sheet.id(), cell.address()))
                    .expect("every formula has a result")
                    .clone(),
            };
            values.insert((sheet.name().as_str().to_owned(), cell.address()), value);
        }
    }
    values
}

/// Every cell by sheet name, since restoring a deleted sheet gives it a new ID.
fn named_cells(
    workbook: &cellrune::WorkbookSnapshot,
) -> Vec<(String, CellAddress, CellContent, cellrune::NumberFormat)> {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| {
            sheet.cells().map(|cell| {
                (
                    sheet.name().as_str().to_owned(),
                    cell.address(),
                    cell.content().clone(),
                    cell.number_format().clone(),
                )
            })
        })
        .collect()
}

fn batch_for(session: &WorkbookCalculationSession, operation: u8, step: usize) -> EditBatch {
    let sheet_id = sheet_id();
    let value = number(f64::from(operation) + step as f64);
    let data = session
        .workbook()
        .sheet_by_name(DATA)
        .map(|sheet| sheet.id());
    let changes = match operation & 0x0f {
        // Moves the cell `Data!C1`'s `OFFSET` reads among `Data!A1:A3`.
        3 if data.is_some() => vec![WorkbookChange::set_cell_value(
            data.expect("presence checked"),
            address("B1"),
            number(f64::from(3 + operation % 3)),
        )],
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
