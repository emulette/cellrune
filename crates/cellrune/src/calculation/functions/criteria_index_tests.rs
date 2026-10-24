use super::super::super::criteria::ExactCriterion;
use super::super::super::eval::Engine;
use super::super::lookup_index::LookupIndexCache;
use crate::{
    CalculationLimits, CalculationOptions, CellAddress, CellValue, FormulaText, SheetId,
    WorkbookDraft,
};

/// Values in `A1:A40` and three `COUNTIF` calls over `range`, enough to index it.
fn counted_workbook(range: &str) -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("default sheet");
    for row in 1..=40 {
        draft
            .set_cell_value(
                sheet,
                CellAddress::from_indices(row, 1).expect("data address"),
                CellValue::number(f64::from(row % 3)).expect("finite"),
            )
            .expect("data");
    }
    for row in 1..=3 {
        draft
            .set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 26).expect("formula address"),
                FormulaText::from_xlsx(format!("COUNTIF({range},1)")).expect("formula"),
            )
            .expect("formula cell");
    }
    draft
}

fn criteria_cache_cells(engine: &Engine<'_>) -> Vec<usize> {
    let mut cache = engine.lookup_index_cache().borrow_mut();
    LookupIndexCache::criteria_mut(&mut cache)
        .indexes
        .iter()
        .map(|(_, index)| index.texts.words.len())
        .collect()
}

#[test]
fn a_criteria_range_beyond_the_reference_budget_is_never_indexed() {
    // Each call reports max_array_cells after reading the budget, so an index of the range would
    // read all of its cells for nothing.
    let limits = CalculationLimits::default()
        .with_max_array_cells(1_000)
        .expect("nonzero limit");
    let draft = counted_workbook("$A$1:$J$200");
    let engine = Engine::evaluate(
        draft.workbook(),
        CalculationOptions::default().with_limits(limits),
    );
    assert!(criteria_cache_cells(&engine).is_empty());
}

#[test]
fn a_criteria_index_reads_only_the_populated_rows_of_its_range() {
    let draft = counted_workbook("$A$1:$A$100000");
    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
    // One 64-bit word covers the 40 populated cells.
    assert_eq!(criteria_cache_cells(&engine), vec![1]);
}

#[test]
fn an_index_matches_numbers_to_fifteen_digits_only_when_the_criterion_does() {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("default sheet");
    for row in 1..=40 {
        let formula = match row % 3 {
            0 => "0.1+0.2",
            1 => "0.3",
            _ => "0.30000000000001",
        };
        draft
            .set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 1).expect("data address"),
                FormulaText::from_xlsx(formula).expect("formula"),
            )
            .expect("data");
    }
    for row in 1..=3 {
        draft
            .set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 26).expect("formula address"),
                FormulaText::from_xlsx("COUNTIF($A$1:$A$40,0.3)").expect("formula"),
            )
            .expect("formula cell");
    }
    let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
    let mut cache = engine.lookup_index_cache().borrow_mut();
    let indexes = &LookupIndexCache::criteria_mut(&mut cache).indexes;
    let [(_, index)] = indexes.as_slice() else {
        panic!("one criteria index, got {}", indexes.len());
    };
    let offsets = |approximate| {
        index
            .matching(&ExactCriterion::Number {
                number: 0.3,
                approximate,
            })
            .0
    };
    let rows = |remainders: &[u32]| {
        (1..=40_u32)
            .filter(|row| remainders.contains(&(row % 3)))
            .map(|row| row - 1)
            .collect::<Vec<_>>()
    };
    assert_eq!(offsets(true), rows(&[0, 1]));
    assert_eq!(offsets(false), rows(&[1]));
}
