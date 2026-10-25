//! Functions that take a value where an array arrives, as Mac Excel 16.114 calculates them: called
//! once per element in an array context (a dynamic or array formula, or an argument that takes an
//! array), and by implicit intersection where a formula that is not an array formula ends.

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationOptions, CellAddress,
    CellContent, CellRange, CellValue, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName,
    SheetVisibility, WorkbookSnapshot, WorkbookSource, calculate_workbook,
};

/// How the formula under test is entered in `Z2`.
#[derive(Clone, Copy)]
enum Entry {
    /// A formula that is not an array formula.
    Legacy,
    /// A single-cell array formula.
    Cse,
    /// A dynamic array formula, which spills.
    Dynamic,
}

/// Calculates `formula` in `Z2` over `cells` and returns the rows of its result: the spilled area
/// of a dynamic formula, or the single cell otherwise.
fn results(entry: Entry, formula: &str, cells: &[(&str, CellValue)]) -> Vec<Vec<String>> {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (address, value) in cells {
        sheet
            .insert_cell(
                CellAddress::from_a1(address).expect("valid input address"),
                CellContent::Literal(value.clone()),
            )
            .expect("unique input address");
    }
    let anchor = CellAddress::from_indices(2, 26).expect("valid anchor");
    let metadata = match entry {
        Entry::Legacy => FormulaMetadata::Normal,
        Entry::Cse => FormulaMetadata::Array {
            range: CellRange::new(anchor, anchor).expect("valid array range"),
            always_calculate: false,
        },
        Entry::Dynamic => FormulaMetadata::DynamicArray {
            range: None,
            always_calculate: false,
        },
    };
    sheet
        .insert_cell(
            anchor,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx(formula).expect("valid formula"),
                SavedResult::Missing,
                metadata,
            )),
        )
        .expect("unique formula address");
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("elementwise-test", "1").expect("valid provider identity"),
            None,
        ),
    )
    .expect("valid workbook");
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let at = |row: u32, column: u32| {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_indices(row, column).expect("valid result address"),
        );
        if (row, column) == (2, 26) {
            calculation.cell(id).cloned()
        } else {
            calculation
                .materialized_cell(id)
                .map(|cell| cell.result().clone())
        }
    };
    let mut rows = Vec::new();
    for row in 2..=6 {
        let values = (26..=29)
            .map_while(|column| at(row, column).map(|result| render(&result)))
            .collect::<Vec<_>>();
        if values.is_empty() {
            break;
        }
        rows.push(values);
    }
    rows
}

fn render(result: &CalculationCellResult) -> String {
    match result {
        CalculationCellResult::Value(CellValue::Number(number)) => number.get().to_string(),
        CalculationCellResult::Value(CellValue::Text(text)) => format!("{text:?}"),
        CalculationCellResult::Value(CellValue::Logical(logical)) => {
            logical.to_string().to_uppercase()
        }
        CalculationCellResult::Value(CellValue::Error(error)) => error.as_str().to_owned(),
        CalculationCellResult::Value(CellValue::Blank) => "blank".to_owned(),
        CalculationCellResult::Unavailable(issue) => issue.code().as_str().to_owned(),
        _ => format!("{result:?}"),
    }
}

fn number(value: f64) -> CellValue {
    CellValue::number(value).expect("finite test number")
}

fn text(value: &str) -> CellValue {
    CellValue::Text(value.to_owned())
}

fn grid(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(|value| (*value).to_owned()).collect())
        .collect()
}

