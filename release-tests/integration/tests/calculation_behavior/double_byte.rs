use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CalculationSnapshot,
    CellAddress, CellValue, ExcelError, FormulaText, Locale, LocaleProfile, SheetId, WorkbookDraft,
    WorkbookSnapshot, calculate_workbook, scan_formula_capabilities,
};

use super::materialized_result;

#[derive(Debug, Clone, Copy)]
enum Expected {
    Number(f64),
    Text(&'static str),
    Error(ExcelError),
    Unsupported,
}

fn workbook(cases: &[(&str, Expected)]) -> (WorkbookSnapshot, SheetId) {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (row, (formula, _)) in cases.iter().enumerate() {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_indices(row as u32 + 1, 1).expect("valid formula address"),
                FormulaText::from_user_input(*formula).expect("valid formula"),
            )
            .expect("formula mutation");
    }
    (draft.workbook().clone(), sheet_id)
}

fn assert_cases(calculation: &CalculationSnapshot, sheet_id: SheetId, cases: &[(&str, Expected)]) {
    for (row, (formula, expected)) in cases.iter().enumerate() {
        let actual = materialized_result(calculation, sheet_id, &format!("A{}", row + 1));
        let expected_value = match expected {
            Expected::Number(number) => CellValue::number(*number).expect("finite number"),
            Expected::Text(text) => CellValue::Text((*text).to_owned()),
            Expected::Error(error) => CellValue::Error(*error),
            Expected::Unsupported => {
                match actual {
                    Some(CalculationCellResult::Unavailable(issue)) => assert_eq!(
                        issue.code(),
                        CalculationIssueCode::UnsupportedExpression,
                        "{formula}"
                    ),
                    other => panic!("{formula}: expected an unsupported result, got {other:?}"),
                }
                continue;
            }
        };
        assert_eq!(
            actual,
            Some(&CalculationCellResult::Value(expected_value)),
            "{formula}"
        );
    }
}

#[test]
fn byte_functions_count_characters_without_a_double_byte_locale() {
    let cases = [
        ("=LENB(\"CellRune 오라클\")", Expected::Number(12.0)),
        ("=LEFTB(\"CellRune Oracle\",4)", Expected::Text("Cell")),
        ("=LEFTB(\"가나다\",2)", Expected::Text("가나")),
        ("=RIGHTB(\"CellRune Oracle\",6)", Expected::Text("Oracle")),
        ("=MIDB(\"CellRune Oracle\",5,4)", Expected::Text("Rune")),
        (
            "=FINDB(\"Rune\",\"CellRune Oracle\")",
            Expected::Number(5.0),
        ),
        (
            "=SEARCHB(\"rune\",\"CellRune Oracle\")",
            Expected::Number(5.0),
        ),
        (
            "=REPLACEB(\"CellRune\",5,4,\"Oracle\")",
            Expected::Text("CellOracle"),
        ),
        (
            "=MIDB(\"CellRune Oracle\",0,4)",
            Expected::Error(ExcelError::Value),
        ),
        // Excel 16.114 without a double-byte language counts UTF-16 code units and leaves ASC
        // and DBCS text unchanged (excel-measure RESULTS.md, section D).
        ("=LENB(\"😀\")", Expected::Number(2.0)),
        ("=FINDB(\"b\",\"😀b\")", Expected::Number(3.0)),
        ("=MIDB(\"a😀b\",2,2)", Expected::Text("😀")),
        ("=RIGHTB(\"a😀\",2)", Expected::Text("😀")),
        ("=REPLACEB(\"😀b\",3,1,\"c\")", Expected::Text("😀c")),
        ("=ASC(\"ＡＢ가\")", Expected::Text("ＡＢ가")),
        ("=DBCS(\"AB\")", Expected::Text("AB")),
        // A cut inside a surrogate pair is unmeasured.
        ("=MIDB(\"a😀b\",2,1)", Expected::Unsupported),
        ("=LEFTB(\"😀\",1)", Expected::Unsupported),
        ("=FINDB(\"b\",\"😀b\",2)", Expected::Unsupported),
    ];
    let (workbook, sheet_id) = workbook(&cases);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_cases(&calculation, sheet_id, &cases);
}

#[test]
fn the_korean_profile_counts_two_bytes_for_hangul_and_full_width_forms() {
    let cases = [
        ("=LENB(\"CellRune 오라클\")", Expected::Number(15.0)),
        ("=LENB(\"ㄱ　Ａ\")", Expected::Number(6.0)),
        ("=LEFTB(\"CellRune Oracle\",\"2\")", Expected::Text("Ce")),
        ("=LEFTB(\"가나다\",4)", Expected::Text("가나")),
        ("=RIGHTB(\"ab가\",3)", Expected::Text("b가")),
        ("=MIDB(\"가나다\",3,2)", Expected::Text("나")),
        ("=MIDB(\"가나다\",9,2)", Expected::Text("")),
        ("=FINDB(\"다\",\"가나다\")", Expected::Number(5.0)),
        ("=FINDB(\"가\",\"가나가\",3)", Expected::Number(5.0)),
        ("=SEARCHB(\"나?\",\"가나다\")", Expected::Number(3.0)),
        ("=REPLACEB(\"가나다\",3,2,\"x\")", Expected::Text("가x다")),
        ("=ASC(\"ＡＢＣ１２３\")", Expected::Text("ABC123")),
        ("=ASC(\"가　나\")", Expected::Text("가 나")),
        (
            "=DBCS(\"CellRune Oracle 2026\")",
            Expected::Text("ＣｅｌｌＲｕｎｅ　Ｏｒａｃｌｅ　２０２６"),
        ),
        ("=DBCS(\" \")", Expected::Text("\u{3000}")),
        (
            "=LEFTB(\"CellRune Oracle\",-1)",
            Expected::Error(ExcelError::Value),
        ),
        (
            "=FINDB(\"zz\",\"CellRune Oracle\")",
            Expected::Error(ExcelError::Value),
        ),
        ("=LEFTB(\"가나다\",3)", Expected::Unsupported),
        ("=MIDB(\"가나다\",2,2)", Expected::Unsupported),
        ("=LENB(\"é\")", Expected::Unsupported),
        ("=DBCS(\"é\")", Expected::Unsupported),
    ];
    let (workbook, sheet_id) = workbook(&cases);
    let options =
        CalculationOptions::default().with_locale_profile(LocaleProfile::new(Locale::KoKr));
    let calculation = calculate_workbook(&workbook, options);

    assert_cases(&calculation, sheet_id, &cases);
}
