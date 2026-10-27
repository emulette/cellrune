//! Functions that take a value where an array arrives, as Mac Excel 16.114 calculates them: called
//! once per element in an array context (a dynamic or array formula, or an argument that takes an
//! array), and by implicit intersection where a formula that is not an array formula ends.

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationLimits,
    CalculationOptions, CellAddress, CellContent, CellRange, CellValue, DateSystem, FormulaCell,
    FormulaDialect, FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult, Sheet,
    SheetId, SheetName, SheetVisibility, WorkbookSnapshot, WorkbookSource, calculate_workbook,
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
    results_with(entry, formula, cells, CalculationOptions::default())
}

/// [`results`] under `options`.
fn results_with(
    entry: Entry,
    formula: &str,
    cells: &[(&str, CellValue)],
    options: CalculationOptions,
) -> Vec<Vec<String>> {
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
    let calculation = calculate_workbook(&workbook, options);
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
        ("TYPE({1,2}*1)", &[][..], "64"),
        ("TYPE(SQRT({4}))", &[][..], "64"),
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
    for formula in ["TYPE({1})", "TYPE({1}*1)", "LET(x,{1,2},TYPE(x))"] {
        assert_eq!(
            results(Entry::Dynamic, formula, &[]),
            grid(&[&["64"]]),
            "{formula}"
        );
    }
    assert_eq!(results(Entry::Dynamic, "TYPE(A1)", &cells), grid(&[&["1"]]));
    // A reference's shape decides, whatever part of a column is populated.
    for formula in ["TYPE(A:A)", "TYPE(Y:Y)", "TYPE(A1:A1048576)"] {
        assert_eq!(
            results(Entry::Dynamic, formula, &cells),
            grid(&[&["64"]]),
            "{formula}"
        );
    }
    // An array that cannot be calculated is not an array type.
    assert_eq!(
        results(Entry::Dynamic, "TYPE(SEQUENCE({2,3}))", &[]),
        grid(&[&["calculation.unsupported_expression"]])
    );
    // A function that reads an array and returns one value is not an array.
    for formula in [
        "TYPE(LARGE({3,1,2},1))",
        "TYPE(INDEX({1,2,3},2))",
        "TYPE(TYPE({1,2}))",
    ] {
        for entry in [Entry::Legacy, Entry::Dynamic] {
            assert_eq!(results(entry, formula, &[]), grid(&[&["1"]]), "{formula}");
        }
    }
    assert_eq!(
        results(Entry::Dynamic, "TYPE(INDEX({1,2;3,4},1))", &[]),
        grid(&[&["64"]])
    );
}

