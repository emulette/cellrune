use super::super::super::eval::Engine;
use super::super::super::pipeline::calculate_and_compile;
use crate::{
    CalculationCellId, CalculationLimits, CalculationOptions, CellAddress, CellValue, ExcelError,
    FormulaText, SheetId, WorkbookDraft,
};

const ROWS: u32 = 120;

/// Lookup values covering every equality class: present and absent numbers, zero (which equals a
/// blank cell under `compare`), numbers stored as text, mixed-case text, empty text, logicals,
/// text holding wildcard syntax, and a blank reference.
const LOOKUPS: &[&str] = &[
    "7",
    "0",
    "-3.5",
    "999",
    "5",
    "\"apple\"",
    "\"APPLE\"",
    "\"pear\"",
    "\"\"",
    "TRUE",
    "FALSE",
    "\"a*e\"",
    "\"5\"",
    "Z1",
];

fn data_value(row: u32) -> CellValue {
    match row % 12 {
        0 => CellValue::number(f64::from(row % 9)).expect("finite"),
        1 => CellValue::Text(["Apple", "apple", "PEAR", "Fig"][(row % 4) as usize].to_owned()),
        2 => CellValue::Text(" 5".to_owned()),
        3 => CellValue::Text(String::new()),
        4 => CellValue::Logical(row.is_multiple_of(2)),
        5 => CellValue::Error(ExcelError::NotAvailable),
        6 => CellValue::number(-3.5).expect("finite"),
        7 => CellValue::Text("a*e".to_owned()),
        8 => CellValue::Blank,
        9 => CellValue::number(-0.0).expect("finite"),
        10 => CellValue::Text(format!("item{row}")),
        _ => CellValue::number(f64::from(row)).expect("finite"),
    }
}

fn lookup_formulas() -> Vec<String> {
    let last = ROWS;
    let mut formulas = Vec::new();
    for lookup in LOOKUPS {
        formulas.push(format!("VLOOKUP({lookup},$A$1:$B${last},2,FALSE)"));
        formulas.push(format!("MATCH({lookup},$A$1:$A${last},0)"));
        formulas.push(format!(
            "XLOOKUP({lookup},$A$1:$A${last},$B$1:$B${last},\"none\",0,1)"
        ));
        formulas.push(format!(
            "XLOOKUP({lookup},$A$1:$A${last},$B$1:$B${last},\"none\",0,-1)"
        ));
        formulas.push(format!("HLOOKUP({lookup},Data!$A$1:$DP$2,2,FALSE)"));
        formulas.push(format!("MATCH({lookup},INDIRECT(\"A1:A{last}\"),0)"));
    }
    formulas
}

/// A workbook holding the lookup data and the given formulas in column D of the first sheet.
fn workbook(formulas: &[String]) -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("default sheet");
    let data = draft
        .add_sheet(crate::SheetName::new("Data").expect("sheet name"))
        .expect("data sheet");
    for row in 1..=ROWS {
        let value = data_value(row);
        if !matches!(value, CellValue::Blank) {
            draft
                .set_cell_value(sheet, address(row, 1), value.clone())
                .expect("vertical data");
            draft
                .set_cell_value(data, address(1, row), value)
                .expect("horizontal data");
        }
        let result = CellValue::number(f64::from(row) * 10.0).expect("finite");
        draft
            .set_cell_value(sheet, address(row, 2), result.clone())
            .expect("vertical results");
        draft
            .set_cell_value(data, address(2, row), result)
            .expect("horizontal results");
    }
    for (index, formula) in formulas.iter().enumerate() {
        let row = u32::try_from(index).expect("formula row") + 1;
        draft
            .set_cell_formula(
                sheet,
                address(row, 4),
                FormulaText::from_xlsx(formula).expect("lookup formula"),
            )
            .expect("formula cell");
    }
    draft
}

fn address(row: u32, column: u32) -> CellAddress {
    CellAddress::from_indices(row, column).expect("test address")
}

/// Calculates every formula together, where repeated searches use an index, and each formula
/// alone, where its only search scans, and requires the same result and the same total work.
fn assert_indexed_matches_linear(options: CalculationOptions) {
    let formulas = lookup_formulas();
    let shared = workbook(&formulas);
    let (shared_snapshot, _, shared_work) =
        calculate_and_compile(shared.workbook(), options, || false).expect("shared calculation");
    let sheet = SheetId::new(1).expect("default sheet");
    let mut function_iterations = 0;
    let mut reference_cells = 0;
    for (index, formula) in formulas.iter().enumerate() {
        let alone = workbook(std::slice::from_ref(formula));
        let (alone_snapshot, _, alone_work) =
            calculate_and_compile(alone.workbook(), options, || false).expect("lone calculation");
        let row = u32::try_from(index).expect("formula row") + 1;
        assert_eq!(
            shared_snapshot.cell(CalculationCellId::new(sheet, address(row, 4))),
            alone_snapshot.cell(CalculationCellId::new(sheet, address(1, 4))),
            "{formula}"
        );
        function_iterations += alone_work.function_iterations;
        reference_cells += alone_work.reference_cells;
    }
    assert_eq!(shared_work.function_iterations, function_iterations);
    assert_eq!(shared_work.reference_cells, reference_cells);
}

#[test]
fn indexed_exact_lookups_return_and_charge_what_linear_searches_do() {
    let formulas = lookup_formulas();
    let shared = workbook(&formulas);
    let engine = Engine::evaluate(shared.workbook(), CalculationOptions::default());
    assert!(
        !engine.lookup_index_cache().borrow().indexes.is_empty(),
        "repeated searches must build an index"
    );

    assert_indexed_matches_linear(CalculationOptions::default());
}

#[test]
fn searches_that_could_exceed_a_limit_scan_and_fail_where_linear_searches_fail() {
    for limits in [
        CalculationLimits::default()
            .with_max_function_iterations(30)
            .expect("iteration limit"),
        CalculationLimits::default()
            .with_max_array_cells(150)
            .expect("array cell limit"),
    ] {
        assert_indexed_matches_linear(CalculationOptions::default().with_limits(limits));
    }
}

#[test]
fn a_result_written_inside_an_indexed_range_drops_the_index() {
    let mut cache = super::LookupIndexCache::default();
    let area = super::LookupArea {
        sheet: 0,
        row: 1,
        column: 1,
        length: 40,
        horizontal: false,
    };
    let index = std::sync::Arc::new(super::ExactLookupIndex {
        values: std::collections::HashMap::new(),
        numeric_texts: std::collections::HashMap::new(),
        blanks: None,
        contains_nan: false,
        texts_before: vec![0; 41],
        nonempty_texts_before: vec![0; 41],
        blanks_before: vec![0; 41],
    });
    cache.insert(area, std::sync::Arc::clone(&index));
    cache.invalidate_cell((0, 41, 1));
    cache.invalidate_cell((0, 1, 2));
    assert!(cache.get(area).is_some(), "cells outside the range keep it");

    cache.invalidate_cell((0, 40, 1));
    assert!(cache.get(area).is_none());
    assert!(
        !cache.record_search(area),
        "an invalidated range waits for repeated searches again"
    );
}
