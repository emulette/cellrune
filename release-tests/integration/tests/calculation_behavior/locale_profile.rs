use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationOptions, CalculationSnapshot,
    CellAddress, CellValue, FormulaText, Locale, LocaleProfile, SheetId, WorkbookDraft,
    WorkbookSnapshot, calculate_workbook,
};

use super::materialized_result;

const SCALARS: [(&str, &str); 6] = [
    ("A1", "=DOLLAR(1234.567,2)"),
    ("B1", "=DOLLAR(-5)"),
    ("C1", "=TEXT(DATE(2026,7,27),\"mmmm\")"),
    ("D1", "=TEXT(DATE(2026,7,27),\"[$-409]mmmm\")"),
    ("E1", "=TEXT(DATE(2026,7,27),\"yyyy-mm-dd\")"),
    ("F1", "=TEXT(TIME(13,0,0),\"h AM/PM\")"),
];

fn locale_workbook() -> (WorkbookSnapshot, SheetId) {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in SCALARS {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid scalar address"),
                FormulaText::from_user_input(formula).expect("valid scalar formula"),
            )
            .expect("scalar formula mutation");
    }
    for (address, formula) in [
        ("A3", "=GROUPBY({\"A\";\"B\";\"A\"},{1;2;3},SUM)"),
        (
            "D3",
            "=GROUPBY({\"E\",\"A\";\"E\",\"B\";\"W\",\"A\"},{10;20;30},SUM,,2)",
        ),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid grouped anchor"),
                FormulaText::from_user_input(formula).expect("valid grouped formula"),
                None,
            )
            .expect("grouped formula mutation");
    }
    (draft.workbook().clone(), sheet_id)
}

fn text_at<'a>(calculation: &'a CalculationSnapshot, sheet_id: SheetId, address: &str) -> &'a str {
    match materialized_result(calculation, sheet_id, address) {
        Some(CalculationCellResult::Value(CellValue::Text(text))) => text,
        other => panic!("expected text at {address}, got {other:?}"),
    }
}

/// The texts the two-level GROUPBY spills into column D.
fn grouped_labels(calculation: &CalculationSnapshot, sheet_id: SheetId) -> Vec<String> {
    (3..=12)
        .filter_map(
            |row| match materialized_result(calculation, sheet_id, &format!("D{row}")) {
                Some(CalculationCellResult::Value(CellValue::Text(text))) => Some(text.clone()),
                _ => None,
            },
        )
        .collect()
}

fn assert_unsupported(calculation: &CalculationSnapshot, sheet_id: SheetId, address: &str) {
    match materialized_result(calculation, sheet_id, address) {
        Some(CalculationCellResult::Unavailable(issue)) => {
            assert_eq!(
                issue.code(),
                CalculationIssueCode::UnsupportedExpression,
                "{address}"
            );
        }
        other => panic!("expected an unsupported result at {address}, got {other:?}"),
    }
}

#[test]
fn the_default_profile_keeps_english_currency_names_and_labels() {
    let (workbook, sheet_id) = locale_workbook();
    let explicit =
        CalculationOptions::default().with_locale_profile(LocaleProfile::new(Locale::EnUs));
    for options in [CalculationOptions::default(), explicit] {
        let calculation = calculate_workbook(&workbook, options);
        for (address, expected) in [
            ("A1", "$1,234.57"),
            ("B1", "($5.00)"),
            ("C1", "July"),
            ("D1", "July"),
            ("E1", "2026-07-27"),
            ("F1", "1 PM"),
            ("A5", "Total"),
        ] {
            assert_eq!(
                text_at(&calculation, sheet_id, address),
                expected,
                "{address}"
            );
        }
        assert!(grouped_labels(&calculation, sheet_id).contains(&"Grand Total".to_owned()));
    }
}

#[test]
fn the_korean_profile_shows_won_and_korean_total_labels() {
    let (workbook, sheet_id) = locale_workbook();
    let options =
        CalculationOptions::default().with_locale_profile(LocaleProfile::new(Locale::KoKr));
    let calculation = calculate_workbook(&workbook, options);

    for (address, expected) in [
        ("A1", "₩1,234.57"),
        ("D1", "July"),
        ("E1", "2026-07-27"),
        ("A5", "합계"),
    ] {
        assert_eq!(
            text_at(&calculation, sheet_id, address),
            expected,
            "{address}"
        );
    }
    // A negative amount, the names, and the grand-total label of the Korean regional format have
    // not been measured.
    for address in ["B1", "C1", "F1", "D3"] {
        assert_unsupported(&calculation, sheet_id, address);
    }
}

#[test]
fn the_display_language_labels_totals_apart_from_the_regional_format() {
    let (workbook, sheet_id) = locale_workbook();
    let profile = LocaleProfile::new(Locale::KoKr).with_display_language(Locale::EnUs);
    let calculation = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_locale_profile(profile),
    );

    assert_eq!(text_at(&calculation, sheet_id, "A1"), "₩1,234.57");
    assert_eq!(text_at(&calculation, sheet_id, "A5"), "Total");
    assert!(grouped_labels(&calculation, sheet_id).contains(&"Grand Total".to_owned()));
    assert_unsupported(&calculation, sheet_id, "C1");
}