#[test]
fn a_value_selected_from_an_array_takes_the_place_of_a_value() {
    let cells = [
        ("B1", number(1.0)),
        ("B2", number(2.0)),
        ("B3", number(3.0)),
        ("B4", number(4.0)),
    ];
    for (entry, formula, expected) in [
        (
            Entry::Dynamic,
            "SEQUENCE(LARGE({3,1,2},1))",
            grid(&[&["1"], &["2"], &["3"]]),
        ),
        (
            Entry::Dynamic,
            "SEQUENCE(INDEX({2,3},1))",
            grid(&[&["1"], &["2"]]),
        ),
        (
            Entry::Legacy,
            "SUM(B1:INDEX(B1:B4,LARGE({1,2,3},1)))",
            grid(&[&["6"]]),
        ),
    ] {
        assert_eq!(results(entry, formula, &cells), expected, "{formula}");
    }
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
        (Entry::Dynamic, "LET(x,SEQUENCE(2),OFFSET(A1,x,0))"),
        // Indexes holding arrays select a reference per element where one reference is needed.
        (Entry::Dynamic, "COUNTIF(INDEX(A1:A2,{1,2}),1)"),
        (Entry::Dynamic, "SUM(A1:INDEX(A1:A2,{1,2}))"),
        // Several delimiters match whichever occurs first.
        (Entry::Dynamic, "TEXTBEFORE(\"a-b+c\",{\"-\",\"+\"})"),
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

#[test]
fn index_rows_and_columns_take_arrays_of_values() {
    let cells = [
        ("A2", number(21.0)),
        ("B2", number(22.0)),
        ("B1", number(12.0)),
    ];
    for (formula, expected) in [
        ("INDEX({10,20,30},2)", grid(&[&["20"]])),
        ("INDEX({10,20;30,40},2,1)", grid(&[&["30"]])),
        // A two-dimensional array without a column selects a row; a reference needs the column.
        ("INDEX({10,20;30,40},2)", grid(&[&["30", "40"]])),
        ("INDEX({10,20;30,40},2,0)", grid(&[&["30", "40"]])),
        ("INDEX({10,20,30},0,2)", grid(&[&["20"]])),
        ("INDEX({10,20,30},{1,3})", grid(&[&["10", "30"]])),
        ("INDEX({10,20,30},4)", grid(&[&["#REF!"]])),
        ("INDEX(A1:B3,2)", grid(&[&["#REF!"]])),
        ("INDEX(A1:A3,2)", grid(&[&["21"]])),
        ("INDEX(A1:B1,2)", grid(&[&["12"]])),
        ("ROWS({1;2;3})", grid(&[&["3"]])),
        ("COLUMNS({1,2})", grid(&[&["2"]])),
        ("ROWS(A1:A3*1)", grid(&[&["3"]])),
        ("ROWS(1)", grid(&[&["1"]])),
        ("COLUMNS(\"a\")", grid(&[&["1"]])),
    ] {
        assert_eq!(
            results(Entry::Dynamic, formula, &cells),
            expected,
            "{formula}"
        );
    }
    // INDEX over an array value selects values in a formula that is not an array formula too.
    for (formula, expected) in [
        ("INDEX({10,20,30},2)", "20"),
        ("INDEX({10,20;30,40},2,2)", "40"),
        ("INDEX(A1:A3*1,2)", "21"),
        ("INDEX(SORT({3;1;2}),2)", "2"),
    ] {
        assert_eq!(
            results(Entry::Legacy, formula, &cells),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
}

#[test]
fn lookups_search_arrays_and_take_arrays_of_lookup_values() {
    let criteria = [
        ("A1", text("a")),
        ("A2", text("b")),
        ("A3", text("b")),
        ("B1", number(1.0)),
        ("B2", number(1.0)),
        ("B3", number(2.0)),
        ("C3", number(32.0)),
    ];
    let keys = [
        ("A1", number(1.0)),
        ("A2", number(2.0)),
        ("A3", number(3.0)),
        ("B2", text("x")),
        ("B3", text("y")),
        ("C2", text("z")),
    ];
    for (formula, cells, expected) in [
        ("MATCH(4,{1,4,9},0)", &[][..], grid(&[&["2"]])),
        ("MATCH({4,9},{1,4,9},0)", &[][..], grid(&[&["2", "3"]])),
        // A lookup array of several rows and columns is #N/A.
        ("MATCH(5,{1,2;3,4},0)", &[][..], grid(&[&["#N/A"]])),
        ("MATCH(2,A1:B3,0)", &keys[..], grid(&[&["#N/A"]])),
        (
            "INDEX(C1:C3,MATCH(1,(A1:A3=\"b\")*(B1:B3=2),0))",
            &criteria[..],
            grid(&[&["32"]]),
        ),
        (
            "VLOOKUP(2,{1,\"a\";2,\"b\"},2,FALSE)",
            &[][..],
            grid(&[&["\"b\""]]),
        ),
        (
            "VLOOKUP({1;2},{1,\"a\";2,\"b\"},2,FALSE)",
            &[][..],
            grid(&[&["\"a\""], &["\"b\""]]),
        ),
        (
            "HLOOKUP(2,{1,2;\"a\",\"b\"},2,FALSE)",
            &[][..],
            grid(&[&["\"b\""]]),
        ),
        (
            "LOOKUP({1;3},{1,2,3},{\"a\",\"b\",\"c\"})",
            &[][..],
            grid(&[&["\"a\""], &["\"c\""]]),
        ),
        // XLOOKUP is not in Mac Excel 16.114; these follow Microsoft's documentation.
        (
            "XLOOKUP(2,{1,2,3},{\"a\",\"b\",\"c\"})",
            &[][..],
            grid(&[&["\"b\""]]),
        ),
        (
            "XLOOKUP({2;3},A1:A3,B1:B3)",
            &keys[..],
            grid(&[&["\"x\""], &["\"y\""]]),
        ),
        (
            "XLOOKUP(2,A1:A3,B1:C3)",
            &keys[..],
            grid(&[&["\"x\"", "\"z\""]]),
        ),
        (
            "XLOOKUP(5,{\"5\";5},{\"text\";\"number\"})",
            &[][..],
            grid(&[&["\"number\""]]),
        ),
        (
            "XLOOKUP({2;3},A1:A3,B1:C3)",
            &keys[..],
            grid(&[&["calculation.unsupported_expression"]]),
        ),
    ] {
        assert_eq!(
            results(Entry::Dynamic, formula, cells),
            expected,
            "{formula}"
        );
    }
}

#[test]
fn if_and_choose_return_the_reference_they_select() {
    // Mac Excel 16.114: the selected argument stays a reference, so aggregates pass over its text
    // and logical values as they do in a range.
    for (formula, cells, expected) in [
        ("ISREF(IF(TRUE,A1))", vec![("A1", number(1.0))], "TRUE"),
        ("ISREF(CHOOSE(1,A1))", vec![("A1", number(1.0))], "TRUE"),
        ("SUM(IF(TRUE,A1))", vec![("A1", text("5"))], "0"),
        ("COUNT(CHOOSE(1,A1))", vec![("A1", text("5"))], "0"),
        ("SUM(CHOOSE(1,A1))", vec![("A1", text("1,000"))], "0"),
        (
            "SUM(CHOOSE(1,A1))",
            vec![("A1", CellValue::Logical(true))],
            "0",
        ),
        ("AND(CHOOSE(1,A1))", vec![("A1", text("TRUE"))], "#VALUE!"),
        ("OR(IF(TRUE,A1),FALSE)", vec![("A1", text("TRUE"))], "FALSE"),
        (
            "ROWS(CHOOSE(2,A1:A2,A1:A5))",
            vec![("A1", number(1.0))],
            "5",
        ),
        // A condition that holds an array selects element by element.
        (
            "SUM(IF({1,0},A1,A2))",
            vec![("A1", number(1.0)), ("A2", number(2.0))],
            "3",
        ),
    ] {
        assert_eq!(
            results(Entry::Legacy, formula, &cells),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
}

#[test]
fn and_reads_the_elements_of_an_array_argument() {
    // Mac Excel 16.114: `AND({TRUE,FALSE})` is FALSE.
    assert_eq!(
        results(Entry::Legacy, "AND({TRUE,FALSE})", &[]),
        grid(&[&["FALSE"]])
    );
    assert_eq!(
        results(Entry::Legacy, "AND({1,0})", &[]),
        grid(&[&["FALSE"]])
    );
    let cells = [("A1", number(1.0)), ("A2", number(0.0))];
    assert_eq!(
        results(Entry::Dynamic, "AND(A1:A2>0)", &cells),
        grid(&[&["FALSE"]])
    );
}

#[test]
fn an_index_holding_an_array_reads_values_where_a_value_is_needed() {
    let cells = [
        ("A1", number(1.0)),
        ("A2", number(2.0)),
        ("A3", number(3.0)),
    ];
    for (entry, formula, expected) in [
        (Entry::Legacy, "INDEX(A1:A3,{2,3})", grid(&[&["2"]])),
        (Entry::Dynamic, "INDEX(A1:A3,{2,3})", grid(&[&["2", "3"]])),
        (
            Entry::Dynamic,
            "LET(r,INDEX(A1:A3,{2,3}),SUM(r))",
            grid(&[&["5"]]),
        ),
        (Entry::Dynamic, "SUM(INDEX(A1:A3,{1,2}))", grid(&[&["3"]])),
    ] {
        assert_eq!(results(entry, formula, &cells), expected, "{formula}");
    }
}

#[test]
fn choose_ifs_and_switch_select_element_by_element() {
    let cells = [
        ("A1", number(1.0)),
        ("A2", number(2.0)),
        ("A3", number(3.0)),
        ("B1", text("a")),
        ("B2", text("b")),
        ("B3", text("c")),
    ];
    for (entry, formula, expected) in [
        (
            Entry::Dynamic,
            "CHOOSE({1,2},B1:B3,A1:A3)",
            grid(&[&["\"a\"", "1"], &["\"b\"", "2"], &["\"c\"", "3"]]),
        ),
        (
            Entry::Dynamic,
            "CHOOSE(2,{1,2},{3,4})",
            grid(&[&["3", "4"]]),
        ),
        (Entry::Legacy, "SUM(CHOOSE(2,{1,2},{3,4}))", grid(&[&["7"]])),
        (
            Entry::Dynamic,
            "IFS(A1:A3>1,B1:B3,TRUE,0)",
            grid(&[&["0"], &["\"b\""], &["\"c\""]]),
        ),
        (
            Entry::Dynamic,
            "SWITCH(A1:A3,1,B1:B3,0)",
            grid(&[&["\"a\""], &["0"], &["0"]]),
        ),
        (Entry::Dynamic, "IFS(TRUE,{1,2})", grid(&[&["1", "2"]])),
    ] {
        assert_eq!(results(entry, formula, &cells), expected, "{formula}");
    }
}

#[test]
fn iferror_broadcasts_an_array_value_against_its_fallback() {
    // Mac Excel 16.114: an array value meets the fallback's shape even without an error, while a
    // single value that is not an error stands alone.
    for (formula, expected) in [
        ("IFERROR({1,2},{7;8})", grid(&[&["1", "2"], &["1", "2"]])),
        ("IFERROR({1,#N/A},{7;8})", grid(&[&["1", "7"], &["1", "8"]])),
        ("IFERROR(1,{7,8})", grid(&[&["1"]])),
        ("IFERROR(1/0,{7,8})", grid(&[&["7", "8"]])),
        // An error evaluating the fallback is the fallback's value, not the result's.
        ("IFERROR({1,2},INDEX({1,2},5))", grid(&[&["1", "2"]])),
        ("IFERROR({1,#N/A},INDEX({1,2},5))", grid(&[&["1", "#REF!"]])),
    ] {
        assert_eq!(results(Entry::Dynamic, formula, &[]), expected, "{formula}");
    }
}

#[test]
fn analysis_toolpak_functions_reject_a_range_of_several_cells() {
    // Mac Excel 16.114 returns #VALUE! for a range in a value argument of a function that came from
    // the Analysis ToolPak, in any formula, and lifts a computed array.
    let cells = [
        ("A1", number(45_000.0)),
        ("A2", number(45_100.0)),
        ("A3", number(45_200.0)),
    ];
    for (entry, formula, expected) in [
        (Entry::Legacy, "EOMONTH(A1:A3,0)", grid(&[&["#VALUE!"]])),
        (Entry::Dynamic, "EOMONTH(A1:A3,0)", grid(&[&["#VALUE!"]])),
        (Entry::Legacy, "QUOTIENT(A1:A3,2)", grid(&[&["#VALUE!"]])),
        (Entry::Dynamic, "EOMONTH(A1:A1,0)", grid(&[&["45016"]])),
        (
            Entry::Dynamic,
            "EOMONTH(A1:A3+0,0)",
            grid(&[&["45016"], &["45107"], &["45230"]]),
        ),
        (Entry::Dynamic, "SUM(QUOTIENT({5,7},2))", grid(&[&["5"]])),
    ] {
        assert_eq!(results(entry, formula, &cells), expected, "{formula}");
    }
}

#[test]
fn textjoin_uses_several_delimiters_in_turn() {
    // Microsoft's documentation for TEXTJOIN; Mac Excel 16.114 does not have the function.
    let cells = [
        ("A1", text("x")),
        ("A2", text("y")),
        ("A3", text("z")),
        ("B1", text(",")),
        ("C1", text(";")),
    ];
    for (entry, formula, expected) in [
        (
            Entry::Dynamic,
            "TEXTJOIN({\"-\",\"+\"},TRUE,\"a\",\"b\",\"c\",\"d\")",
            "\"a-b+c-d\"",
        ),
        (Entry::Legacy, "TEXTJOIN(B1:C1,TRUE,A1:A3)", "\"x,y;z\""),
        (
            Entry::Dynamic,
            "TEXTJOIN(\"-\",FALSE,\"a\",\"\",\"b\")",
            "\"a--b\"",
        ),
        // A whole column cycles through every row, blank past the populated ones.
        (
            Entry::Dynamic,
            "TEXTJOIN(B:B,TRUE,\"a\",\"b\",\"c\")",
            "\"a,bc\"",
        ),
        (Entry::Dynamic, "TEXTJOIN(Y:Y,TRUE,\"a\",\"b\")", "\"ab\""),
        (Entry::Dynamic, "TEXTBEFORE(\"a,b\",B1)", "\"a\""),
        (
            Entry::Dynamic,
            "TEXTBEFORE(\"a,b\",B:B)",
            "calculation.unsupported_expression",
        ),
    ] {
        assert_eq!(
            results(entry, formula, &cells),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
}

#[test]
fn if_and_choose_evaluate_their_condition_or_index_once() {
    // One LAMBDA call per condition: a second evaluation would pass the limit.
    let once = CalculationOptions::default().with_limits(
        CalculationLimits::default()
            .with_max_lambda_invocations(1)
            .expect("positive lambda invocation limit"),
    );
    let cells = [
        ("A1", number(1.0)),
        ("A2", number(2.0)),
        ("A3", number(3.0)),
        ("B1", number(10.0)),
        ("C1", CellValue::Logical(true)),
    ];
    for (formula, expected) in [
        ("ROUND(IF(LAMBDA(x,x)(1)>0,1,2),0)", "1"),
        ("ROUND(IF(ROUND(IF(LAMBDA(x,x)(1)>0,1,2),0)>0,1,2),0)", "1"),
        ("LEN(IF(LAMBDA(x,x)(1)>0,\"a\",\"bb\"))", "1"),
        ("COUNTIF(IF(LAMBDA(x,x)(1)>0,A1:A3,B1),1)", "1"),
        ("AND(IF(LAMBDA(x,x)(1)>0,C1))", "TRUE"),
        ("SUM(IF(LAMBDA(x,x)(1)>5,A1,0))", "0"),
        ("SUM(CHOOSE(LAMBDA(x,x)(1),A1:A3,B1))", "6"),
    ] {
        assert_eq!(
            results_with(Entry::Legacy, formula, &cells, once),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
    // A condition that holds an array is evaluated once for all its elements.
    let three = CalculationOptions::default().with_limits(
        CalculationLimits::default()
            .with_max_lambda_invocations(3)
            .expect("positive lambda invocation limit"),
    );
    assert_eq!(
        results_with(
            Entry::Cse,
            "SUM(IF(MAP(A1:A3,LAMBDA(x,x>1)),A1:A3,0))",
            &cells,
            three
        ),
        grid(&[&["5"]])
    );
}

#[test]
fn an_argument_that_is_not_lifted_is_read_once_for_every_element() {
    // The same random number reaches each element.
    let rounded = results_with(
        Entry::Dynamic,
        "ROUND(RAND(),{9,9})",
        &[],
        CalculationOptions::default().with_random_seed(7),
    );
    assert_eq!(rounded.len(), 1);
    assert_eq!(rounded[0].len(), 2);
    assert_eq!(rounded[0][0], rounded[0][1]);

    // INDEX and XLOOKUP read their source, the arrays they search and return, and the indexes and
    // fallback that hold one value once: one LAMBDA call each, and every element from one column.
    let once = CalculationOptions::default().with_limits(
        CalculationLimits::default()
            .with_max_lambda_invocations(1)
            .expect("positive lambda invocation limit"),
    );
    let cells = [
        ("A1", number(1.0)),
        ("A2", number(2.0)),
        ("A3", number(3.0)),
        ("B1", number(10.0)),
        ("B2", number(20.0)),
        ("B3", number(30.0)),
    ];
    for (formula, expected) in [
        ("INDEX(A1:A3,{1,2},LAMBDA(x,x)(1))", ["1", "2"]),
        (
            "INDEX(IF(LAMBDA(x,x)(1)>0,B1:B3,A1:A3),{1,2})",
            ["10", "20"],
        ),
        (
            "XLOOKUP({1,2},A1:A3,IF(LAMBDA(x,x)(1)>0,B1:B3,A1:A3))",
            ["10", "20"],
        ),
        ("XLOOKUP({1,9},A1:A3,B1:B3,LAMBDA(x,x)(5))", ["10", "5"]),
    ] {
        assert_eq!(
            results_with(Entry::Dynamic, formula, &cells, once),
            grid(&[&expected]),
            "{formula}"
        );
    }
    for seed in 0..16 {
        let options = CalculationOptions::default().with_random_seed(seed);
        for formula in [
            "INDEX(CHOOSE(RANDBETWEEN(1,2),A1:A3,B1:B3),{1,2,3})",
            "XLOOKUP({1,2,3},A1:A3,IF(RAND()>0.5,A1:A3,B1:B3))",
        ] {
            let selected = results_with(Entry::Dynamic, formula, &cells, options);
            assert!(
                selected == grid(&[&["1", "2", "3"]]) || selected == grid(&[&["10", "20", "30"]]),
                "{formula} with seed {seed}: {selected:?}"
            );
        }
    }
}

#[test]
fn criteria_arrays_far_larger_than_the_range_stay_within_the_array_budget() {
    let cells = [
        ("A1", number(1.0)),
        ("A2", number(4.0)),
        ("A3", number(9.0)),
    ];
    assert_eq!(
        results(
            Entry::Dynamic,
            "SUM(COUNTIF(A1:A3,SEQUENCE(20000)))",
            &cells
        ),
        grid(&[&["3"]])
    );
}

#[test]
fn a_range_argument_given_by_a_call_or_name_stays_a_reference_while_another_argument_expands() {
    let cells = [
        ("A1", text("a")),
        ("A2", text("b")),
        ("A3", text("a")),
        ("A4", text("c")),
        ("B1", number(1.0)),
        ("B2", number(2.0)),
        ("B3", number(3.0)),
        ("B4", number(4.0)),
    ];
    for (formula, expected) in [
        ("SUMPRODUCT(COUNTIF(OFFSET(A1,0,0,4),{\"a\",\"b\"}))", "3"),
        ("LET(r,A1:A4,SUMPRODUCT(COUNTIF(r,{\"a\",\"b\"})))", "3"),
        ("LET(r,A1:A4,SUMPRODUCT(1/COUNTIF(r,r)))", "3"),
        (
            "SUMPRODUCT(SUMIF(INDIRECT(\"A1:A4\"),{\"a\",\"b\"},B1:B4))",
            "6",
        ),
    ] {
        assert_eq!(
            results(Entry::Legacy, formula, &cells),
            grid(&[&[expected]]),
            "{formula}"
        );
    }
}
