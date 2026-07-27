use std::collections::BTreeSet;

use super::{Engine, EvalContext};
use crate::calculation::runtime::Rect;
use crate::{
    CalculationOptions, CellAddress, DefinedName, DefinedNameScope, FormulaText, SheetId,
    WorkbookDraft,
};

#[test]
fn reference_selection_recurses_into_calls_and_honors_map_scope() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    draft
        .set_defined_name(
            DefinedName::new("item", DefinedNameScope::Workbook, formula("Z1"), false)
                .expect("valid defined name"),
        )
        .expect("defined name edit");
    draft
        .set_cell_formula(sheet_id, address("B1"), formula("SUM(C1)"))
        .expect("ordinary call formula");
    draft
        .set_cell_formula(
            sheet_id,
            address("B2"),
            formula("MAP(A1,LAMBDA(item,item))"),
        )
        .expect("MAP formula");
    let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());

    let ordinary = collect_reference_selection_inputs(&engine, (0, 1, 2));
    assert_eq!(ordinary, vec![rect(1, 3)]);

    let scoped = collect_reference_selection_inputs(&engine, (0, 2, 2));
    assert_eq!(scoped, vec![rect(1, 1)]);
}

fn collect_reference_selection_inputs(engine: &Engine<'_>, cell: (usize, u32, u32)) -> Vec<Rect> {
    let expr = engine.parsed_expr(cell).expect("parsed test formula");
    let mut output = Vec::new();
    engine.collect_reference_selection_inputs(
        EvalContext::for_cell(cell),
        expr,
        &mut BTreeSet::new(),
        &mut Vec::new(),
        &mut output,
    );
    output
}

fn rect(row: u32, column: u32) -> Rect {
    Rect {
        sheet: 0,
        row_start: row,
        col_start: column,
        row_end: row,
        col_end: column,
        whole_rows: false,
    }
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid test address")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("valid test formula")
}
