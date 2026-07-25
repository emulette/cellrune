use std::fs;

use cellrune::{CellContent, ReadOptions, SavedResult, read_xlsx_bytes};

const CORPUS_PATH_ENV: &str = "WORKBOOK_FORMULA_CORPUS";

#[test]
#[ignore = "requires an external formula corpus path in WORKBOOK_FORMULA_CORPUS"]
fn external_corpus_preserves_all_551_formulas_without_saved_results() {
    let path = std::env::var_os(CORPUS_PATH_ENV)
        .expect("external formula corpus path environment variable");
    let bytes = fs::read(path).expect("read external formula corpus");
    let snapshot =
        read_xlsx_bytes(&bytes, ReadOptions::default()).expect("read external formula corpus");
    let formulas = snapshot
        .sheets()
        .iter()
        .flat_map(|sheet| sheet.cells())
        .filter_map(|cell| match cell.content() {
            CellContent::Formula(formula) => Some(formula),
            CellContent::Literal(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(formulas.len(), 551);
    assert!(formulas.iter().all(|formula| formula.text().is_some()));
    assert!(
        formulas
            .iter()
            .all(|formula| matches!(formula.saved_result(), SavedResult::Missing))
    );
}