#[test]
fn value_arguments_take_arrays_element_by_element_in_array_contexts() {
    let numbers = [
        ("A1", number(1.0)),
        ("A2", number(4.0)),
        ("A3", number(9.0)),
    ];
    let texts = [("A1", text("a")), ("A2", text("bb")), ("A3", text("ccc"))];
    let dates = [
        ("A1", number(45_306.0)),
        ("A2", number(45_337.0)),
        ("A3", number(45_307.0)),
    ];
    let reciprocals = [
        ("A1", number(0.0)),
        ("A2", number(2.0)),
        ("A3", number(4.0)),
    ];
    let rounding = [
        ("A1", number(1.25)),
        ("A2", number(2.55)),
        ("A3", number(3.15)),
    ];
    for (entry, formula, cells, expected) in [
        (
            Entry::Dynamic,
            "SQRT(A1:A3)",
            &numbers[..],
            grid(&[&["1"], &["2"], &["3"]]),
        ),
        (
            Entry::Dynamic,
            "LEN(A1:A3)",
            &texts[..],
            grid(&[&["1"], &["2"], &["3"]]),
        ),
        (
            Entry::Dynamic,
            "ROUND(A1:A3,{0,1})",
            &rounding[..],
            grid(&[&["1", "1.3"], &["3", "2.6"], &["3", "3.2"]]),
        ),
        (
            Entry::Dynamic,
            "DATE(2024,{1,2},1)",
            &[][..],
            grid(&[&["45292", "45323"]]),
        ),
        (
            Entry::Dynamic,
            "CONCATENATE({\"a\",\"b\"},\"x\")",
            &[][..],
            grid(&[&["\"ax\"", "\"bx\""]]),
        ),
        (
            Entry::Dynamic,
            "TEXT({1,2},\"0.0\")",
            &[][..],
            grid(&[&["\"1.0\"", "\"2.0\""]]),
        ),
        (
            Entry::Dynamic,
            "COUNTIF(A1:A3,{1,4})",
            &numbers[..],
            grid(&[&["1", "1"]]),
        ),
        (
            Entry::Legacy,
            "SUMPRODUCT(LEN(A1:A3))",
            &texts[..],
            grid(&[&["6"]]),
        ),
        (Entry::Cse, "SUM(LEN(A1:A3))", &texts[..], grid(&[&["6"]])),
        (
            Entry::Legacy,
            "SUM(LEN({\"a\",\"bb\"}))",
            &[][..],
            grid(&[&["3"]]),
        ),
        (
            Entry::Legacy,
            "SUMPRODUCT(--(MONTH(A1:A3)=1))",
            &dates[..],
            grid(&[&["2"]]),
        ),
        (
            Entry::Cse,
            "SUM(IFERROR(1/A1:A3,0))",
            &reciprocals[..],
            grid(&[&["0.75"]]),
        ),
        (
            Entry::Dynamic,
            "IFERROR(1/A1:A3,\"x\")",
            &reciprocals[..],
            grid(&[&["\"x\""], &["0.5"], &["0.25"]]),
        ),
    ] {
        assert_eq!(results(entry, formula, cells), expected, "{formula}");
    }
}

#[test]
fn a_blank_element_is_a_blank_cell_rather_than_an_omitted_argument() {
    // `LEFT("abc")` is "a"; `LEFT("abc",blank)` reads the blank as 0.
    let cells = [("B2", number(2.0))];
    assert_eq!(
        results(Entry::Dynamic, "LEFT(\"abc\",B1:B2)", &cells),
        grid(&[&["\"\""], &["\"ab\""]]),
    );
    assert_eq!(
        results(Entry::Dynamic, "ISBLANK(B1:B3)", &cells),
        grid(&[&["TRUE"], &["FALSE"], &["TRUE"]]),
    );
}

#[test]
fn a_formula_that_is_not_an_array_formula_intersects_ranges_and_takes_an_arrays_first_element() {
    let texts = [("A1", text("a")), ("A2", text("bb")), ("A3", text("ccc"))];
    let values = [("A1", number(-1.0)), ("A2", text("x")), ("A3", number(9.0))];
    for (formula, cells, expected) in [
        // `Z2` intersects row 2.
        ("LEN(A1:A3)", &texts[..], "2"),
        ("ISNUMBER(A1:A3)", &values[..], "FALSE"),
        ("ABS(A1:A3)", &[("A2", number(-4.0))][..], "4"),
        ("LEN({\"a\",\"bb\"})", &[][..], "1"),
        ("ISNUMBER({1,\"a\"})", &[][..], "TRUE"),
        ("ABS({-1,2})", &[][..], "1"),
        ("TYPE(A1:A3)", &values[..], "2"),
        ("TYPE({1,2})", &[][..], "64"),
    ] {
        assert_eq!(
            results(Entry::Legacy, formula, cells),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
}

#[test]
fn type_is_64_for_an_array_in_an_array_context() {
    let cells = [("A1", number(1.0)), ("A2", text("x")), ("A3", number(3.0))];
    assert_eq!(
        results(Entry::Dynamic, "TYPE(A1:A3)", &cells),
        grid(&[&["64"]])
    );
    assert_eq!(
        results(Entry::Dynamic, "TYPE({1,2})", &[]),
        grid(&[&["64"]])
    );
    assert_eq!(results(Entry::Dynamic, "TYPE(A1)", &cells), grid(&[&["1"]]));
}

#[test]
fn index_selects_one_value_for_each_element_of_its_indexes() {
    let cells = [
        ("A1", number(11.0)),
        ("B1", number(12.0)),
        ("A3", number(31.0)),
        ("B3", number(32.0)),
    ];
    for (entry, formula, expected) in [
        (
            Entry::Dynamic,
            "INDEX(A1:C3,{1;3},{1,2})",
            grid(&[&["11", "12"], &["31", "32"]]),
        ),
        (
            Entry::Dynamic,
            "INDEX(A1:C3,{1;3},0)",
            grid(&[&["11"], &["31"]]),
        ),
        (
            Entry::Dynamic,
            "INDEX(A1:C3,{1;4},1)",
            grid(&[&["11"], &["#REF!"]]),
        ),
        (
            Entry::Legacy,
            "SUM(INDEX(A1:C3,{1;3},{1,2}))",
            grid(&[&["86"]]),
        ),
        (Entry::Legacy, "INDEX(A1:C3,{1;3},{1,2})", grid(&[&["11"]])),
    ] {
        assert_eq!(results(entry, formula, &cells), expected, "{formula}");
    }
}

#[test]
fn functions_without_established_array_rules_fail_explicitly_on_array_values() {
    let cells = [("A1", number(1.0)), ("A2", number(2.0))];
    for (entry, formula) in [
        (Entry::Dynamic, "SEQUENCE({2,3})"),
        (Entry::Dynamic, "OFFSET(A1,{0;1},0)"),
        (Entry::Legacy, "OFFSET(A1,{0;1},0)"),
        (Entry::Dynamic, "XLOOKUP({1;2},A1:A2,A1:A2)"),
    ] {
        assert_eq!(
            results(entry, formula, &cells),
            grid(&[&["calculation.unsupported_expression"]]),
            "{formula}"
        );
    }
}

#[test]
fn the_absorbed_element_wise_functions_keep_their_results_in_every_context() {
    let numbers = [
        ("A1", number(-1.0)),
        ("A2", number(-4.0)),
        ("A3", number(9.0)),
    ];
    let values = [("A1", number(-1.0)), ("A2", text("x")), ("A3", number(9.0))];
    let conditions = [
        ("A1", number(-1.0)),
        ("A2", number(4.0)),
        ("A3", number(9.0)),
    ];
    for (entry, formula, cells, expected) in [
        (Entry::Cse, "ABS(A1:A3)", &numbers[..], grid(&[&["1"]])),
        (
            Entry::Dynamic,
            "ABS(A1:A3)",
            &numbers[..],
            grid(&[&["1"], &["4"], &["9"]]),
        ),
        (
            Entry::Cse,
            "SUM(ABS(A1:A3))",
            &numbers[..],
            grid(&[&["14"]]),
        ),
        (
            Entry::Dynamic,
            "ABS(A1:A3/0)",
            &numbers[..],
            grid(&[&["#DIV/0!"], &["#DIV/0!"], &["#DIV/0!"]]),
        ),
        (
            Entry::Legacy,
            "IF(A1:A3>0,\"p\",\"n\")",
            &conditions[..],
            grid(&[&["\"p\""]]),
        ),
        (
            Entry::Dynamic,
            "IF(A1:A3>0,\"p\",\"n\")",
            &conditions[..],
            grid(&[&["\"n\""], &["\"p\""], &["\"p\""]]),
        ),
        (
            Entry::Cse,
            "ISNUMBER(A1:A3)",
            &values[..],
            grid(&[&["TRUE"]]),
        ),
        (
            Entry::Dynamic,
            "ISNUMBER(A1:A3)",
            &values[..],
            grid(&[&["TRUE"], &["FALSE"], &["TRUE"]]),
        ),
        (
            Entry::Dynamic,
            "N(A1:A3)",
            &values[..],
            grid(&[&["-1"], &["0"], &["9"]]),
        ),
        (
            Entry::Dynamic,
            "T(A1:A3)",
            &values[..],
            grid(&[&["\"\""], &["\"x\""], &["\"\""]]),
        ),
    ] {
        assert_eq!(results(entry, formula, cells), expected, "{formula}");
    }
}
