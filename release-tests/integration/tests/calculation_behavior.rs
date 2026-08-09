use std::collections::BTreeSet;

use cellrune::{
    ArithmeticSemantics, CalculationCellId, CalculationCellResult, CalculationHints,
    CalculationIssueCode, CalculationLimits, CalculationOptions, CalculationOptionsError,
    CellAddress, CellContent, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope,
    ExcelError, FiniteNumber, FormulaCapability, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, MaterializedResultOrigin, Provenance, ProviderIdentity, SavedResult, Sheet,
    SheetId, SheetName, SheetVisibility, WorkbookDraft, WorkbookSnapshot, WorkbookSource,
    calculate_workbook, scan_formula_capabilities, scan_formula_capabilities_with_options,
    scan_function_usage, supported_function_catalog,
};

#[path = "calculation_behavior/criteria_matching.rs"]
mod criteria_matching;
#[path = "calculation_behavior/database.rs"]
mod database;
#[path = "calculation_behavior/distributions.rs"]
mod distributions;
#[path = "calculation_behavior/moments.rs"]
mod moments;
#[path = "calculation_behavior/regression.rs"]
mod regression;
#[path = "calculation_behavior/roman_numeral.rs"]
mod roman_numeral;
#[path = "calculation_behavior/support.rs"]
mod support;

use support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn unsupported_functions_are_not_hidden_by_iferror() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "NOPE()"),
        (1, 2, "IFERROR(A1,42)"),
        (1, 3, "A1+1"),
        (1, 4, "1+2"),
    ]);
    let report = scan_formula_capabilities(&workbook);
    assert_eq!(report.formula_count(), 4);
    assert_eq!(report.supported_count(), 3);

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&calculation, 1, CalculationIssueCode::UnsupportedFunction);
    assert_issue(&calculation, 2, CalculationIssueCode::BlockedByUpstream);
    assert_issue(&calculation, 3, CalculationIssueCode::BlockedByUpstream);
    assert_eq!(
        calculation.cell(cell_id(4)),
        Some(&CalculationCellResult::Value(
            CellValue::number(3.0).expect("finite expected result")
        ))
    );
}

#[test]
fn v0_1_10_text_array_functions_materialize_rectangular_spills() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "=TEXTSPLIT(\"alpha|beta;gamma|delta\",\"|\",\";\")"),
        ("D1", "=REGEXEXTRACT(\"a1 b22 c333\",\"[0-9]+\",1)"),
        (
            "F1",
            "=REGEXEXTRACT(\"CR-2026-0727\",\"([A-Z]+)-([0-9]{4})\",2)",
        ),
        ("I1", "=TEXTSPLIT(\"a|b;c\",\"|\",\";\",FALSE,0,\"-\")"),
        ("L1", "=TEXTSPLIT(\"Axxb|c\",{\"XX\",\"|\"},,TRUE,1)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }
    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    for (address, expected) in [
        ("A1", "alpha"),
        ("B1", "beta"),
        ("A2", "gamma"),
        ("B2", "delta"),
        ("D1", "1"),
        ("D2", "22"),
        ("D3", "333"),
        ("F1", "CR"),
        ("G1", "2026"),
        ("I1", "a"),
        ("J1", "b"),
        ("I2", "c"),
        ("J2", "-"),
        ("L1", "A"),
        ("M1", "b"),
        ("N1", "c"),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "unexpected materialized text at {address}",
        );
    }
}

#[test]
fn v0_1_10_text_and_regex_work_respects_calculation_limits() {
    let literal = "a".repeat(1_000);
    let literal_formula = format!("REGEXTEST(\"{literal}\",\"{literal}\")");
    let literal_result = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, literal_formula.as_str())]),
        CalculationOptions::default(),
    );
    assert_eq!(
        literal_result.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );

    let linear_formula = format!("REGEXREPLACE(\"{}\",\"(?:a)+\",\"X\")", "a".repeat(1_000));
    let linear = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, linear_formula.as_str())]),
        CalculationOptions::default(),
    );
    assert_eq!(
        linear.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "X".to_owned()
        )))
    );

    let regex_limits = CalculationLimits::default()
        .with_max_function_iterations(60)
        .expect("positive regex work limit");
    let regex = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(REGEXTEST(\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"(a+)+b\"),FALSE)",
        )]),
        CalculationOptions::default().with_limits(regex_limits),
    );
    assert_issue(&regex, 1, CalculationIssueCode::ResourceLimitExceeded);

    let split_limits = CalculationLimits::default()
        .with_max_function_iterations(25)
        .expect("positive split work limit");
    let split = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(TEXTSPLIT(\"a|a|a|a|a|a|a|a|a|a\",\"|\"),\"hidden\")",
        )]),
        CalculationOptions::default().with_limits(split_limits),
    );
    assert_issue(&split, 1, CalculationIssueCode::ResourceLimitExceeded);

    let delimiter_formula = format!("IFERROR(TEXTSPLIT(\"\",\"{}\"),\"hidden\")", "x".repeat(64));
    let delimiter_limits = CalculationLimits::default()
        .with_max_function_iterations(100)
        .expect("positive delimiter preprocessing limit");
    let delimiter = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, delimiter_formula.as_str())]),
        CalculationOptions::default().with_limits(delimiter_limits),
    );
    assert_issue(&delimiter, 1, CalculationIssueCode::ResourceLimitExceeded);

    let text_limits = CalculationLimits::default()
        .with_max_text_bytes(8)
        .expect("positive text limit");
    let output = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(REGEXREPLACE(\"aaaa\",\"a\",\"xxxx\"),\"hidden\")",
        )]),
        CalculationOptions::default().with_limits(text_limits),
    );
    assert_issue(&output, 1, CalculationIssueCode::ResourceLimitExceeded);

    let mut split_draft = WorkbookDraft::new();
    let split_sheet = split_draft.workbook().sheets()[0].id();
    split_draft
        .set_cell_dynamic_formula(
            split_sheet,
            CellAddress::from_a1("A1").expect("valid split anchor"),
            FormulaText::from_user_input("=TEXTSPLIT(\"a|b;c\",\"|\",\";\",FALSE,0,\"123456789\")")
                .expect("valid split formula"),
            None,
        )
        .expect("dynamic split formula mutation");
    let padded = calculate_workbook(
        split_draft.workbook(),
        CalculationOptions::default().with_limits(text_limits),
    );
    assert_issue(&padded, 1, CalculationIssueCode::ResourceLimitExceeded);

    let padding_formula = format!(
        "IFERROR(TEXTSPLIT(\"a|b;c\",\"|\",\";\",FALSE,0,\"{}\"),\"hidden\")",
        "p".repeat(256)
    );
    let padding_limits = CalculationLimits::default()
        .with_max_function_iterations(200)
        .expect("positive aggregate padding limit");
    let padding = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, padding_formula.as_str())]),
        CalculationOptions::default().with_limits(padding_limits),
    );
    assert_issue(&padding, 1, CalculationIssueCode::ResourceLimitExceeded);

    let capture_limits = CalculationLimits::default()
        .with_max_function_iterations(100)
        .expect("positive capture work limit");
    let captures = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(REGEXREPLACE(\"aaaa\",\"()()()()()()()()()()a\",\"\"),\"hidden\")",
        )]),
        CalculationOptions::default().with_limits(capture_limits),
    );
    assert_issue(&captures, 1, CalculationIssueCode::ResourceLimitExceeded);

    let repeated_limits = CalculationLimits::default()
        .with_max_function_iterations(500)
        .expect("positive repeated native-call limit");
    let repeated = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(REGEXREPLACE(\"aaaaaaaaaaaaaaaaaaaa\",\"(*NO_START_OPT)(*NO_AUTO_POSSESS)(?=(?:(a|aa)+b))|\",\"\"),\"hidden\")",
        )]),
        CalculationOptions::default().with_limits(repeated_limits),
    );
    assert_issue(&repeated, 1, CalculationIssueCode::ResourceLimitExceeded);

    let named_pattern = format!("{}(?<target>)", "()".repeat(100));
    let named_replacement = "${target}".repeat(100);
    let named_formula = format!("REGEXREPLACE(\"\",\"{named_pattern}\",\"{named_replacement}\")");
    let named_limits = CalculationLimits::default()
        .with_max_function_iterations(9_000)
        .expect("positive named-capture lookup limit");
    let named = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, named_formula.as_str())]),
        CalculationOptions::default().with_limits(named_limits),
    );
    assert_eq!(
        named.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(
            CellValue::Text(String::new())
        ))
    );
}

#[test]
fn v0_1_10_grouped_aggregations_share_callable_and_spill_semantics() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        (
            "A1",
            "=GROUPBY({\"A\";\"B\";\"A\";\"C\"},{100;200;300;400},SUM)",
        ),
        (
            "D1",
            "=PIVOTBY({\"A\";\"B\";\"A\";\"C\"},{FALSE;FALSE;TRUE;TRUE},{100;200;300;400},SUM)",
        ),
        (
            "I1",
            "=GROUPBY({\"A\";\"B\";\"A\";\"C\"},{100;200;300;400},LAMBDA(items,SUM(items)))",
        ),
        (
            "L1",
            "=GROUPBY({\"A\";\"B\";\"A\";\"C\"},{100;200;300;400},PERCENTOF)",
        ),
        (
            "Q1",
            "=PIVOTBY({\"A\";\"A\";\"B\";\"B\"},{\"X\";\"Y\";\"X\";\"Y\"},{10;30;20;40},LAMBDA(subset,totalset,SUM(subset)/SUM(totalset)),,,,,,,1)",
        ),
        (
            "V1",
            "=PIVOTBY({\"Region\";\"A\";\"A\";\"B\";\"B\"},{\"Period\";\"X\";\"Y\";\"X\";\"Y\"},{\"Sales\",\"Units\";10,1;30,3;20,2;40,4},SUM,3)",
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
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("O1").expect("valid percent anchor"),
            FormulaText::from_user_input("=PERCENTOF({100;200;300;400},100)")
                .expect("valid percent formula"),
        )
        .expect("percent formula mutation");
    for (address, formula) in [
        ("AD1", "=ISBLANK(F3)"),
        ("AE1", "=ISTEXT(F3)"),
        ("AF1", "=F3=0"),
        ("AG1", "=COUNTBLANK(F3)"),
    ] {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid empty-intersection check"),
                FormulaText::from_user_input(formula)
                    .expect("valid empty-intersection check formula"),
            )
            .expect("empty-intersection check mutation");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    for (address, expected) in [
        ("B1", 400.0),
        ("B2", 200.0),
        ("B3", 400.0),
        ("B4", 1_000.0),
        ("E2", 100.0),
        ("F2", 300.0),
        ("G2", 400.0),
        ("E3", 200.0),
        ("G3", 200.0),
        ("F4", 400.0),
        ("G4", 400.0),
        ("E5", 300.0),
        ("F5", 700.0),
        ("G5", 1_000.0),
        ("J1", 400.0),
        ("J4", 1_000.0),
        ("M1", 0.4),
        ("M2", 0.2),
        ("M3", 0.4),
        ("M4", 1.0),
        ("O1", 10.0),
        ("R2", 0.25),
        ("S2", 0.75),
        ("T2", 1.0),
        ("R3", 1.0 / 3.0),
        ("S3", 2.0 / 3.0),
        ("T3", 1.0),
        ("W4", 10.0),
        ("X4", 1.0),
        ("Y4", 30.0),
        ("Z4", 3.0),
        ("AA4", 40.0),
        ("AB4", 4.0),
        ("W5", 20.0),
        ("X5", 2.0),
        ("Y5", 40.0),
        ("Z5", 4.0),
        ("AA5", 60.0),
        ("AB5", 6.0),
        ("W6", 30.0),
        ("X6", 3.0),
        ("Y6", 70.0),
        ("Z6", 7.0),
        ("AA6", 100.0),
        ("AB6", 10.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for (address, expected) in [
        ("A1", "A"),
        ("A2", "B"),
        ("A3", "C"),
        ("A4", "Total"),
        ("D2", "A"),
        ("D3", "B"),
        ("D4", "C"),
        ("D5", "Total"),
        ("W1", "Period"),
        ("W2", "X"),
        ("X2", "X"),
        ("Y2", "Y"),
        ("Z2", "Y"),
        ("AA2", "Total"),
        ("AB2", "Total"),
        ("V3", "Region"),
        ("W3", "Sales"),
        ("X3", "Units"),
        ("Y3", "Sales"),
        ("Z3", "Units"),
        ("V4", "A"),
        ("V5", "B"),
        ("V6", "Total"),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "unexpected grouped label at {address}",
        );
    }
    assert_eq!(
        materialized_result(&calculation, sheet_id, "D1"),
        Some(&CalculationCellResult::Value(
            CellValue::Text(String::new())
        ))
    );
    assert_eq!(
        materialized_result(&calculation, sheet_id, "E1"),
        Some(&CalculationCellResult::Value(CellValue::Logical(false)))
    );
    assert_eq!(
        materialized_result(&calculation, sheet_id, "F1"),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );
    for (address, expected) in [("AD1", false), ("AE1", true), ("AF1", false)] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Logical(expected))),
            "unexpected empty-intersection reference semantics at {address}",
        );
    }
    assert_materialized_number(&calculation, sheet_id, "AG1", 1.0);
    for address in ["E4", "F3"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(
                CellValue::Text(String::new())
            )),
            "expected an empty pivot intersection at {address}",
        );
    }
}

#[test]
fn v0_1_10_grouped_options_are_typed_before_grouping() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        (
            "A1",
            "=GROUPBY({\"Category\";\"B\";\"A\";\"B\";\"C\"},{\"Amount\";10;30;20;40},SUM,3,-1,-2,{TRUE;TRUE;TRUE;FALSE;TRUE},0)",
        ),
        (
            "E1",
            "=PIVOTBY({\"A\";\"A\";\"B\";\"B\"},{\"X\";\"Y\";\"X\";\"Y\"},{10;30;20;40},PERCENTOF)",
        ),
        (
            "K1",
            "=GROUPBY({\"East\",\"A\";\"East\",\"B\";\"West\",\"A\"},{10;20;30},SUM)",
        ),
        ("P1", "=GROUPBY({1,1;1,2;2,1},{10;20;30},SUM,,,,,1)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid grouped option anchor"),
                FormulaText::from_user_input(formula).expect("valid grouped option formula"),
                None,
            )
            .expect("grouped option formula mutation");
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    for (address, expected) in [
        ("A1", "Category"),
        ("A2", "Total"),
        ("A3", "C"),
        ("A4", "A"),
        ("A5", "B"),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            )))
        );
    }
    for (address, expected) in [
        ("B2", 80.0),
        ("B3", 40.0),
        ("B4", 30.0),
        ("B5", 10.0),
        ("F2", 1.0 / 3.0),
        ("G2", 3.0 / 7.0),
        ("H2", 0.4),
        ("F3", 2.0 / 3.0),
        ("G3", 4.0 / 7.0),
        ("H3", 0.6),
        ("F4", 1.0),
        ("G4", 1.0),
        ("H4", 1.0),
        ("M1", 10.0),
        ("M2", 20.0),
        ("M3", 30.0),
        ("M4", 60.0),
        ("R1", 10.0),
        ("R2", 20.0),
        ("R3", 30.0),
        ("R4", 60.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for (address, expected) in [
        ("K1", "East"),
        ("L1", "A"),
        ("K2", "East"),
        ("L2", "B"),
        ("K3", "West"),
        ("L3", "A"),
        ("K4", "Total"),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            )))
        );
    }
}

#[test]
fn v0_1_10_grouped_hierarchy_sort_headers_and_relative_sets_are_stable() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        (
            "A1",
            "=PIVOTBY({\"A\";\"A\";\"B\";\"B\"},{\"X\";\"Y\";\"X\";\"Y\"},{10;30;20;40},PERCENTOF,,,,,,,1)",
        ),
        (
            "F1",
            "=PIVOTBY({\"A\";\"A\";\"B\";\"B\"},{\"X\";\"Y\";\"X\";\"Y\"},{10;30;20;40},PERCENTOF,,,,,,,2)",
        ),
        (
            "K1",
            "=GROUPBY({2,\"b\";1,\"c\";1,\"a\"},{20;30;10},SUM,,0,{-1;2})",
        ),
        ("O1", "=GROUPBY({\"B\";\"A\"},{2;1},SUM,,P10)"),
        (
            "R1",
            "=GROUPBY({\"Category\";\"B\";\"A\"},{\"Amount\";2;1},SUM,1,0)",
        ),
        ("U1", "=GROUPBY({\"B\";\"A\"},{2;1},SUM,2,0)"),
        (
            "Y1",
            "=PIVOTBY({\"A\",\"a\";\"A\",\"b\";\"B\",\"a\";\"B\",\"b\"},{\"X\",\"x\";\"X\",\"y\";\"Y\",\"x\";\"Y\",\"y\"},{10;30;20;40},PERCENTOF,0,2,,2,,,3)",
        ),
        (
            "AI1",
            "=PIVOTBY({\"A\",\"a\";\"A\",\"b\";\"B\",\"a\";\"B\",\"b\"},{\"X\",\"x\";\"X\",\"y\";\"Y\",\"x\";\"Y\",\"y\"},{10;30;20;40},PERCENTOF,0,2,,2,,,4)",
        ),
        (
            "AR1",
            "=GROUPBY({\"A\",\"x\";\"A\",\"y\";\"B\",\"x\";\"B\",\"y\"},{100;1;60;50},SUM,,2,-3)",
        ),
        (
            "AU1",
            "=GROUPBY({\"Category\";\"A\";\"B\"},{\"Amount\";1;2},SUM)",
        ),
        (
            "AW1",
            "=PIVOTBY({\"Row\";\"A\";\"A\";\"B\";\"B\"},{\"Col\";\"X\";\"Y\";\"X\";\"Y\"},{\"V1\",\"V2\";10,1;30,3;20,2;40,4},SUM,1,0,,0)",
        ),
        (
            "BB1",
            "=PIVOTBY({\"A\",\"a\";\"B\",\"b\"},{\"X\",\"x\";\"Y\",\"y\"},{10;20},SUM,2,0,,0)",
        ),
        (
            "BF1",
            "=GROUPBY({\"H1\",\"H2\";\"A\",\"x\";\"A\",\"y\";\"B\",\"x\";\"B\",\"y\"},{\"V\";10;30;20;40},SUM)",
        ),
        ("BO1", "=GROUPBY(BM1:BM2,BN1:BN2,SUM,0,0,-1)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid grouped edge anchor"),
                FormulaText::from_user_input(formula).expect("valid grouped edge formula"),
                None,
            )
            .expect("grouped edge formula mutation");
    }
    for (address, value) in [("BN1", 1.0), ("BN2", 2.0)] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid blank-key value address"),
                CellValue::number(value).expect("finite blank-key value"),
            )
            .expect("blank-key value mutation");
    }
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("BM2").expect("valid blank-key label address"),
            CellValue::Text("A".to_owned()),
        )
        .expect("blank-key label mutation");
    for (address, value) in [("BR2", 0.0), ("BS1", 1.0), ("BS2", 2.0)] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid blank-sort input address"),
                CellValue::number(value).expect("finite blank-sort input"),
            )
            .expect("blank-sort input mutation");
    }
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("BT1").expect("valid blank-sort anchor"),
            FormulaText::from_user_input("=GROUPBY(BR1:BR2,BS1:BS2,SUM,0,0)")
                .expect("valid blank-sort formula"),
            None,
        )
        .expect("blank-sort formula mutation");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    for (address, expected) in [
        ("B2", 0.25),
        ("C2", 0.75),
        ("D2", 1.0),
        ("B3", 1.0 / 3.0),
        ("C3", 2.0 / 3.0),
        ("D3", 1.0),
        ("B4", 0.3),
        ("C4", 0.7),
        ("D4", 1.0),
        ("G2", 0.1),
        ("H2", 0.3),
        ("I2", 0.4),
        ("G3", 0.2),
        ("H3", 0.4),
        ("I3", 0.6),
        ("G4", 0.3),
        ("H4", 0.7),
        ("I4", 1.0),
        ("M1", 20.0),
        ("M2", 10.0),
        ("M3", 30.0),
        ("P1", 1.0),
        ("P2", 2.0),
        ("S1", 1.0),
        ("S2", 2.0),
        ("V2", 1.0),
        ("V3", 2.0),
        ("AA3", 0.25),
        ("AC3", 0.1),
        ("AB4", 0.75),
        ("AC4", 0.3),
        ("AA5", 0.25),
        ("AB5", 0.75),
        ("AC5", 0.4),
        ("AD6", 1.0 / 3.0),
        ("AE7", 2.0 / 3.0),
        ("AF8", 0.6),
        ("AG9", 1.0),
        ("AK3", 0.25),
        ("AL4", 0.75),
        ("AM5", 0.4),
        ("AN6", 1.0 / 3.0),
        ("AO7", 2.0 / 3.0),
        ("AP8", 0.6),
        ("AQ9", 1.0),
        ("AT1", 60.0),
        ("AT2", 50.0),
        ("AT3", 110.0),
        ("AT4", 100.0),
        ("AT5", 1.0),
        ("AT6", 101.0),
        ("AT7", 211.0),
        ("AV1", 1.0),
        ("AV2", 2.0),
        ("AV3", 3.0),
        ("AX2", 10.0),
        ("AY2", 1.0),
        ("AZ2", 30.0),
        ("BA2", 3.0),
        ("AX3", 20.0),
        ("AY3", 2.0),
        ("AZ3", 40.0),
        ("BA3", 4.0),
        ("BD5", 10.0),
        ("BE6", 20.0),
        ("BH1", 10.0),
        ("BH2", 30.0),
        ("BH3", 20.0),
        ("BH4", 40.0),
        ("BH5", 100.0),
        ("BO2", 0.0),
        ("BP1", 2.0),
        ("BP2", 1.0),
        ("BT1", 0.0),
        ("BU1", 2.0),
        ("BT2", 0.0),
        ("BU2", 1.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for (address, expected) in [
        ("K1", "2"),
        ("L1", "b"),
        ("K2", "1"),
        ("L2", "a"),
        ("K3", "1"),
        ("L3", "c"),
        ("O1", "A"),
        ("O2", "B"),
        ("R1", "A"),
        ("R2", "B"),
        ("U1", "Row Field 1"),
        ("V1", "Value 1"),
        ("U2", "A"),
        ("U3", "B"),
        ("AR1", "B"),
        ("AS1", "x"),
        ("AR2", "B"),
        ("AS2", "y"),
        ("AR3", "B"),
        ("AR4", "A"),
        ("AR6", "A"),
        ("AR7", "Grand Total"),
        ("AU1", "A"),
        ("AU2", "B"),
        ("AU3", "Total"),
        ("AX1", "X"),
        ("AY1", "X"),
        ("AZ1", "Y"),
        ("BA1", "Y"),
        ("AW2", "A"),
        ("AW3", "B"),
        ("BD1", "Column Field"),
        ("BD2", "X"),
        ("BE2", "Y"),
        ("BD3", "x"),
        ("BE3", "y"),
        ("BB4", "Row Field 1"),
        ("BC4", "Row Field 2"),
        ("BD4", "Value 1"),
        ("BE4", "Value 1"),
        ("BB5", "A"),
        ("BC5", "a"),
        ("BB6", "B"),
        ("BC6", "b"),
        ("BF1", "A"),
        ("BG1", "x"),
        ("BF2", "A"),
        ("BG2", "y"),
        ("BF3", "B"),
        ("BG3", "x"),
        ("BF4", "B"),
        ("BG4", "y"),
        ("BF5", "Total"),
        ("BO1", "A"),
    ] {
        let expected = if let Ok(number) = expected.parse::<f64>() {
            CellValue::number(number).expect("finite grouped label")
        } else {
            CellValue::Text(expected.to_owned())
        };
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(expected)),
            "unexpected grouped edge label at {address}",
        );
    }
    assert!(materialized_result(&calculation, sheet_id, "O3").is_none());
    assert!(materialized_result(&calculation, sheet_id, "R3").is_none());
    assert!(materialized_result(&calculation, sheet_id, "K4").is_none());
    assert_eq!(
        materialized_result(&calculation, sheet_id, "AW1"),
        Some(&CalculationCellResult::Value(
            CellValue::Text(String::new())
        ))
    );
    assert!(materialized_result(&calculation, sheet_id, "AW4").is_none());
    for address in ["AB3", "AS3", "AS6", "BE5", "BD6", "BG5"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(
                CellValue::Text(String::new())
            )),
            "expected a materialized grouped blank at {address}",
        );
    }
    assert!(materialized_result(&calculation, sheet_id, "BB7").is_none());
    assert!(materialized_result(&calculation, sheet_id, "BF6").is_none());
    assert!(materialized_result(&calculation, sheet_id, "BO3").is_none());
    assert!(materialized_result(&calculation, sheet_id, "BT3").is_none());
}

#[test]
fn v0_1_10_grouped_options_reject_invalid_shapes_domains_and_callables() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "GROUPBY({1;2},{1},SUM)"),
        (1, 2, "GROUPBY({1;2},{1;2},COUNTBLANK)"),
        (1, 3, "GROUPBY({1;2},{1;2},LAMBDA(a,b,c,SUM(a)))"),
        (1, 4, "GROUPBY({1;2},{1;2},SUM,4)"),
        (1, 5, "GROUPBY({1;2},{1;2},SUM,,2)"),
        (1, 6, "GROUPBY({1;2},{1;2},SUM,,,0)"),
        (1, 7, "GROUPBY({1;2},{1;2},SUM,,,,{TRUE,FALSE})"),
        (1, 8, "GROUPBY({1;2},{1;2},SUM,,,,{TRUE;FALSE},2)"),
        (1, 9, "GROUPBY({1,1;1,2},{1;2},SUM,,2,,,1)"),
        (1, 10, "PIVOTBY({1;2},{1;2},{1;2},SUM,,,,,,,5)"),
        (1, 11, "PERCENTOF({1;2},0)"),
        (1, 12, "GROUPBY({1,1;1,2},{1;2},LAMBDA(items,1/0),,2,-3,,1)"),
        (1, 13, "GROUPBY({1;2},{1;2},COUNTBLANK,,,,{FALSE;FALSE})"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=10 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "invalid grouped option in column {column} was accepted",
        );
    }
    assert_eq!(
        calculation.cell(cell_id(11)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::DivisionByZero
        )))
    );
    for column in [12, 13] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "grouped validation in column {column} did not fail before execution",
        );
    }
}

#[test]
fn v0_1_10_grouping_amplification_respects_calculation_limits() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("A1").expect("valid bounded pivot anchor"),
            FormulaText::from_user_input("=IFERROR(PIVOTBY({1;2;3;4},{1;2;3;4},{1;2;3;4},SUM),42)")
                .expect("valid bounded pivot formula"),
            None,
        )
        .expect("bounded pivot formula mutation");
    let array_limits = CalculationLimits::default()
        .with_max_array_cells(20)
        .expect("positive grouped array limit");
    let array_limited = calculate_workbook(
        draft.workbook(),
        CalculationOptions::default().with_limits(array_limits),
    );
    let anchor = CalculationCellId::new(
        sheet_id,
        CellAddress::from_a1("A1").expect("valid bounded pivot anchor"),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = array_limited.cell(anchor) else {
        panic!("pivot output amplification must consume the array-cell budget");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_array_cells"));

    let iteration_limits = CalculationLimits::default()
        .with_max_function_iterations(20)
        .expect("positive grouped iteration limit");
    let iteration_limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "IFERROR(GROUPBY({1;2;3;4;5;6},{1;2;3;4;5;6},SUM),42)")]),
        CalculationOptions::default().with_limits(iteration_limits),
    );
    assert_issue(
        &iteration_limited,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let text_limits = CalculationLimits::default()
        .with_max_text_bytes(1)
        .expect("positive grouped text limit");
    let text_limited = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "GROUPBY({1;2},{1;2},SUM)"),
            (1, 2, "GROUPBY({1,1;2,2},{1;2},SUM,2,0)"),
            (1, 3, "GROUPBY({1,1;2,2},{1;2},SUM,0,2)"),
        ]),
        CalculationOptions::default().with_limits(text_limits),
    );
    assert_issue(
        &text_limited,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
    );
    assert_issue(
        &text_limited,
        2,
        CalculationIssueCode::ResourceLimitExceeded,
    );
    assert_issue(
        &text_limited,
        3,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    for max_function_iterations in 100..=220 {
        let limits = CalculationLimits::default()
            .with_max_function_iterations(max_function_iterations)
            .expect("positive relative-set iteration limit");
        let relative = calculate_workbook(
            &workbook_with_formulas(&[
                (
                    1,
                    1,
                    "PIVOTBY({1,1;1,2;2,1;2,2},{1;1;2;2},{10;20;30;40},SUM,,,,,,,0)",
                ),
                (
                    1,
                    2,
                    "PIVOTBY({1,1;1,2;2,1;2,2},{1;1;2;2},{10;20;30;40},SUM,,,,,,,4)",
                ),
            ]),
            CalculationOptions::default().with_limits(limits),
        );
        assert_eq!(
            relative.cell(cell_id(1)),
            relative.cell(cell_id(2)),
            "relative_to changed unary aggregate work at iteration limit {max_function_iterations}",
        );
    }
}

#[test]
fn lambda_core_captures_lexical_bindings_and_rejects_callable_coercion() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "LET(a,2,f,LAMBDA(x,x+a),f(3))"),
        (1, 2, "LET(a,2,f,LAMBDA(x,x+a),LET(a,10,f(3)))"),
        (1, 3, "LAMBDA(x,x+1)"),
        (1, 4, "LET(f,LAMBDA(x,x+1),f())"),
        (1, 5, "LET(f,LAMBDA(x,x+1),f(3))"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 5.0, 0.0);
    assert_number(&calculation, 2, 5.0, 0.0);
    assert!(matches!(
        calculation.cell(cell_id(3)),
        Some(CalculationCellResult::Value(CellValue::Error(_)))
    ));
    assert!(matches!(
        calculation.cell(cell_id(4)),
        Some(CalculationCellResult::Value(CellValue::Error(_)))
    ));
    assert_number(&calculation, 5, 4.0, 0.0);
}

#[test]
fn named_lambda_calls_and_finite_recursion_use_the_defined_name_registry() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "AddOne(4)"),
            (1, 2, "Factorial(5)"),
            (1, 3, "Adder(2)"),
        ],
        &[
            ("AddOne", "LAMBDA(x,x+1)"),
            ("Factorial", "LAMBDA(n,IF(n<=1,1,n*Factorial(n-1)))"),
            ("Adder", "LAMBDA(x,x+A1)"),
        ],
    );
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 5.0, 0.0);
    assert_number(&calculation, 2, 120.0, 0.0);
    assert_number(&calculation, 3, 7.0, 0.0);
}

#[test]
fn workbook_lambda_bodies_keep_their_defined_name_resolution_scope() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    for defined_name in [
        DefinedName::new(
            "Entry",
            DefinedNameScope::Workbook,
            FormulaText::from_xlsx("LAMBDA(n,Base(n))").expect("valid formula"),
            false,
        )
        .expect("valid workbook lambda"),
        DefinedName::new(
            "Base",
            DefinedNameScope::Workbook,
            FormulaText::from_xlsx("LAMBDA(n,IF(n<=0,0,Base(n-1)+1))").expect("valid formula"),
            false,
        )
        .expect("valid recursive workbook lambda"),
        DefinedName::new(
            "Base",
            DefinedNameScope::Sheet(sheet_id),
            FormulaText::from_xlsx("LAMBDA(n,NO_SUCH_FUNCTION())").expect("valid formula"),
            false,
        )
        .expect("valid sheet-local shadow"),
    ] {
        draft
            .set_defined_name(defined_name)
            .expect("defined name edit");
    }
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("A1").expect("valid address"),
            FormulaText::from_xlsx("Entry(3)").expect("valid formula"),
        )
        .expect("formula edit");

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let usage = scan_function_usage(draft.workbook());
    assert!(
        usage
            .entries()
            .iter()
            .all(|entry| entry.name() != "NO_SUCH_FUNCTION")
    );
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_number(&calculation, 1, 3.0, 0.0);
}

#[test]
fn workbook_value_names_keep_their_defined_name_resolution_scope() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = SheetId::new(1).expect("default sheet ID");
    for (name, scope, formula) in [
        ("EntryValue", DefinedNameScope::Workbook, "BaseValue+1"),
        ("BaseValue", DefinedNameScope::Workbook, "1"),
        (
            "BaseValue",
            DefinedNameScope::Sheet(sheet_id),
            "NO_SUCH_FUNCTION()",
        ),
        (
            "EntryReference",
            DefinedNameScope::Workbook,
            "BaseReference",
        ),
        (
            "BaseReference",
            DefinedNameScope::Workbook,
            "Sheet1!$B$1:$B$2",
        ),
        (
            "BaseReference",
            DefinedNameScope::Sheet(sheet_id),
            "Sheet1!$B$3:$B$4",
        ),
        ("EntryArray", DefinedNameScope::Workbook, "BaseArray"),
        ("BaseArray", DefinedNameScope::Workbook, "{1,2}"),
        ("BaseArray", DefinedNameScope::Sheet(sheet_id), "{9,9}"),
    ] {
        draft
            .set_defined_name(
                DefinedName::new(
                    name,
                    scope,
                    FormulaText::from_xlsx(formula).expect("valid formula"),
                    false,
                )
                .expect("valid defined name"),
            )
            .expect("defined name edit");
    }
    for (address, value) in [("B1", 2.0), ("B2", 3.0), ("B3", 40.0), ("B4", 50.0)] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid address"),
                CellValue::number(value).expect("finite value"),
            )
            .expect("literal edit");
    }
    for (address, formula) in [
        ("A1", "EntryValue"),
        ("C1", "SUM(EntryReference)"),
        ("D1", "SUM(EntryArray)"),
    ] {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid address"),
                FormulaText::from_xlsx(formula).expect("valid formula"),
            )
            .expect("formula edit");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_number(&calculation, 1, 2.0, 0.0);
    assert_number(&calculation, 3, 5.0, 0.0);
    assert_number(&calculation, 4, 3.0, 0.0);
}

#[test]
fn lambda_calls_preserve_arrays_and_nested_callable_values() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(LET(f,LAMBDA(x,x+1),f({1,2})))"),
        (
            1,
            2,
            "LET(make,LAMBDA(a,LAMBDA(x,x+a)),apply,LAMBDA(f,f(3)),apply(make(2)))",
        ),
        (1, 3, "LET(apply,LAMBDA(f,f(3)),apply(LAMBDA(x,x+1)))"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 5.0, 0.0);
    assert_number(&calculation, 2, 5.0, 0.0);
    assert_number(&calculation, 3, 4.0, 0.0);
}

#[test]
fn callable_bindings_shadow_builtins_without_falling_through() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "LET(SUM,2,SUM(1,2))"),
            (1, 2, "ScalarName(1)"),
            (1, 3, "LoopCall(1)"),
            (1, 4, "LET(f,2,f(Loop))"),
            (1, 5, "_xleta.SUM(1)"),
        ],
        &[
            ("ScalarName", "2"),
            ("LoopCall", "LoopCall"),
            ("Loop", "Loop"),
            ("SUM", "SUM"),
        ],
    );
    let capabilities = scan_formula_capabilities(&workbook);
    assert!(capabilities.is_supported(), "{capabilities:?}");
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=5 {
        assert!(
            matches!(
                calculation.cell(cell_id(column)),
                Some(CalculationCellResult::Value(CellValue::Error(
                    ExcelError::Value
                )))
            ),
            "{:?}",
            calculation.cell(cell_id(column))
        );
    }
    let usage = scan_function_usage(&workbook);
    assert!(
        usage
            .entries()
            .iter()
            .all(|entry| !matches!(entry.name(), "SUM" | "SCALARNAME" | "LOOPCALL" | "LOOP")),
        "{:?}",
        usage.entries()
    );
}

#[test]
fn defined_call_targets_shadow_every_special_builtin_dispatch_path() {
    let names = [
        ("MAP", "LAMBDA(x,callback,41)"),
        ("BYROW", "LAMBDA(x,42)"),
        ("BYCOL", "LAMBDA(x,43)"),
        ("REDUCE", "LAMBDA(x,44)"),
        ("SCAN", "LAMBDA(x,45)"),
        ("MAKEARRAY", "LAMBDA(x,46)"),
        ("INDEX", "LAMBDA(x,47)"),
        ("OFFSET", "LAMBDA(x,48)"),
        ("INDIRECT", "LAMBDA(x,49)"),
        ("TODAY", "LAMBDA(50)"),
    ];
    let formulas = [
        "MAP(1,LAMBDA(x,x))",
        "MAP(1,LAMBDA(x,x))+0",
        "BYROW(1)",
        "BYROW(1)+0",
        "BYCOL(1)",
        "BYCOL(1)+0",
        "REDUCE(1)",
        "REDUCE(1)+0",
        "SCAN(1)",
        "SCAN(1)+0",
        "MAKEARRAY(1)",
        "MAKEARRAY(1)+0",
        "INDEX(1)",
        "INDEX(1)+0",
        "OFFSET(1)",
        "OFFSET(1)+0",
        "INDIRECT(1)",
        "INDIRECT(1)+0",
        "TODAY()",
        "TODAY()+0",
    ];
    let formulas = formulas
        .iter()
        .enumerate()
        .map(|(index, formula)| (1, index as u32 + 1, *formula))
        .collect::<Vec<_>>();
    let workbook = workbook_with_formulas_and_names(&formulas, &names);

    let capabilities = scan_formula_capabilities(&workbook);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let usage = scan_function_usage(&workbook);
    assert!(
        usage.entries().iter().all(|entry| entry.name() == "LAMBDA"),
        "{:?}",
        usage.entries()
    );
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (pair, expected) in (1_u32..=10).zip(41_u32..=50) {
        let direct = pair * 2 - 1;
        let composed = pair * 2;
        assert_number(&calculation, direct, f64::from(expected), 0.0);
        assert_number(&calculation, composed, f64::from(expected), 0.0);
    }

    let syntax_workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "LET(1)"),
            (1, 2, "SUM(LET(1))"),
            (1, 3, "LAMBDA(1)"),
            (1, 4, "SUM(LAMBDA(1))"),
        ],
        &[("LET", "LAMBDA(x,51)"), ("LAMBDA", "_xlfn.LAMBDA(x,52)")],
    );
    let capabilities = scan_formula_capabilities(&syntax_workbook);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let syntax_calculation = calculate_workbook(&syntax_workbook, CalculationOptions::default());
    for (column, expected) in [(1, 51.0), (2, 51.0), (3, 52.0), (4, 52.0)] {
        assert_number(&syntax_calculation, column, expected, 0.0);
    }
}

#[test]
fn callable_range_endpoints_do_not_consume_cell_references_during_lookahead() {
    let workbook = workbook_with_formulas_and_names(
        &[(1, 1, "2"), (2, 1, "5"), (1, 2, "SUM(A1:MAP(0))")],
        &[("MAP", "LAMBDA(value,A2)")],
    );

    let capabilities = scan_formula_capabilities(&workbook);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 2, 7.0, 0.0);
}

#[test]
fn lambda_invoke_and_iteration_helpers_share_the_callable_kernel() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "LAMBDA(x,x+1)(5)"),
        (1, 2, "LAMBDA(x,y,ISOMITTED(y))(1,)"),
        (1, 3, "SUM(BYROW({1,2;3,4},LAMBDA(row,SUM(row))))"),
        (1, 4, "SUM(BYCOL({1,2;3,4},LAMBDA(col,SUM(col))))"),
        (1, 5, "REDUCE(0,{1,2,3},LAMBDA(acc,value,acc+value))"),
        (1, 6, "SUM(SCAN(0,{1,2,3},LAMBDA(acc,value,acc+value)))"),
        (1, 7, "SUM(MAKEARRAY(2,3,LAMBDA(row,column,row*10+column)))"),
        (1, 8, "SUM(LAMBDA(value,value+1)({1,2}))"),
        (1, 9, "_xlfn.LAMBDA(_xlpm.value,_xlpm.value+1)(5)"),
        (1, 10, "BYROW({1,2},LAMBDA(row,row))"),
        (1, 11, "MAP({1,2},LAMBDA(value,{value,value}))"),
        (1, 12, "MAKEARRAY(1,1,LAMBDA(row,column,{row,column}))"),
        (1, 13, "_XlFn._xLwS.LaMbDa(x,x+1)(5)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 6.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );
    assert_number(&calculation, 3, 10.0, 0.0);
    assert_number(&calculation, 4, 10.0, 0.0);
    assert_number(&calculation, 5, 6.0, 0.0);
    assert_number(&calculation, 6, 10.0, 0.0);
    assert_number(&calculation, 7, 102.0, 0.0);
    assert_number(&calculation, 8, 5.0, 0.0);
    assert_number(&calculation, 9, 6.0, 0.0);
    assert_number(&calculation, 13, 6.0, 0.0);
    for column in 10..=12 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Calculation
            )))
        );
    }
}

#[test]
fn builtin_aggregates_use_the_shared_typed_callable_kernel() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(BYROW({1,2;3,4},SUM))"),
        (1, 2, "SUM(BYCOL({1,2;3,4},AVERAGE))"),
        (1, 3, "SUM(MAP({1,2},{10,20},SUM))"),
        (1, 4, "REDUCE(0,{1,2,3},SUM)"),
        (1, 5, "SUM(SCAN(0,{1,2,3},SUM))"),
        (1, 6, "SUM(MAKEARRAY(2,2,SUM))"),
        (1, 7, "SUM(BYROW({1,TRUE;2,FALSE},COUNT))"),
        (1, 8, "SUM(MAP({TRUE,FALSE},SUM))"),
        (1, 9, "SUM(BYROW({1,2;3,4},_xleta.SUM))"),
        (1, 10, "LET(f,_xleta.SUM,f(1,2))"),
        (1, 11, "SUM(BYROW({1,2;3,4},MIN))"),
        (1, 12, "SUM(BYROW({1,2;3,4},MAX))"),
        (1, 13, "SUM(MAP({1,2},COUNTA))"),
        (1, 14, "SUM(BYROW({1,2;3,4},PRODUCT))"),
        (1, 15, "_xleta.SUM(1,2)"),
        (1, 16, "_xleta.COUNT(#N/A)"),
        (1, 17, "_xleta.COUNTA(#N/A)"),
        (1, 18, "COUNT(#N/A)"),
        (1, 19, "COUNTA(#N/A)"),
        (1, 20, "_xleta.SUM(#N/A)"),
        (1, 21, "SUM({TRUE})"),
        (1, 22, "_xleta.SUM({TRUE})"),
        (1, 23, "SUM({\"1\"})"),
        (1, 24, "_xleta.SUM({\"1\"})"),
        (1, 25, "_xleta.SUM(LET(value,{TRUE},value))"),
    ]);

    let capabilities = scan_formula_capabilities(&workbook);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [
        (1, 10.0),
        (2, 5.0),
        (3, 33.0),
        (4, 6.0),
        (5, 10.0),
        (6, 12.0),
        (7, 2.0),
        (8, 1.0),
        (9, 10.0),
        (10, 3.0),
        (11, 4.0),
        (12, 6.0),
        (13, 2.0),
        (14, 14.0),
        (15, 3.0),
        (16, 0.0),
        (17, 1.0),
        (18, 0.0),
        (19, 1.0),
        (21, 0.0),
        (22, 0.0),
        (23, 0.0),
        (24, 0.0),
        (25, 0.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_eq!(
        calculation.cell(cell_id(20)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );

    let usage = scan_function_usage(&workbook);
    let names = usage
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect::<BTreeSet<_>>();
    for expected in [
        "AVERAGE",
        "BYCOL",
        "BYROW",
        "COUNT",
        "COUNTA",
        "LET",
        "MAKEARRAY",
        "MAP",
        "MAX",
        "MIN",
        "PRODUCT",
        "REDUCE",
        "SCAN",
        "SUM",
    ] {
        assert!(names.contains(expected), "missing {expected}: {names:?}");
    }
    assert!(names.iter().all(|name| !name.contains("XLETA")));
}

#[test]
fn invalid_typed_builtin_invocations_do_not_reach_arguments_in_static_or_runtime_paths() {
    let arguments = std::iter::once("A1")
        .chain(std::iter::once("NO_SUCH_FUNCTION()"))
        .chain(std::iter::once("NOW()"))
        .chain(std::iter::repeat_n("1", 253))
        .collect::<Vec<_>>()
        .join(",");
    let direct = format!("_xleta.SUM({arguments})");
    let parenthesized = format!("(_xleta.SUM)({arguments})");
    let workbook = workbook_with_formulas(&[(1, 1, "7"), (1, 2, &direct), (1, 3, &parenthesized)]);
    let limits = CalculationLimits::default()
        .with_max_dependency_edges(1)
        .expect("positive dependency boundary");
    let options = CalculationOptions::default().with_limits(limits);

    assert!(scan_formula_capabilities_with_options(&workbook, options).is_supported());
    let calculation = calculate_workbook(&workbook, options);
    assert_number(&calculation, 1, 7.0, 0.0);
    for column in [2, 3] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }

    let usage = scan_function_usage(&workbook);
    let names = usage
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect::<BTreeSet<_>>();
    assert!(!names.contains("NO_SUCH_FUNCTION"), "{names:?}");
    assert!(!names.contains("NOW"), "{names:?}");

    let local_shadow = workbook_with_formulas(&[
        (1, 1, "7"),
        (1, 2, "LET(SUM,2,_xleta.SUM(A1+NO_SUCH_FUNCTION()+NOW()))"),
    ]);
    let defined_shadow = workbook_with_formulas_and_names(
        &[
            (1, 1, "7"),
            (1, 2, "_xleta.SUM(A1+NO_SUCH_FUNCTION()+NOW())"),
        ],
        &[("SUM", "42")],
    );
    for shadowed in [&local_shadow, &defined_shadow] {
        assert!(scan_formula_capabilities_with_options(shadowed, options).is_supported());
        let calculation = calculate_workbook(shadowed, options);
        assert_eq!(
            calculation.cell(cell_id(2)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
        let usage = scan_function_usage(shadowed);
        let names = usage
            .entries()
            .iter()
            .map(|entry| entry.name())
            .collect::<BTreeSet<_>>();
        assert!(!names.contains("NO_SUCH_FUNCTION"), "{names:?}");
        assert!(!names.contains("NOW"), "{names:?}");
    }
}

#[test]
fn builtin_callable_normalization_preserves_local_and_defined_name_shadowing() {
    let local = workbook_with_formulas(&[
        (
            1,
            1,
            "LET(SUM,LAMBDA(row,41),SUMPRODUCT(BYROW({1,2;3,4},SUM)))",
        ),
        (1, 2, "LET(SUM,2,BYROW({1,2},SUM))"),
        (1, 3, "LET(SUM,LAMBDA(value,43),_xleta.SUM(1))"),
    ]);
    assert!(scan_formula_capabilities(&local).is_supported());
    let calculation = calculate_workbook(&local, CalculationOptions::default());
    assert_number(&calculation, 1, 82.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
    assert_number(&calculation, 3, 43.0, 0.0);
    assert!(
        scan_function_usage(&local)
            .entries()
            .iter()
            .all(|entry| entry.name() != "SUM")
    );

    let defined = workbook_with_formulas_and_names(
        &[
            (1, 1, "SUMPRODUCT(BYROW({1,2;3,4},SUM))"),
            (1, 2, "SUMPRODUCT(BYROW({1,2;3,4},_xleta.SUM))"),
            (1, 26, "40+2"),
        ],
        &[("SUM", "LAMBDA(row,Z1)")],
    );
    let capabilities = scan_formula_capabilities(&defined);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let calculation = calculate_workbook(&defined, CalculationOptions::default());
    assert_number(&calculation, 1, 84.0, 0.0);
    assert_number(&calculation, 2, 84.0, 0.0);
    assert!(
        scan_function_usage(&defined)
            .entries()
            .iter()
            .all(|entry| entry.name() != "SUM")
    );

    let alias = workbook_with_formulas_and_names(
        &[(1, 1, "SUM(1)"), (1, 2, "_xleta.SUM(1)")],
        &[("F", "LAMBDA(value,value+1)"), ("SUM", "F")],
    );
    assert!(scan_formula_capabilities(&alias).is_supported());
    let calculation = calculate_workbook(&alias, CalculationOptions::default());
    assert_number(&calculation, 1, 2.0, 0.0);
    assert_number(&calculation, 2, 2.0, 0.0);

    let callable_result = workbook_with_formulas_and_names(
        &[(1, 1, "SUM(1)"), (1, 2, "_xleta.SUM(1)")],
        &[("F", "LAMBDA(x,LAMBDA(y,y))"), ("SUM", "F")],
    );
    let calculation = calculate_workbook(&callable_result, CalculationOptions::default());
    for column in [1, 2] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Calculation
            )))
        );
    }

    let array_provenance = workbook_with_formulas_and_names(
        &[
            (1, 1, "SUM(({TRUE}))"),
            (1, 2, "_xleta.SUM(({TRUE}))"),
            (1, 3, "SUM(LET(value,{TRUE},value))"),
            (1, 4, "_xleta.SUM(LET(value,{TRUE},value))"),
            (1, 5, "SUM(X)"),
            (1, 6, "_xleta.SUM(X)"),
        ],
        &[("X", "{TRUE}")],
    );
    let calculation = calculate_workbook(&array_provenance, CalculationOptions::default());
    for column in 1..=6 {
        assert_number(&calculation, column, 0.0, 0.0);
    }

    let error_shadow = workbook_with_formulas_and_names(
        &[(1, 1, "SUM(1)"), (1, 2, "_xleta.SUM(1)")],
        &[("SUM", "#N/A")],
    );
    let calculation = calculate_workbook(&error_shadow, CalculationOptions::default());
    for column in [1, 2] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::NotAvailable
            )))
        );
    }
}

#[test]
fn callable_aliases_share_static_reachability_arity_and_callee_evaluation() {
    let alias = workbook_with_formulas_and_names(
        &[(1, 1, "1"), (1, 2, "2"), (1, 3, "SUM(B1)")],
        &[
            ("F", "LAMBDA(x,A1+x+NO_SUCH_FUNCTION()+NOW())"),
            ("SUM", "F"),
        ],
    );
    let capabilities = scan_formula_capabilities(&alias);
    assert!(!capabilities.is_supported());
    let usage = scan_function_usage(&alias);
    let names = usage
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect::<BTreeSet<_>>();
    assert!(names.contains("NO_SUCH_FUNCTION"), "{names:?}");
    assert!(names.contains("NOW"), "{names:?}");
    let calculation = calculate_workbook(&alias, CalculationOptions::default());
    assert_issue(&calculation, 3, CalculationIssueCode::UnsupportedFunction);

    let defined_arity = workbook_with_formulas_and_names(
        &[
            (1, 1, "SUM(A1+NO_SUCH_FUNCTION()+NOW(),1)"),
            (1, 2, "_xleta.SUM(A1+NO_SUCH_FUNCTION()+NOW(),1)"),
        ],
        &[("SUM", "LAMBDA(value,value)")],
    );
    let local_arity = workbook_with_formulas(&[
        (
            1,
            1,
            "LET(SUM,LAMBDA(value,value),SUM(A1+NO_SUCH_FUNCTION()+NOW(),1))",
        ),
        (
            1,
            2,
            "LET(SUM,LAMBDA(value,value),_xleta.SUM(A1+NO_SUCH_FUNCTION()+NOW(),1))",
        ),
        (
            1,
            3,
            "LET(X,2,SUM,X,_xleta.SUM(A1+NO_SUCH_FUNCTION()+NOW()))",
        ),
    ]);
    for workbook in [&defined_arity, &local_arity] {
        assert!(scan_formula_capabilities(workbook).is_supported());
        let usage = scan_function_usage(workbook);
        let names = usage
            .entries()
            .iter()
            .map(|entry| entry.name())
            .collect::<BTreeSet<_>>();
        assert!(!names.contains("NO_SUCH_FUNCTION"), "{names:?}");
        assert!(!names.contains("NOW"), "{names:?}");
        let calculation = calculate_workbook(workbook, CalculationOptions::default());
        for column in 1..=workbook.sheets()[0].len() as u32 {
            assert_eq!(
                calculation.cell(cell_id(column)),
                Some(&CalculationCellResult::Value(CellValue::Error(
                    ExcelError::Value
                )))
            );
        }
    }
}

#[test]
fn lambda_callable_values_errors_and_reduce_seeding_match_excel_contracts() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "LAMBDA(value,42)(1/0)"),
        (1, 2, "LAMBDA(value,)(1)"),
        (1, 3, "LET(f,LAMBDA(v,v),f+1)"),
        (1, 4, "LET(f,LAMBDA(v,v+1),SUM(MAP({1,2},f)))"),
        (1, 5, "LET(f,LAMBDA(a,v,a+v),REDUCE(0,{1,2},f))"),
        (1, 6, "REDUCE(,{2,3},LAMBDA(a,b,a*b))"),
        (
            1,
            7,
            "SUM(REDUCE(0,{1,2},LAMBDA(acc,value,VSTACK(acc,value))))",
        ),
        (1, 8, "_XlFn.LaMbDa(value,value+1)(5)"),
        (1, 9, "MAP({0},LAMBDA(value,J1:K1))"),
        (1, 10, "1"),
        (1, 11, "2"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_eq!(
        calculation.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::DivisionByZero
        )))
    );
    for column in [2, 3] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
    assert_number(&calculation, 4, 5.0, 0.0);
    assert_number(&calculation, 5, 3.0, 0.0);
    assert_number(&calculation, 6, 6.0, 0.0);
    assert_number(&calculation, 7, 3.0, 0.0);
    assert_number(&calculation, 8, 6.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(9)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Calculation
        )))
    );

    let limits = CalculationLimits::default()
        .with_max_function_iterations(1)
        .expect("nonzero helper iteration limit");
    let limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "BYCOL({1;2},LAMBDA(column,1))")]),
        CalculationOptions::default().with_limits(limits),
    );
    assert_issue(&limited, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn helper_callbacks_preserve_engine_issues_and_cumulative_work_limits() {
    let text_limits = CalculationLimits::default()
        .with_max_text_bytes(3)
        .expect("nonzero text limit");
    let text_limited = calculate_workbook(
        &workbook_with_formulas(&[
            (
                1,
                1,
                "IFERROR(MAP({1},LET(x,\"ab\"&\"cd\",LAMBDA(v,v))),42)",
            ),
            (1, 2, "IFERROR(MAP({1},LAMBDA(v,{v,\"ab\"&\"cd\"})),42)"),
        ]),
        CalculationOptions::default().with_limits(text_limits),
    );
    assert_issue(
        &text_limited,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
    );
    assert_issue(
        &text_limited,
        2,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let builtin_limited = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "IFERROR(BYROW({1,\"ab\"&\"cd\"},COUNTA),42)"),
            (1, 2, "IFERROR(REDUCE(0,{1,\"ab\"&\"cd\"},COUNTA),42)"),
            (1, 3, "\"ab\"&\"cd\""),
            (1, 4, "IFERROR(_xleta.COUNTA(C1),42)"),
        ]),
        CalculationOptions::default().with_limits(text_limits),
    );
    for column in [1, 2, 3] {
        assert_issue(
            &builtin_limited,
            column,
            CalculationIssueCode::ResourceLimitExceeded,
        );
    }
    assert_issue(
        &builtin_limited,
        4,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let work_limits = CalculationLimits::default()
        .with_max_function_iterations(20)
        .expect("nonzero function-work limit");
    let work_limited = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "IFERROR(REDUCE(0,{1,2,3,4},LAMBDA(a,v,VSTACK(a,v))),42)",
        )]),
        CalculationOptions::default().with_limits(work_limits),
    );
    assert_issue(
        &work_limited,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let callable = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "REDUCE(0,{1},LAMBDA(a,v,LAMBDA(x,x)))")]),
        CalculationOptions::default(),
    );
    assert_eq!(
        callable.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Calculation
        )))
    );

    let capture_subject = "a".repeat(1_000);
    let amplified_pattern = format!("{}.*", "(?=(.*))".repeat(100));
    let amplified_formula =
        format!("REGEXEXTRACT(\"{capture_subject}\",\"{amplified_pattern}\",2)");
    let amplified_limits = CalculationLimits::default()
        .with_max_function_iterations(250_000)
        .expect("positive aggregate capture-copy limit");
    let amplified = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, amplified_formula.as_str())]),
        CalculationOptions::default().with_limits(amplified_limits),
    );
    assert_issue(&amplified, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn named_lambdas_are_isolated_and_helpers_accept_typed_callables() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "LET(secret,10,GetOuter(1))"),
            (1, 2, "SUM(BYROW({1,2;3,4},SumRow))"),
            (1, 3, "Broken(1)"),
        ],
        &[
            ("GetOuter", "LAMBDA(x,secret+x)"),
            ("SumRow", "LAMBDA(row,SUM(row))"),
            ("Broken", "LAMBDA(x,NO_SUCH_FUNCTION(x))"),
        ],
    );
    let report = scan_formula_capabilities(&workbook);
    assert_capability_issue(
        &report,
        1,
        CalculationIssueCode::UnsupportedName,
        Some("secret"),
    );
    assert_capability_issue(
        &report,
        3,
        CalculationIssueCode::UnsupportedFunction,
        Some("NO_SUCH_FUNCTION"),
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 2, 10.0, 0.0);
}

#[test]
fn named_lambda_bodies_do_not_hide_ordinary_defined_name_cycles() {
    let workbook = workbook_with_formulas_and_names(
        &[(1, 1, "Broken(1)")],
        &[("Broken", "LAMBDA(x,Loop)"), ("Loop", "Loop")],
    );
    let report = scan_formula_capabilities(&workbook);
    assert_capability_issue(&report, 1, CalculationIssueCode::UnsupportedName, None);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&calculation, 1, CalculationIssueCode::UnsupportedName);
}

#[test]
fn builtin_named_lambda_shadows_do_not_hide_ordinary_name_cycles() {
    let workbook = workbook_with_formulas_and_names(
        &[(1, 1, "MAP(0,LAMBDA(x,x))")],
        &[("MAP", "LAMBDA(x,callback,Loop)"), ("Loop", "Loop")],
    );
    let report = scan_formula_capabilities(&workbook);
    assert_capability_issue(&report, 1, CalculationIssueCode::UnsupportedName, None);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&calculation, 1, CalculationIssueCode::UnsupportedName);
}

#[test]
fn function_usage_counts_lambda_bodies_without_user_callable_names() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "LET(f,LAMBDA(x,SUM(x)),f(1))"),
            (1, 2, "RowTotal({1,2})"),
        ],
        &[("RowTotal", "LAMBDA(row,SUM(row))")],
    );
    let report = scan_function_usage(&workbook);
    let names = report
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect::<BTreeSet<_>>();

    assert!(names.contains("LET"));
    assert!(names.contains("LAMBDA"));
    assert!(names.contains("SUM"));
    assert!(!names.contains("F"));
    assert!(!names.contains("ROWTOTAL"));
    assert!(report.is_fully_supported());
}

#[test]
fn unresolved_structured_and_non_dynamic_spill_references_are_excel_errors() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Table1[Amount])"),
        (1, 2, "A1#"),
        (1, 3, "LAMBDA(x,x+1)"),
        (1, 4, "SUMPRODUCT(MAP({1,2},LAMBDA(x,x+1)))"),
        (1, 5, "MAP({1},_XlFn._xLwS.LaMbDa(x,NO_SUCH_FUNCTION()))"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(1))
            .expect("structured-reference capability entry")
            .capability(),
        FormulaCapability::Supported
    ));
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(2))
            .expect("spill-reference capability entry")
            .capability(),
        FormulaCapability::Supported
    ));
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(3))
            .expect("LAMBDA capability entry")
            .capability(),
        FormulaCapability::Supported
    ));
    assert_capability_issue(
        &report,
        5,
        CalculationIssueCode::UnsupportedFunction,
        Some("NO_SUCH_FUNCTION"),
    );
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(4))
            .expect("MAP capability entry")
            .capability(),
        FormulaCapability::Supported
    ));

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert!(matches!(
        calculation.cell(cell_id(1)),
        Some(CalculationCellResult::Value(CellValue::Error(_)))
    ));
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
    assert!(matches!(
        calculation.cell(cell_id(3)),
        Some(CalculationCellResult::Value(_))
    ));
    assert_issue(&calculation, 5, CalculationIssueCode::UnsupportedFunction);
}

#[test]
fn structured_and_external_references_are_typed_before_resolution() {
    // Structured and external workbook references have distinct typed syntax nodes. Structured
    // references resolve to Excel errors when their table context is absent, while external
    // workbooks remain an unsupported capability and malformed brackets remain parse failures.
    let workbook = workbook_with_formulas(&[
        (1, 1, "Table1[Amount]"),
        (1, 2, "SUM([@Amount])"),
        (1, 3, "Table1[[#Headers],[Amount]]"),
        (1, 4, "SUM(Table1[[Col1]:[Col2]])"),
        (1, 5, "COUNTA(Table1[#Data])"),
        (1, 6, "[1]Sheet1!A1"),
        (1, 7, "[Book1.xlsx]Sheet1!A1"),
        (1, 8, "[1]!Name"),
        (1, 9, "SUM(Table1[Amount"),
        (1, 10, "Table1['[odd']name]"),
        (1, 11, "_xleta.COUNTA(Table1[#Data])"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    for column in 1..=5 {
        assert!(matches!(
            report
                .entries()
                .iter()
                .find(|entry| entry.cell() == cell_id(column))
                .expect("structured-reference capability entry")
                .capability(),
            FormulaCapability::Supported
        ));
    }
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(11))
            .expect("typed structured-reference capability entry")
            .capability(),
        FormulaCapability::Supported
    ));
    for column in 6..=8 {
        assert_capability_issue_code(&report, column, CalculationIssueCode::UnsupportedExpression);
    }
    assert_capability_issue_code(&report, 9, CalculationIssueCode::ParseError);
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(10))
            .expect("escaped structured-reference capability entry")
            .capability(),
        FormulaCapability::Supported
    ));

    // Calculate and scan must agree: the same cells are unavailable for the same reason.
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=5 {
        assert!(matches!(
            calculation.cell(cell_id(column)),
            Some(CalculationCellResult::Value(CellValue::Error(_)))
        ));
    }
    for column in 6..=8 {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::UnsupportedExpression,
        );
    }
    assert_issue(&calculation, 9, CalculationIssueCode::ParseError);
    assert!(matches!(
        calculation.cell(cell_id(10)),
        Some(CalculationCellResult::Value(CellValue::Error(_)))
    ));
    assert_eq!(calculation.cell(cell_id(11)), calculation.cell(cell_id(5)));
}

#[test]
fn three_d_aggregates_resolve_in_tab_order_across_quoted_prefixes() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Sheet1:Sheet3!Z1)"),
            (1, 2, "SUM('Sheet1:Sheet3'!Z1)"),
            (1, 3, "SUM(Sheet1:'Sheet3'!Z1)"),
            (1, 4, "IFERROR(SUM('Sheet1:Sheet3'!Z1),42)"),
            (1, 5, "'Sheet2'!B1+1"),
            (1, 6, "SUM(Sheet1:Sheet3!Z:Z)"),
        ],
        // A defined name that shadows the start sheet must not turn the 3-D
        // reference into an ordinary range over the named rect.
        &[("Sheet1", "Sheet3!Z1:Z2")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=4 {
        assert_number(&calculation, column, 111.0, 0.0);
    }
    assert_number(&calculation, 5, 11.0, 0.0);
    assert_number(&calculation, 6, 333.0, 0.0);
}

#[test]
fn typed_and_aliased_builtin_aggregates_inherit_the_descriptor_three_d_policy() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Sheet1:Sheet3!Z1)"),
            (1, 2, "_xleta.SUM(Sheet1:Sheet3!Z1)"),
            (1, 3, "LET(F,_xleta.SUM,F(Sheet1:Sheet3!Z1))"),
            (1, 4, "F(Sheet1:Sheet3!Z1)"),
        ],
        &[("F", "_xleta.SUM")],
    );
    let capabilities = scan_formula_capabilities(&workbook);
    assert!(capabilities.is_supported(), "{:?}", capabilities.entries());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=4 {
        assert_number(&calculation, column, 111.0, 0.0);
    }
}

#[test]
fn three_d_consumers_share_scanner_and_excel_error_policy() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "INDEX(Sheet1:Sheet3!Z1:Z2,1)"),
            (1, 2, "VLOOKUP(10,Sheet1:Sheet3!Z1:AA2,2,FALSE)"),
            (1, 3, "OFFSET(Sheet1:Sheet3!Z1,0,0)"),
            (1, 4, "SUM(Sheet1:Sheet3!Z1+1)"),
            (1, 5, "COUNTBLANK(Sheet1:Sheet3!Z1)"),
            (1, 6, "IFERROR(INDEX(Sheet1:Sheet3!Z1:Z2,1),42)"),
            (1, 7, "SUM(Missing:Sheet3!Z1)"),
            (1, 8, "SUM(Sheet3:Sheet1!Z1)"),
            (1, 9, "INDEX(Sheet1:Sheet1!Z1:Z2,1)"),
            (1, 10, "OFFSET(Sheet1:Sheet1!Z1,0,0)"),
            (1, 11, "SUM(Sheet1:Sheet1!Z1:Z2)"),
            (1, 12, "Sheet1:Sheet1!Z1"),
        ],
        &[],
    );
    let report = scan_formula_capabilities(&workbook);
    assert_eq!(report.formula_count(), 12);
    assert_eq!(report.supported_count(), 11);
    assert_capability_issue(
        &report,
        5,
        CalculationIssueCode::UnsupportedSheetRange,
        Some("Sheet1:Sheet3"),
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, error) in [
        (1, ExcelError::Value),
        (2, ExcelError::Value),
        (3, ExcelError::Reference),
        (4, ExcelError::Value),
        (7, ExcelError::Reference),
        (9, ExcelError::Value),
        (10, ExcelError::Reference),
        (12, ExcelError::Value),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error))),
            "column {column}",
        );
    }
    assert_issue(&calculation, 5, CalculationIssueCode::UnsupportedSheetRange);
    assert_number(&calculation, 6, 42.0, 0.0);
    assert_number(&calculation, 8, 111.0, 0.0);
    assert_number(&calculation, 11, 3.0, 0.0);
}

#[test]
fn every_three_d_aggregate_matches_explicit_sheet_arguments() {
    let workbook = three_sheet_workbook(
        &[
            (3, 1, "SUM(Sheet1:Sheet3!Z1:Z2)"),
            (3, 2, "SUM(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (4, 1, "AVERAGE(Sheet1:Sheet3!Z1:Z2)"),
            (4, 2, "AVERAGE(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (5, 1, "AVERAGEA(Sheet1:Sheet3!Z1:Z2)"),
            (5, 2, "AVERAGEA(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (6, 1, "COUNT(Sheet1:Sheet3!Z1:Z2)"),
            (6, 2, "COUNT(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (7, 1, "COUNTA(Sheet1:Sheet3!Z1:Z2)"),
            (7, 2, "COUNTA(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (8, 1, "MAX(Sheet1:Sheet3!Z1:Z2)"),
            (8, 2, "MAX(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (9, 1, "MAXA(Sheet1:Sheet3!Z1:Z2)"),
            (9, 2, "MAXA(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (10, 1, "MIN(Sheet1:Sheet3!Z1:Z2)"),
            (10, 2, "MIN(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (11, 1, "MINA(Sheet1:Sheet3!Z1:Z2)"),
            (11, 2, "MINA(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (12, 1, "PRODUCT(Sheet1:Sheet3!Z1:Z2)"),
            (12, 2, "PRODUCT(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (13, 1, "STDEV.P(Sheet1:Sheet3!Z1:Z2)"),
            (13, 2, "STDEV.P(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (14, 1, "STDEV.S(Sheet1:Sheet3!Z1:Z2)"),
            (14, 2, "STDEV.S(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (15, 1, "VAR.P(Sheet1:Sheet3!Z1:Z2)"),
            (15, 2, "VAR.P(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
            (16, 1, "VAR.S(Sheet1:Sheet3!Z1:Z2)"),
            (16, 2, "VAR.S(Sheet1!Z1:Z2,Sheet2!Z1:Z2,Sheet3!Z1:Z2)"),
        ],
        &[],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for row in 3..=16 {
        assert_eq!(
            calculation.cell(calculation_cell_id(row, 1)),
            calculation.cell(calculation_cell_id(row, 2)),
            "row {row}",
        );
    }
}

#[test]
fn let_reference_bindings_inherit_the_consuming_three_d_policy() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(LET(data,Sheet1:Sheet3!Z1,data))"),
            (1, 2, "LET(data,Sheet1:Sheet3!Z1,SUM(data))"),
            (1, 3, "LET(data,Sheet1:Sheet3!Z1,data)"),
        ],
        &[],
    );
    let report = scan_formula_capabilities(&workbook);
    for column in 1..=2 {
        assert!(
            matches!(
                report
                    .entries()
                    .iter()
                    .find(|entry| entry.cell() == cell_id(column))
                    .expect("LET 3-D capability entry")
                    .capability(),
                FormulaCapability::Supported
            ),
            "column {column} must inherit SUM's collecting policy",
        );
    }
    assert!(
        matches!(
            report
                .entries()
                .iter()
                .find(|entry| entry.cell() == cell_id(3))
                .expect("top-level LET capability entry")
                .capability(),
            FormulaCapability::Supported
        ),
        "a top-level LET must inherit the ordinary expression policy",
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 111.0, 0.0);
    assert_number(&calculation, 2, 111.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(3)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
}

#[test]
fn function_catalog_and_scanner_share_the_explicit_three_d_policy() {
    let catalog = supported_function_catalog();
    let mut first = Sheet::new(
        SheetId::new(1).expect("valid sheet ID"),
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    insert_number(&mut first, "Z1", 1.0);
    for (index, entry) in catalog.iter().enumerate() {
        let formula = format!("{}(Sheet1:Sheet3!Z1)", entry.name());
        let address =
            CellAddress::from_indices(index as u32 + 1, 1).expect("bounded catalog address");
        first
            .insert_cell(
                address,
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_xlsx(formula).expect("catalog name parses"),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("unique catalog formula");
    }
    let workbook = workbook_with_sheets_and_names(
        vec![
            first,
            numeric_sheet(2, "Sheet2", 10.0, SheetVisibility::Hidden),
            numeric_sheet(3, "Sheet3", 100.0, SheetVisibility::Visible),
        ],
        &[],
    );
    let report = scan_formula_capabilities(&workbook);
    let calculation = calculate_workbook(
        &workbook,
        CalculationOptions::default()
            .with_today_serial(FiniteNumber::new(45_000.0).expect("finite date"))
            .with_now_serial(FiniteNumber::new(45_000.5).expect("finite timestamp")),
    );
    assert_eq!(report.entries().len(), catalog.len());

    for (entry, capability) in catalog.iter().zip(report.entries()) {
        let calculates_sheet_span_argument = matches!(
            entry.canonical_name(),
            "SUM"
                | "AVERAGE"
                | "AVERAGEA"
                | "COUNT"
                | "COUNTA"
                | "MAX"
                | "MAXA"
                | "MIN"
                | "MINA"
                | "PRODUCT"
                | "STDEV.P"
                | "STDEV.S"
                | "VAR.P"
                | "VAR.S"
                | "INDEX"
                | "VLOOKUP"
                | "OFFSET"
                | "LET"
                | "LAMBDA"
                | "AREAS"
                | "ISREF"
                | "SHEETS"
        );
        let result = calculation.cell(capability.cell());
        match capability.capability() {
            FormulaCapability::Supported if calculates_sheet_span_argument => {
                assert!(
                    matches!(result, Some(CalculationCellResult::Value(_))),
                    "{} scanner/kernel policy mismatch: {result:?}",
                    entry.name(),
                );
            }
            FormulaCapability::Supported => {
                assert_eq!(
                    result,
                    Some(&CalculationCellResult::Value(CellValue::Error(
                        ExcelError::Value
                    ))),
                    "{} must reject an invalid unary call before inspecting its 3-D argument",
                    entry.name(),
                );
            }
            FormulaCapability::Unsupported(issues) => {
                assert!(
                    !calculates_sheet_span_argument,
                    "{} unexpectedly rejected its audited 3-D context: {issues:?}",
                    entry.name(),
                );
                assert!(
                    issues.iter().any(|issue| {
                        issue.code() == CalculationIssueCode::UnsupportedSheetRange
                    }),
                    "{} lost the sheet-range diagnosis: {issues:?}",
                    entry.name(),
                );
                assert!(
                    !matches!(result, Some(CalculationCellResult::Value(_))),
                    "{} scanner/kernel policy mismatch: {result:?}",
                    entry.name(),
                );
            }
        }
    }
}

#[test]
fn cross_sheet_range_operator_calculates_the_excel_value_error() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Sheet2!B1:Sheet3!B1)"),
            (1, 2, "IFERROR(SUM(Sheet2!B1:Sheet3!B1),99)"),
            (1, 3, "SUM(Sheet2!B1:Sheet2!B2)"),
            (1, 4, "SUM(Sheet2!B1:B2)"),
        ],
        &[],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_eq!(
        calculation.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
    assert_number(&calculation, 2, 99.0, 0.0);
    assert_number(&calculation, 3, 30.0, 0.0);
    assert_number(&calculation, 4, 30.0, 0.0);
}

#[test]
fn quoted_external_workbook_references_are_unsupported_and_cannot_be_hidden_by_iferror() {
    // Both quoted and unquoted external workbook spellings reach one typed node. Resolving either
    // as an ordinary missing sheet would yield a catchable `#REF!` and misreport the formula as
    // supported, so the capability remains an engine-level unsupported expression.
    let workbook = workbook_with_formulas(&[
        (1, 1, "'[1]Sheet1'!A1"),
        (1, 2, "IFERROR('[1]Sheet1'!A1,0)"),
        (1, 3, "SUM('[Book.xlsx]Sheet1'!A1:B2)"),
        (1, 4, "'Sheet1'!B9"),
        (1, 5, "'C:\\Reports\\[Q1.xlsx]Sheet1'!A1"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    for column in 1..=3 {
        assert_capability_issue_code(&report, column, CalculationIssueCode::UnsupportedExpression);
    }
    // Excel writes the saved path into the sheet token, so the prefix carries a drive colon. That
    // colon marks no end sheet: the reader must see the whole path and one diagnosis, not a
    // truncated one plus a 3-D sheet range the formula does not contain.
    assert_capability_issue(
        &report,
        5,
        CalculationIssueCode::UnsupportedExpression,
        Some("C:\\Reports\\[Q1.xlsx]Sheet1"),
    );
    assert_capability_issue_count(&report, 5, 1);
    // Excel forbids brackets in sheet names, so an ordinary quoted sheet name must stay supported.
    assert!(matches!(
        report
            .entries()
            .iter()
            .find(|entry| entry.cell() == cell_id(4))
            .expect("quoted local sheet capability entry")
            .capability(),
        FormulaCapability::Supported
    ));

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in [1, 2, 3, 5] {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::UnsupportedExpression,
        );
    }
}

#[test]
fn external_workbook_references_built_by_indirect_stay_catchable() {
    // The capability scan cannot read a reference that only exists once INDIRECT's text argument
    // has been evaluated, so it reports these cells as supported. Raising an engine issue during
    // calculation would contradict that report and leave the cell — and everything downstream of
    // it — unavailable with no way to recover. Excel answers `#REF!` for text it cannot resolve to
    // a reference, and `IFERROR` may hide that because nothing ever promised it would resolve.
    let workbook = workbook_with_formulas(&[
        (1, 1, "IFERROR(INDIRECT(A2),42)"),
        (2, 1, "\"'[Book1.xlsx]Sheet1'!A1\""),
        (1, 2, "INDIRECT(A2)"),
        (1, 3, "IFERROR(INDIRECT(\"'NoSuchSheet'!A1\"),42)"),
        (1, 4, "IFERROR(INDIRECT(\"'Sheet1:Sheet3'!A1\"),42)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 42.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
    // A missing sheet already behaved this way; an unsupported reference form discovered inside
    // the text must not be the one case that becomes uncatchable.
    assert_number(&calculation, 3, 42.0, 0.0);
    assert_number(&calculation, 4, 42.0, 0.0);
}

#[test]
fn parse_error_details_use_stable_codes_and_utf8_byte_spans() {
    // Lex and parse failures share one location contract so callers never have to interpret the
    // same integer as either a character offset or a token index.
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Table1?Amount)"),
        (1, 2, "\"unterminated"),
        (1, 3, "1+"),
        (1, 4, "SUM(1))"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    assert_capability_issue(
        &report,
        1,
        CalculationIssueCode::ParseError,
        Some("bytes 10..11 [formula.lex.unexpected_character]: unexpected character in formula"),
    );
    assert_capability_issue(
        &report,
        2,
        CalculationIssueCode::ParseError,
        Some("bytes 0..13 [formula.lex.unterminated_string]: unterminated string literal"),
    );
    assert_capability_issue(
        &report,
        3,
        CalculationIssueCode::ParseError,
        Some("bytes 2..2 [formula.parse.unexpected_end]: unexpected end of formula"),
    );
    assert_capability_issue(
        &report,
        4,
        CalculationIssueCode::ParseError,
        Some("bytes 6..7 [formula.parse.unexpected_token]: unexpected token"),
    );
}

#[test]
fn typed_reference_grammar_is_classified_without_parse_failures() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Table1[Amount])"),
        (1, 2, "A1#"),
        (1, 3, "[Book.xlsx]Sheet1!A1"),
        (1, 4, "Sheet1!LocalRate"),
        (1, 5, "A1,B1"),
        (1, 6, "A1 B1"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    for column in [1, 2, 5, 6] {
        assert!(matches!(
            report
                .entries()
                .iter()
                .find(|entry| entry.cell() == cell_id(column))
                .expect("supported typed-reference entry")
                .capability(),
            FormulaCapability::Supported
        ));
    }
    let external = report
        .entries()
        .iter()
        .find(|entry| entry.cell() == cell_id(3))
        .expect("external-reference formula entry");
    let FormulaCapability::Unsupported(issues) = external.capability() else {
        panic!("external-reference formula should be unsupported");
    };
    assert!(
        issues
            .iter()
            .any(|issue| issue.code() == CalculationIssueCode::UnsupportedExpression),
        "external reference should be a typed unsupported expression"
    );
    assert!(
        issues
            .iter()
            .all(|issue| issue.code() != CalculationIssueCode::ParseError),
        "external reference must not regress to a parse failure"
    );
    assert_capability_issue(
        &report,
        4,
        CalculationIssueCode::UnsupportedName,
        Some("LocalRate"),
    );
}

#[test]
fn calculated_zeros_are_positive_like_excel() {
    // Float kernels reach `-0.0` by several routes: `Iterator::sum` folds from it, `f64::min` and
    // `f64::max` may return either operand when both compare equal, and `Iterator::product`
    // inherits the sign of an odd number of negative factors. Excel's number model has no negative
    // zero, so the calculation boundary normalizes rather than each kernel remembering to.
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Z1:Z5)"),
        (1, 2, "SUMSQ(Z1:Z5)"),
        (1, 3, "NPV(0.1,Z1:Z5)"),
        (1, 4, "SUM(-0)"),
        (1, 5, "MIN(-0)"),
        (1, 6, "MAX(-0)"),
        (1, 7, "MIN(0,-0)"),
        (1, 8, "PRODUCT(-1,0)"),
        (1, 9, "AVERAGEA(-0)"),
        (1, 10, "MINA(-0)"),
        (1, 11, "MAXA(-0)"),
        (1, 12, "SUM(1,2,3)"),
        (1, 13, "AVERAGE(2,4)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=11 {
        assert_positive_zero(&calculation, column);
    }
    assert_number(&calculation, 12, 6.0, 0.0);
    assert_number(&calculation, 13, 3.0, 0.0);
}

#[test]
fn dynamic_arrays_calculate_and_data_tables_remain_explicitly_unsupported() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let dynamic_address = CellAddress::from_a1("A1").expect("valid dynamic formula address");
    let dynamic_range = CellRange::new(
        dynamic_address,
        CellAddress::from_a1("A2").expect("valid dynamic range end"),
    )
    .expect("ordered dynamic range");
    sheet
        .insert_cell(
            dynamic_address,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx("SEQUENCE(2)").expect("valid dynamic formula"),
                SavedResult::Missing,
                FormulaMetadata::DynamicArray {
                    range: Some(dynamic_range),
                    always_calculate: true,
                },
            )),
        )
        .expect("unique dynamic formula address");

    let table_address = CellAddress::from_a1("B1").expect("valid data table address");
    let table_range = CellRange::new(
        table_address,
        CellAddress::from_a1("B2").expect("valid data table range end"),
    )
    .expect("ordered data table range");
    sheet
        .insert_cell(
            table_address,
            CellContent::Formula(FormulaCell::metadata_only(
                FormulaDialect::ExcelA1,
                SavedResult::Missing,
                FormulaMetadata::DataTable {
                    range: table_range,
                    input_cell_1: Some(
                        CellAddress::from_a1("C1").expect("valid first input address"),
                    ),
                    input_cell_2: None,
                    two_dimensional: false,
                    row_oriented: false,
                    input_cell_1_deleted: false,
                    input_cell_2_deleted: false,
                },
            )),
        )
        .expect("unique data table address");
    let dependent_address = CellAddress::from_a1("C1").expect("valid dependent address");
    sheet
        .insert_cell(
            dependent_address,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx("SUM(A1:A2)").expect("valid dependent formula"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        )
        .expect("unique dependent formula");

    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("calculation-test", "1").expect("valid provider identity"),
            None,
        ),
    )
    .expect("valid metadata workbook");
    let report = scan_formula_capabilities(&workbook);
    assert_capability_issue_code(&report, 2, CalculationIssueCode::UnsupportedExpression);
    assert_capability_issue_code(&report, 2, CalculationIssueCode::MissingFormulaText);

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 1.0, 0.0);
    assert_issue(&calculation, 2, CalculationIssueCode::MissingFormulaText);
    assert_number(&calculation, 3, 3.0, 0.0);
    for (address, expected) in [("A1", 1.0), ("A2", 2.0)] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("valid spill address"),
        );
        let materialized = calculation
            .materialized_cell(id)
            .expect("dynamic cell must be materialized");
        assert_eq!(
            materialized.origin(),
            MaterializedResultOrigin::DynamicSpill {
                anchor: CalculationCellId::new(sheet_id, dynamic_address),
                range: dynamic_range,
            }
        );
        assert_eq!(
            materialized.result(),
            &CalculationCellResult::Value(CellValue::number(expected).expect("finite spill value"))
        );
    }
}

#[test]
fn function_usage_and_catalog_report_normalized_supported_demand() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(1,SUM(2,3))"),
        (1, 2, "_xlfn.UNIQUE({1;1;2})"),
        (1, 3, "NOPE(1)+SUM(4,5)"),
        (1, 4, "1+"),
    ]);

    let report = scan_function_usage(&workbook);
    assert_eq!(report.formula_count(), 4);
    assert_eq!(report.parsed_formula_count(), 3);
    assert_eq!(report.unparsed_formula_count(), 1);
    assert!(!report.is_fully_supported());
    let sum = report
        .entries()
        .iter()
        .find(|entry| entry.name() == "SUM")
        .expect("SUM usage");
    assert_eq!(sum.call_count(), 3);
    assert_eq!(sum.formula_count(), 2);
    assert_eq!(sum.sample_cells(), &[cell_id(1), cell_id(3)]);
    assert_eq!(sum.support(), cellrune::FunctionSupport::Supported);
    let unique = report
        .entries()
        .iter()
        .find(|entry| entry.name() == "UNIQUE")
        .expect("UNIQUE usage");
    assert_eq!(unique.support(), cellrune::FunctionSupport::Supported);
    let unsupported = report
        .entries()
        .iter()
        .find(|entry| entry.name() == "NOPE")
        .expect("unsupported usage");
    assert_eq!(
        unsupported.support(),
        cellrune::FunctionSupport::Unsupported
    );

    let scoped_report = scan_function_usage(&workbook_with_formulas(&[
        (1, 1, "LET(total,SUM(1,2),total)"),
        (1, 2, "MAP({1},LAMBDA(item,item+1))"),
    ]));
    assert!(
        scoped_report
            .entries()
            .iter()
            .any(|entry| entry.name() == "LET")
    );
    assert!(
        scoped_report
            .entries()
            .iter()
            .any(|entry| entry.name() == "MAP")
    );
    assert!(
        scoped_report
            .entries()
            .iter()
            .any(|entry| entry.name() == "LAMBDA")
    );

    let sample_workbook = workbook_with_formulas(
        &(1..=10)
            .map(|column| (1, column, "SUM(1,2)"))
            .collect::<Vec<_>>(),
    );
    let sample_report = scan_function_usage(&sample_workbook);
    let sampled_sum = sample_report
        .entries()
        .iter()
        .find(|entry| entry.name() == "SUM")
        .expect("sampled SUM usage");
    assert_eq!(sampled_sum.call_count(), 10);
    assert_eq!(sampled_sum.formula_count(), 10);
    assert_eq!(sampled_sum.sample_cells().len(), 8);
    assert_eq!(sampled_sum.sample_cells()[0], cell_id(1));
    assert_eq!(sampled_sum.sample_cells()[7], cell_id(8));

    let catalog = supported_function_catalog();
    assert_eq!(
        catalog.iter().filter(|entry| entry.is_official()).count(),
        367
    );
    let let_entry = catalog
        .iter()
        .find(|entry| entry.name() == "LET")
        .expect("LET");
    assert!(let_entry.returns_array());
    let percentile = catalog
        .iter()
        .find(|entry| entry.name() == "PERCENTILE")
        .expect("legacy alias");
    assert!(percentile.is_alias());
    assert_eq!(percentile.canonical_name(), "PERCENTILE.INC");
    assert!(
        catalog
            .iter()
            .find(|entry| entry.name() == "FILTER")
            .expect("FILTER")
            .returns_array()
    );
    for name in ["ABS", "IF", "COUNTIF", "COUNTIFS", "INDEX", "ISNUMBER"] {
        assert!(
            catalog
                .iter()
                .find(|entry| entry.name() == name)
                .unwrap_or_else(|| panic!("{name} catalog entry"))
                .returns_array(),
            "{name} must advertise its multi-cell array kernel",
        );
    }
}

#[test]
fn composed_excel_storage_prefixes_calculate_and_report_canonical_usage() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let address = CellAddress::from_a1("A1").expect("valid formula address");
    let formula = FormulaCell::new(
        FormulaDialect::ExcelA1,
        FormulaText::from_xlsx("_xlfn._xlws.FILTER({1;2},{1;0})")
            .expect("valid storage-prefixed formula"),
        SavedResult::Missing,
        FormulaMetadata::DynamicArray {
            range: Some(CellRange::new(address, address).expect("single-cell range")),
            always_calculate: true,
        },
    );
    sheet
        .insert_cell(address, CellContent::Formula(formula))
        .expect("unique formula address");
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("storage-prefix-test", "1").expect("provider"),
            None,
        ),
    )
    .expect("storage-prefixed workbook");

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let usage = scan_function_usage(&workbook);
    assert_eq!(usage.entries().len(), 1);
    assert_eq!(usage.entries()[0].name(), "FILTER");
    assert_eq!(
        usage.entries()[0].support(),
        cellrune::FunctionSupport::Supported
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 1.0, 0.0);
}

#[test]
fn high_value_modern_array_functions_preserve_shape_and_excel_boundaries() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let cases = [
        ("A1", "B2", "CHOOSECOLS({1,2,3;4,5,6},3,1)"),
        ("D1", "E2", "CHOOSEROWS({1,2;3,4;5,6},-1,1)"),
        ("G1", "H2", "TAKE({1,2,3;4,5,6;7,8,9},-2,2)"),
        ("J1", "K2", "DROP({1,2,3;4,5,6;7,8,9},1,-1)"),
        ("M1", "N2", "HSTACK({1;2},{3})"),
        ("P1", "P4", "VSTACK({1;2},{3;4})"),
        ("R1", "S2", "SORT({2,3;4,1},2,1)"),
        ("U1", "U2", "UNIQUE({3;1;3;1})"),
        ("W1", "X2", "FILTER({1,10;2,20;3,30},{1;0;1})"),
    ];
    for (start, end, text) in cases {
        let start = CellAddress::from_a1(start).expect("valid array anchor");
        let end = CellAddress::from_a1(end).expect("valid array end");
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx(text).expect("valid array formula"),
            SavedResult::Missing,
            FormulaMetadata::Array {
                range: CellRange::new(start, end).expect("ordered array range"),
                always_calculate: false,
            },
        );
        sheet
            .insert_cell(start, CellContent::Formula(formula))
            .expect("unique array anchor");
    }
    let workbook = WorkbookSnapshot::new(
        vec![sheet],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("modern-array-test", "1").expect("provider"),
            None,
        ),
    )
    .expect("modern array workbook");
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (address, expected) in [
        ("A1", 3.0),
        ("B1", 1.0),
        ("A2", 6.0),
        ("B2", 4.0),
        ("D1", 5.0),
        ("E1", 6.0),
        ("D2", 1.0),
        ("E2", 2.0),
        ("G1", 4.0),
        ("H1", 5.0),
        ("G2", 7.0),
        ("H2", 8.0),
        ("J1", 4.0),
        ("K1", 5.0),
        ("J2", 7.0),
        ("K2", 8.0),
        ("M1", 1.0),
        ("N1", 3.0),
        ("M2", 2.0),
        ("P1", 1.0),
        ("P2", 2.0),
        ("P3", 3.0),
        ("P4", 4.0),
        ("R1", 4.0),
        ("S1", 1.0),
        ("R2", 2.0),
        ("S2", 3.0),
        ("U1", 3.0),
        ("U2", 1.0),
        ("W1", 1.0),
        ("X1", 10.0),
        ("W2", 3.0),
        ("X2", 30.0),
    ] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("valid result address"),
        );
        assert_eq!(
            calculation.materialized_cell(id).map(|cell| cell.result()),
            Some(&CalculationCellResult::Value(
                CellValue::number(expected).expect("finite expected value")
            )),
            "unexpected modern array result at {address}",
        );
    }
    let padded = CalculationCellId::new(
        sheet_id,
        CellAddress::from_a1("N2").expect("valid padded address"),
    );
    assert_eq!(
        calculation
            .materialized_cell(padded)
            .map(|cell| cell.result()),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
}

#[test]
fn drop_omitted_axes_and_elementwise_abs_preserve_excel_array_semantics() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "=DROP({1,2;3,4},1)"),
        ("D1", "=DROP({1,2;3,4},-1)"),
        ("G1", "=DROP({1,2;3,4},,1)"),
        ("J1", "=DROP({1,2;3,4},1,1)"),
        ("M1", "=ABS({-1;-2})"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 3.0),
        ("B1", 4.0),
        ("D1", 1.0),
        ("E1", 2.0),
        ("G1", 2.0),
        ("G2", 4.0),
        ("J1", 4.0),
        ("M1", 1.0),
        ("M2", 2.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
}

#[test]
fn undeclared_dynamic_spill_followers_depend_on_their_anchor() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("A1").expect("dependent formula address"),
            FormulaText::from_user_input("=C2+1").expect("valid dependent formula"),
        )
        .expect("dependent formula mutation");
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("C1").expect("dynamic anchor"),
            FormulaText::from_user_input("=SEQUENCE(2)").expect("valid dynamic formula"),
            None,
        )
        .expect("dynamic formula mutation");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_number_at(&calculation, 1, 1, 3.0, 0.0);
    assert_materialized_number(&calculation, sheet_id, "C2", 2.0);
}

#[test]
fn spill_references_resolve_anchor_shapes_and_reject_non_anchors() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A1").expect("ordinary cell"),
            CellValue::number(7.0).expect("finite value"),
        )
        .expect("ordinary value");
    for (address, formula) in [
        ("B1", "SEQUENCE(2,2)"),
        ("B6", "FILTER({1;2;3},{TRUE;FALSE;TRUE})"),
        ("D6", "UNIQUE({1,1,2},TRUE)"),
        ("H1", "SEQUENCE(2)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("dynamic anchor"),
                FormulaText::from_xlsx(formula).expect("dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("H2").expect("spill obstruction"),
            CellValue::Text("occupied".to_owned()),
        )
        .expect("spill obstruction");
    for (address, formula) in [
        ("F1", "SUM(B1#)"),
        ("F2", "ROWS(B1#)"),
        ("F3", "C1#"),
        ("F4", "A1#"),
        ("F5", "SUM(B6#)"),
        ("F6", "SUM(D6#)"),
        ("F7", "AREAS(B1#)"),
        ("F8", "ISREF(B1#)"),
        ("J1", "H1#"),
        ("J2", "ISREF(H1#)"),
    ] {
        draft
            .set_cell_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("spill consumer"),
                FormulaText::from_xlsx(formula).expect("spill reference formula"),
            )
            .expect("spill consumer mutation");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("F1", 10.0),
        ("F2", 2.0),
        ("F5", 4.0),
        ("F6", 3.0),
        ("F7", 1.0),
    ] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("result address"),
        );
        assert_eq!(
            calculation.cell(id),
            Some(&CalculationCellResult::Value(
                CellValue::number(expected).expect("finite expected value")
            )),
            "{address}",
        );
    }
    for address in ["F3", "F4"] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("reference error address"),
        );
        assert_eq!(
            calculation.cell(id),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Reference
            ))),
            "{address}",
        );
    }
    let blocked = CalculationCellId::new(
        sheet_id,
        CellAddress::from_a1("J1").expect("blocked spill consumer"),
    );
    assert_eq!(
        calculation.cell(blocked),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Spill
        )))
    );
    for (address, expected) in [("F8", true), ("J2", false)] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("ISREF spill result"),
        );
        assert_eq!(
            calculation.cell(id),
            Some(&CalculationCellResult::Value(CellValue::Logical(expected))),
            "{address}",
        );
    }
}

#[test]
fn non_finite_numeric_literals_are_excel_number_errors() {
    let workbook =
        workbook_with_formulas(&[(1, 1, "1E309"), (1, 2, "1E309=1E309"), (1, 3, "-1E309")]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=3 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "non-finite literal in column {column} was not rejected",
        );
    }
}

#[test]
fn modern_array_functions_reject_empty_and_out_of_range_requests_explicitly() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "FILTER({1;2},{0;0})"),
        (1, 2, "TAKE({1;2},0)"),
        (1, 3, "CHOOSECOLS({1,2},0)"),
        (1, 4, "DROP({1;2},2)"),
        (1, 5, "SORT({1;2},1,2)"),
        (1, 6, "FILTER({1;2},{0;0},\"empty\")"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in [1, 2, 4] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Calculation
            )))
        );
    }
    for column in [3, 5] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
    assert_eq!(
        calculation.cell(cell_id(6)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "empty".to_owned()
        )))
    );
}

#[test]
fn modern_array_argument_boundaries_are_never_silently_accepted() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "CHOOSECOLS()"),
        (1, 2, "CHOOSECOLS({1,2})"),
        (1, 3, "TAKE()"),
        (1, 4, "TAKE({1;2})"),
        (1, 5, "TAKE({1;2},1,1,1)"),
        (1, 6, "FILTER()"),
        (1, 7, "FILTER({1;2})"),
        (1, 8, "FILTER({1;2},{1;0},\"x\",\"extra\")"),
        (1, 9, "SORT()"),
        (1, 10, "SORT({1;2},1,1,FALSE,0)"),
        (1, 11, "SORT({1,2},0)"),
        (1, 12, "SORT({1,2},3)"),
        (1, 13, "UNIQUE()"),
        (1, 14, "UNIQUE({1;2},FALSE,FALSE,FALSE)"),
        (1, 15, "TAKE({1;2},3)"),
        (1, 16, "DROP({1;2},-3)"),
        (1, 17, "FILTER({1,2;3,4},{1,0;0,1})"),
        (1, 18, "CHOOSECOLS({1,2},2)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=14 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "invalid modern-array call in column {column} was accepted",
        );
    }
    for column in 15..=16 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "out-of-range modern-array count in column {column} was accepted",
        );
    }
    assert_eq!(
        calculation.cell(cell_id(17)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
    assert_number(&calculation, 18, 2.0, 0.0);
}

#[test]
fn modern_dynamic_arrays_cover_column_axes_sort_types_and_unique_modes() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "=CHOOSECOLS({1;2},1,1,1)"),
        ("E1", "=CHOOSEROWS({1,2,3},1,1)"),
        ("I1", "=TAKE({1,2,3,4,5;6,7,8,9,10},-1,-2)"),
        ("L1", "=DROP({1,2,3,4,5;6,7,8,9,10},-1,-2)"),
        ("P1", "=FILTER({1,2,3;4,5,6},{0,1,1})"),
        ("T1", "=HSTACK({1;2;3},{4})"),
        ("W1", "=SORT({3,1,2;30,10,20},2,1,TRUE)"),
        ("A5", "=UNIQUE({1,1,2;10,10,20},TRUE,TRUE)"),
        ("C5", "=UNIQUE({\"A\";\"a\";\"B\";\"C\"},FALSE,FALSE)"),
        ("E5", "=UNIQUE({1;1;2;3},FALSE,TRUE)"),
        ("G5", "=SORT({TRUE;\"z\";2;#N/A})"),
        ("I5", "=VSTACK({1,2},{3})"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 1.0),
        ("B1", 1.0),
        ("C1", 1.0),
        ("A2", 2.0),
        ("B2", 2.0),
        ("C2", 2.0),
        ("E1", 1.0),
        ("F1", 2.0),
        ("G1", 3.0),
        ("E2", 1.0),
        ("F2", 2.0),
        ("G2", 3.0),
        ("I1", 9.0),
        ("J1", 10.0),
        ("L1", 1.0),
        ("M1", 2.0),
        ("N1", 3.0),
        ("P1", 2.0),
        ("Q1", 3.0),
        ("P2", 5.0),
        ("Q2", 6.0),
        ("T1", 1.0),
        ("U1", 4.0),
        ("T2", 2.0),
        ("T3", 3.0),
        ("W1", 1.0),
        ("X1", 2.0),
        ("Y1", 3.0),
        ("W2", 10.0),
        ("X2", 20.0),
        ("Y2", 30.0),
        ("A5", 2.0),
        ("A6", 20.0),
        ("E5", 2.0),
        ("E6", 3.0),
        ("G5", 2.0),
        ("I5", 1.0),
        ("J5", 2.0),
        ("I6", 3.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for address in ["U2", "U3", "J6"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::NotAvailable
            )))
        );
    }
    for (address, expected) in [("C5", "A"), ("C6", "B"), ("C7", "C"), ("G6", "z")] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            )))
        );
    }
    assert_eq!(
        materialized_result(&calculation, sheet_id, "G7"),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );
    assert_eq!(
        materialized_result(&calculation, sheet_id, "G8"),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
}

#[test]
fn array_reshape_and_order_functions_preserve_excel_shapes_and_defaults() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, value) in [("B10", 1.0), ("C10", 2.0), ("B11", 3.0), ("C11", 4.0)] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid trim source address"),
                CellValue::number(value).expect("finite trim source value"),
            )
            .expect("trim source mutation");
    }
    for (address, formula) in [
        ("A1", "=EXPAND({1,2;3,4},3,4,\"pad\")"),
        (
            "F1",
            "=SORTBY({\"r1\",10;\"r2\",20;\"r3\",30},{\"b\";\"a\";\"a\"},1,{2;1;2},-1)",
        ),
        ("K1", "=TOCOL({1,2,3;4,5,6},0,TRUE)"),
        ("M1", "=TOROW({1,2,3;4,5,6},0,TRUE)"),
        ("A14", "=TRIMRANGE(A9:D12)"),
        ("F10", "=WRAPCOLS({1,2,3,4,5},2)"),
        ("J10", "=WRAPROWS({1;2;3;4;5},2,0)"),
        ("M10", "=_xlfn.TOCOL({7,8})"),
        ("T1", "=SORTBY({10;20;30},{2;1;1})"),
        ("V1", "=TOCOL({1,#N/A;2,3},2)"),
        ("X1", "=TOROW({1,2;3,4})"),
        ("D14", "=WRAPCOLS({1,2},5)"),
        ("F14", "=WRAPROWS({1;2},5)"),
        ("I14", "=EXPAND({1,2;3,4},,3)"),
        ("A18", "=SORTBY({10,20,30},{2,1,3})"),
        ("E18", "=SORTBY({3;2;1},{3;2;1},)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 1.0),
        ("B1", 2.0),
        ("A2", 3.0),
        ("B2", 4.0),
        ("G1", 30.0),
        ("G2", 20.0),
        ("G3", 10.0),
        ("K1", 1.0),
        ("K2", 4.0),
        ("K3", 2.0),
        ("K4", 5.0),
        ("K5", 3.0),
        ("K6", 6.0),
        ("M1", 1.0),
        ("N1", 4.0),
        ("O1", 2.0),
        ("P1", 5.0),
        ("Q1", 3.0),
        ("R1", 6.0),
        ("A14", 1.0),
        ("B14", 2.0),
        ("A15", 3.0),
        ("B15", 4.0),
        ("F10", 1.0),
        ("G10", 3.0),
        ("H10", 5.0),
        ("F11", 2.0),
        ("G11", 4.0),
        ("J10", 1.0),
        ("K10", 2.0),
        ("J11", 3.0),
        ("K11", 4.0),
        ("J12", 5.0),
        ("K12", 0.0),
        ("M10", 7.0),
        ("M11", 8.0),
        ("T1", 20.0),
        ("T2", 30.0),
        ("T3", 10.0),
        ("V1", 1.0),
        ("V2", 2.0),
        ("V3", 3.0),
        ("X1", 1.0),
        ("Y1", 2.0),
        ("Z1", 3.0),
        ("AA1", 4.0),
        ("D14", 1.0),
        ("D15", 2.0),
        ("F14", 1.0),
        ("G14", 2.0),
        ("I14", 1.0),
        ("J14", 2.0),
        ("I15", 3.0),
        ("J15", 4.0),
        ("A18", 20.0),
        ("B18", 10.0),
        ("C18", 30.0),
        ("E18", 1.0),
        ("E19", 2.0),
        ("E20", 3.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
    for (address, expected) in [("F1", "r3"), ("F2", "r2"), ("F3", "r1")] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "unexpected SORTBY value at {address}",
        );
    }
    for address in ["C1", "D1", "C2", "D2", "A3", "B3", "C3", "D3"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                "pad".to_owned()
            ))),
            "unexpected EXPAND padding at {address}",
        );
    }
    assert_eq!(
        materialized_result(&calculation, sheet_id, "H11"),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
    for address in ["K14", "K15"] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::NotAvailable
            )))
        );
    }
}

#[test]
fn array_reshape_and_order_functions_reject_invalid_domains() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "EXPAND({1;2},1)"),
        (1, 2, "EXPAND({1},0)"),
        (1, 3, "SORTBY({1;2},{1;2},2)"),
        (1, 4, "SORTBY({1;2},{1,2})"),
        (1, 5, "TOCOL({1},4)"),
        (1, 6, "TOROW({1},-1)"),
        (1, 7, "TRIMRANGE({1},4)"),
        (1, 8, "WRAPCOLS({1,2;3,4},2)"),
        (1, 9, "WRAPROWS({1;2},0)"),
        (1, 10, "TOCOL({#N/A},2)"),
        (1, 11, "TRIMRANGE(A100:B101)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=8 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "invalid array reshape domain in column {column} was accepted",
        );
    }
    assert_eq!(
        calculation.cell(cell_id(9)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(10)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Calculation
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(11)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
}

#[test]
fn array_reshape_and_order_functions_block_on_upstream_engine_issues() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, value) in [("A1", 7.0), ("A3", 8.0)] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid source address"),
                CellValue::number(value).expect("finite source value"),
            )
            .expect("source value mutation");
    }
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("A2").expect("valid unsupported formula address"),
            FormulaText::from_user_input("=MYSTERY()").expect("valid unsupported formula"),
        )
        .expect("unsupported formula mutation");
    for (address, formula) in [
        ("C1", "=EXPAND(A1:A2,3)"),
        ("E1", "=TRIMRANGE(A1:A3)"),
        ("G1", "=SORTBY(A1:A2,{2;1})"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid dynamic formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let unsupported_id = CalculationCellId::new(
        sheet_id,
        CellAddress::from_a1("A2").expect("valid unsupported formula address"),
    );
    let Some(CalculationCellResult::Unavailable(unsupported)) = calculation.cell(unsupported_id)
    else {
        panic!("unsupported source formula unexpectedly calculated");
    };
    assert_eq!(
        unsupported.code(),
        CalculationIssueCode::UnsupportedFunction
    );

    for address in ["C1", "E1", "G1"] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("valid dynamic anchor"),
        );
        let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(id) else {
            panic!("{address} leaked an upstream engine issue into an array value");
        };
        assert_eq!(issue.code(), CalculationIssueCode::BlockedByUpstream);
    }
}

#[test]
fn reference_introspection_and_xmatch_follow_metadata_and_lookup_contracts() {
    let mut first = formula_sheet(
        1,
        "Sheet1",
        &[
            (1, 1, "1+2"),
            (1, 3, "MYSTERY()"),
            (3, 1, "FORMULATEXT(A1)"),
            (3, 2, "ISFORMULA(A1)"),
            (3, 3, "ISFORMULA(B1)"),
            (3, 4, "FORMULATEXT(B1)"),
            (3, 5, "FORMULATEXT(C1)"),
            (3, 6, "ISFORMULA(C1)"),
            (3, 7, "SHEET()"),
            (3, 8, "SHEET(Sheet3!A1)"),
            (3, 9, "SHEET(\"Sheet2\")"),
            (3, 10, "SHEETS()"),
            (3, 11, "SHEETS(Sheet1:Sheet3!A1)"),
            (3, 12, "_xlfn.FORMULATEXT(A1)"),
            (3, 13, "XMATCH(\"beta\",{\"Alpha\",\"beta\",\"beta\"},0,1)"),
            (3, 14, "XMATCH(\"beta\",{\"Alpha\",\"beta\",\"beta\"},0,-1)"),
            (3, 15, "XMATCH(25,{10,30,20},-1,1)"),
            (3, 16, "XMATCH(25,{10,30,20},1,1)"),
            (3, 17, "XMATCH(\"b?t*\",{\"alpha\",\"BETA\",\"beta2\"},2,1)"),
            (3, 18, "XMATCH(30,{10,20,30,40},0,2)"),
            (3, 19, "XMATCH(30,{40,30,20,10},0,-2)"),
            (3, 20, "XMATCH(2,{1;2;3})"),
            (3, 21, "XMATCH(2,{1,2;3,4})"),
            (3, 22, "XMATCH(2,{1,2,3},3)"),
            (3, 23, "ISFORMULA(W3)"),
            (3, 24, "FORMULATEXT(X3)"),
            (3, 25, "ISFORMULA(7)"),
            (3, 26, "FORMULATEXT(7)"),
            (3, 27, "SHEET(7)"),
            (3, 28, "SHEETS(7)"),
            (3, 29, "SHEET(#REF!)"),
            (3, 30, "FORMULATEXT(INDEX(AD3:AD3,1))"),
            (3, 31, "ISFORMULA(OFFSET(AE3,0,0))"),
            (3, 32, "FORMULATEXT(LET(ref_value,AF3,ref_value))"),
            (3, 33, "XMATCH(\"~a\",{\"a\",\"~a\"},2)"),
            (3, 34, "XMATCH(\"a~\",{\"a\",\"a~\"},2)"),
            (3, 35, "XMATCH(\"a~~\",{\"a\",\"a~\"},2)"),
        ],
    );
    insert_number(&mut first, "B1", 7.0);
    let workbook = workbook_with_sheets_and_names(
        vec![
            first,
            numeric_sheet(2, "Sheet2", 10.0, SheetVisibility::Hidden),
            numeric_sheet(3, "Sheet3", 100.0, SheetVisibility::Visible),
        ],
        &[],
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&calculation, 3, CalculationIssueCode::UnsupportedFunction);
    for (column, expected) in [
        (7, 1.0),
        (8, 3.0),
        (9, 2.0),
        (10, 3.0),
        (11, 3.0),
        (13, 2.0),
        (14, 3.0),
        (15, 3.0),
        (16, 2.0),
        (17, 2.0),
        (18, 3.0),
        (19, 2.0),
        (20, 2.0),
        (33, 2.0),
        (34, 2.0),
        (35, 2.0),
    ] {
        assert_number_at(&calculation, 3, column, expected, 0.0);
    }
    for (column, expected) in [(2, true), (3, false), (6, true), (23, true), (31, true)] {
        assert_eq!(
            calculation.cell(calculation_cell_id(3, column)),
            Some(&CalculationCellResult::Value(CellValue::Logical(expected)))
        );
    }
    for (column, expected) in [
        (1, "=1+2"),
        (5, "=MYSTERY()"),
        (12, "=1+2"),
        (24, "=FORMULATEXT(X3)"),
        (30, "=FORMULATEXT(INDEX(AD3:AD3,1))"),
        (32, "=FORMULATEXT(LET(ref_value,AF3,ref_value))"),
    ] {
        assert_eq!(
            calculation.cell(calculation_cell_id(3, column)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            )))
        );
    }
    assert_eq!(
        calculation.cell(calculation_cell_id(3, 4)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
    for column in [21, 22, 25, 26] {
        assert_eq!(
            calculation.cell(calculation_cell_id(3, column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
    assert_eq!(
        calculation.cell(calculation_cell_id(3, 27)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
    assert_eq!(
        calculation.cell(calculation_cell_id(3, 28)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
    assert_eq!(
        calculation.cell(calculation_cell_id(3, 29)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
}

#[test]
fn reference_introspection_and_xmatch_preserve_engine_limits_and_upstream_issues() {
    let oversized_formula = format!("\"{}\"", "a".repeat(8_190));
    let oversized = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, oversized_formula.as_str()),
            (1, 2, "FORMULATEXT(A1)"),
        ]),
        CalculationOptions::default(),
    );
    assert_eq!(
        oversized.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );

    let text_limits = CalculationLimits::default()
        .with_max_text_bytes(8)
        .expect("nonzero formula-text limit");
    let formula_text = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "123456789+1"),
            (1, 2, "IFERROR(FORMULATEXT(A1),\"hidden\")"),
        ]),
        CalculationOptions::default().with_limits(text_limits),
    );
    assert_issue(
        &formula_text,
        2,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let work_limits = CalculationLimits::default()
        .with_max_function_iterations(5)
        .expect("nonzero XMATCH iteration limit");
    let xmatch = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "IFERROR(XMATCH(9,{1,2,3,4,5,6,7,8,9}),42)")]),
        CalculationOptions::default().with_limits(work_limits),
    );
    assert_issue(&xmatch, 1, CalculationIssueCode::ResourceLimitExceeded);

    let approximate_limits = CalculationLimits::default()
        .with_max_function_iterations(7)
        .expect("nonzero approximate XMATCH iteration limit");
    let approximate = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "IFERROR(XMATCH(4,{1,2,3},-1),42)")]),
        CalculationOptions::default().with_limits(approximate_limits),
    );
    assert_issue(&approximate, 1, CalculationIssueCode::ResourceLimitExceeded);

    let upstream = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "1"),
            (2, 1, "MYSTERY()"),
            (1, 2, "IFERROR(XMATCH(1,A1:A2),42)"),
        ]),
        CalculationOptions::default(),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = upstream.cell(calculation_cell_id(2, 1))
    else {
        panic!("expected unsupported source formula in A2");
    };
    assert_eq!(issue.code(), CalculationIssueCode::UnsupportedFunction);
    assert_issue(&upstream, 2, CalculationIssueCode::BlockedByUpstream);
}

#[test]
fn modern_array_function_work_is_bounded_before_materialization() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "CHOOSECOLS({1;2},1,1,1)"),
        (1, 2, "TAKE({1,2,3;4,5,6},2,3)"),
        (1, 3, "HSTACK({1;2;3},{4})"),
        (1, 4, "EXPAND({1},3,3)"),
        (1, 5, "SORTBY({1;2;3},{3;2;1})"),
        (1, 6, "TOCOL({1,2,3;4,5,6})"),
        (1, 7, "TRIMRANGE(A100:C102)"),
        (1, 8, "WRAPROWS({1;2;3;4;5;6},2)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(5)
        .expect("positive iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
    for column in 1..=8 {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::ResourceLimitExceeded,
        );
    }
}

#[test]
fn cycles_are_distinct_from_downstream_failures() {
    let workbook = workbook_with_formulas(&[(1, 1, "B1"), (1, 2, "A1"), (1, 3, "A1+1")]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_issue(&calculation, 1, CalculationIssueCode::CircularReference);
    assert_issue(&calculation, 2, CalculationIssueCode::CircularReference);
    assert_issue(&calculation, 3, CalculationIssueCode::BlockedByUpstream);
}

#[test]
fn volatile_dates_require_an_explicit_deterministic_input() {
    let workbook = workbook_with_formulas(&[(1, 1, "TODAY()"), (1, 2, "NOW()")]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let missing = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&missing, 1, CalculationIssueCode::VolatileInputMissing);
    assert_issue(&missing, 2, CalculationIssueCode::VolatileInputMissing);

    let options = CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(46_225.0).expect("deterministic date is finite"))
        .with_now_serial(FiniteNumber::new(46_225.75).expect("deterministic time is finite"));
    let calculated = calculate_workbook(&workbook, options);
    assert_eq!(calculated.options(), options);
    assert_eq!(
        calculated.provenance().provider().name(),
        "cellrune.calculator"
    );
    assert_eq!(
        calculated.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(
            CellValue::number(46_225.0).expect("finite expected date")
        ))
    );
    assert_eq!(
        calculated.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(
            CellValue::number(46_225.75).expect("finite expected date-time")
        ))
    );
}

#[test]
fn volatile_dates_hidden_in_defined_names_keep_the_specific_issue() {
    let workbook = workbook_with_formulas_and_names(
        &[(1, 1, "NamedToday"), (1, 2, "NamedNow")],
        &[("NamedToday", "TODAY()"), ("NamedNow", "NOW()")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let missing = calculate_workbook(&workbook, CalculationOptions::default());
    assert_issue(&missing, 1, CalculationIssueCode::VolatileInputMissing);
    assert_issue(&missing, 2, CalculationIssueCode::VolatileInputMissing);

    let options = CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(46_225.0).expect("deterministic date is finite"))
        .with_now_serial(FiniteNumber::new(46_225.75).expect("deterministic time is finite"));
    let calculated = calculate_workbook(&workbook, options);
    assert_number(&calculated, 1, 46_225.0, 0.0);
    assert_number(&calculated, 2, 46_225.75, 0.0);
}

#[test]
fn volatile_detection_respects_local_and_defined_callable_shadowing() {
    let local_shadow =
        workbook_with_formulas(&[(1, 1, "LET(TODAY,LAMBDA(INDIRECT(\"A1\",FALSE)),TODAY())")]);
    let local_result = calculate_workbook(&local_shadow, CalculationOptions::default());
    assert_issue(
        &local_result,
        1,
        CalculationIssueCode::UnsupportedExpression,
    );

    let scalar_shadow = workbook_with_formulas(&[(1, 1, "LET(TODAY,2,TODAY(NOW()))")]);
    let scalar_result = calculate_workbook(&scalar_shadow, CalculationOptions::default());
    assert_eq!(
        scalar_result.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );

    let defined_shadow = workbook_with_formulas_and_names(
        &[(1, 1, "MAP(1,LAMBDA(x,x))")],
        &[("MAP", "LAMBDA(x,callback,TODAY())")],
    );
    let defined_result = calculate_workbook(&defined_shadow, CalculationOptions::default());
    assert_issue(
        &defined_result,
        1,
        CalculationIssueCode::VolatileInputMissing,
    );
}

#[test]
fn calculation_limits_reject_zero_values() {
    assert_eq!(
        CalculationLimits::default().with_max_formula_tokens(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_formula_tokens",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_formula_source_bytes(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_formula_source_bytes",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_dependency_edges(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_dependency_edges",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_reference_areas(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_reference_areas",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_text_bytes(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_text_bytes",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_function_iterations(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_function_iterations",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_let_bindings(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_let_bindings",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_lambda_depth(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_lambda_depth",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_lambda_invocations(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_lambda_invocations",
        })
    );
}

#[test]
fn parser_and_dependency_budgets_return_stable_resource_issues() {
    let parser_workbook = workbook_with_formulas(&[(1, 1, "1+2+3")]);
    let parser_limits = CalculationLimits::default()
        .with_max_formula_tokens(3)
        .expect("nonzero parser limit");
    let parser_options = CalculationOptions::default().with_limits(parser_limits);
    let parser_report = scan_formula_capabilities_with_options(&parser_workbook, parser_options);
    assert_capability_issue(
        &parser_report,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
        Some("max_formula_tokens"),
    );
    let parser_calculation = calculate_workbook(&parser_workbook, parser_options);
    assert_issue(
        &parser_calculation,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
    );

    let source_limits = CalculationLimits::default()
        .with_max_formula_source_bytes(3)
        .expect("nonzero formula source limit");
    let source_report = scan_formula_capabilities_with_options(
        &workbook_with_formulas(&[(1, 1, "표A")]),
        CalculationOptions::default().with_limits(source_limits),
    );
    assert_capability_issue(
        &source_report,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
        Some("max_formula_source_bytes"),
    );

    let ast_limits = CalculationLimits::default()
        .with_max_formula_ast_nodes(2)
        .expect("nonzero AST limit");
    let ast_report = scan_formula_capabilities_with_options(
        &workbook_with_formulas(&[(1, 1, "1+2")]),
        CalculationOptions::default().with_limits(ast_limits),
    );
    assert_capability_issue(
        &ast_report,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
        Some("max_formula_ast_nodes"),
    );

    let depth_limits = CalculationLimits::default()
        .with_max_formula_nesting_depth(1)
        .expect("nonzero nesting limit");
    let depth_report = scan_formula_capabilities_with_options(
        &workbook_with_formulas(&[(1, 1, "(1)")]),
        CalculationOptions::default().with_limits(depth_limits),
    );
    assert_capability_issue(
        &depth_report,
        1,
        CalculationIssueCode::ResourceLimitExceeded,
        Some("max_formula_nesting_depth"),
    );

    let dependency_workbook = workbook_with_formulas(&[(1, 1, "1"), (1, 2, "A1"), (1, 3, "A1+B1")]);
    let dependency_limits = CalculationLimits::default()
        .with_max_dependency_edges(1)
        .expect("nonzero dependency limit");
    let dependency_options = CalculationOptions::default().with_limits(dependency_limits);
    let dependency_report =
        scan_formula_capabilities_with_options(&dependency_workbook, dependency_options);
    assert_eq!(dependency_report.unsupported_count(), 3);
    for column in 1..=3 {
        assert_capability_issue(
            &dependency_report,
            column,
            CalculationIssueCode::ResourceLimitExceeded,
            Some("max_dependency_edges"),
        );
    }
}

#[test]
fn array_budget_cannot_be_hidden_by_iferror() {
    let workbook = workbook_with_formulas(&[(1, 1, "IFERROR(SUM({1,2,3}),42)")]);
    let limits = CalculationLimits::default()
        .with_max_array_cells(2)
        .expect("nonzero array limit");
    let options = CalculationOptions::default().with_limits(limits);

    assert!(scan_formula_capabilities_with_options(&workbook, options).is_supported());
    let calculation = calculate_workbook(&workbook, options);
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(1)) else {
        panic!("array budget must withhold the calculated value");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_array_cells"));
}

#[test]
fn text_budget_cannot_be_hidden_and_empty_find_is_safe() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "IFERROR(\"ab\"&\"cd\",\"hidden\")"),
        (1, 2, "FIND(\"\",\"abc\")"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_text_bytes(3)
        .expect("nonzero text limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(1)) else {
        panic!("text budget must withhold the calculated value");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_text_bytes"));
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(
            CellValue::number(1.0).expect("finite FIND result")
        ))
    );
}

#[test]
fn defined_name_cycles_and_expansion_depth_are_bounded_before_evaluation() {
    let cyclic = workbook_with_formulas_and_names(
        &[(1, 1, "Alpha")],
        &[("Alpha", "Beta"), ("Beta", "Alpha")],
    );
    let cyclic_calculation = calculate_workbook(&cyclic, CalculationOptions::default());
    assert_issue(
        &cyclic_calculation,
        1,
        CalculationIssueCode::UnsupportedName,
    );

    let chain = workbook_with_formulas_and_names(
        &[(1, 1, "Alpha")],
        &[("Alpha", "Beta"), ("Beta", "Gamma"), ("Gamma", "1")],
    );
    let limits = CalculationLimits::default()
        .with_max_formula_nesting_depth(2)
        .expect("nonzero name expansion depth");
    let calculation = calculate_workbook(&chain, CalculationOptions::default().with_limits(limits));
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(1)) else {
        panic!("defined name expansion limit must withhold the result");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_formula_nesting_depth"));

    let branching_chain = workbook_with_formulas_and_names(
        &[(1, 1, "Deep+Shallow")],
        &[
            ("Deep", "Middle"),
            ("Middle", "Tail"),
            ("Shallow", "Tail"),
            ("Tail", "1"),
        ],
    );
    let branching_calculation = calculate_workbook(
        &branching_chain,
        CalculationOptions::default().with_limits(limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = branching_calculation.cell(cell_id(1))
    else {
        panic!("a shallow shared tail must not hide a deeper defined-name path");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_formula_nesting_depth"));

    let callable_chain = workbook_with_formulas_and_names(
        &[(1, 1, "Reader(0)")],
        &[
            ("Reader", "LAMBDA(value,Alpha)"),
            ("Alpha", "Beta"),
            ("Beta", "Gamma"),
            ("Gamma", "1"),
        ],
    );
    let callable_calculation = calculate_workbook(
        &callable_chain,
        CalculationOptions::default().with_limits(limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = callable_calculation.cell(cell_id(1))
    else {
        panic!("named lambda body must not bypass defined-name expansion limits");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_formula_nesting_depth"));

    let branching_callable_chain = workbook_with_formulas_and_names(
        &[(1, 1, "Deep(0)+Shallow(0)")],
        &[
            ("Deep", "LAMBDA(value,Middle(value))"),
            ("Middle", "LAMBDA(value,Tail(value))"),
            ("Shallow", "LAMBDA(value,Tail(value))"),
            ("Tail", "LAMBDA(value,1)"),
        ],
    );
    let branching_callable_calculation = calculate_workbook(
        &branching_callable_chain,
        CalculationOptions::default().with_limits(limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) =
        branching_callable_calculation.cell(cell_id(1))
    else {
        panic!("a shallow callable tail must not hide a deeper callable path");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_formula_nesting_depth"));
}

#[test]
fn function_iterations_and_extreme_coordinate_arithmetic_are_bounded() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "IFERROR(WORKDAY(1,100),42)"),
        (1, 2, "OFFSET(D1,9E307,0)"),
        (1, 3, "DATE(9E307,1,1)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero function iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(1)) else {
        panic!("function iteration budget must withhold the result");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_function_iterations"));
    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Reference
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(3)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
}

#[test]
fn audited_math_dates_rows_and_general_text_follow_excel_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SIGN(0)"),
        (1, 2, "CEILING(-2.5,2)"),
        (1, 3, "CEILING(-2.5,-2)"),
        (1, 4, "FLOOR(-2.5,2)"),
        (1, 5, "FLOOR(-2.5,-2)"),
        (1, 6, "POWER(0,0)"),
        (1, 7, "MOD(3,0)"),
        (1, 8, "DAY(0)"),
        (1, 9, "MONTH(0)"),
        (1, 10, "YEAR(0)"),
        (1, 11, "WEEKDAY(0)"),
        (1, 12, "DATE(1900,1,0)"),
        (1, 13, "0.1+0.2&\"\""),
        (1, 14, "1.234567890123456&\"\""),
        (1, 15, "DATE(1900,2,29)"),
        (1, 16, "DATE(1900,3,0)"),
        (1, 17, "DAY(60)"),
        (1, 18, "MONTH(60)"),
        (1, 19, "YEAR(60)"),
        (1, 20, "WEEKDAY(60)"),
        (7, 1, "ROW()"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for (column, expected) in [
        (1, 0.0),
        (2, -2.0),
        (3, -4.0),
        (4, -4.0),
        (5, -2.0),
        (8, 0.0),
        (9, 1.0),
        (10, 1900.0),
        (11, 7.0),
        (12, 0.0),
        (15, 60.0),
        (16, 60.0),
        (17, 29.0),
        (18, 2.0),
        (19, 1900.0),
        (20, 4.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for (column, error) in [(6, ExcelError::Number), (7, ExcelError::DivisionByZero)] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error)))
        );
    }
    assert_eq!(
        calculation.cell(cell_id(13)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "0.3".to_owned()
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(14)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "1.23456789012345".to_owned()
        )))
    );
    assert_number_at(&calculation, 7, 1, 7.0, 0.0);
}

#[test]
fn real_workbook_regressions_follow_excel_argument_and_lookup_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "VLOOKUP(534,N2:O4,2)"),
        (1, 2, "SUMX2PY2({1,\"x\",2},{3,4,TRUE})"),
        (1, 3, "SUMXMY2({\"x\",TRUE},{1,2})"),
        (1, 4, "SUMPRODUCT(2,{3;4})"),
        (1, 5, "AND(\"1\",1)"),
        (1, 6, "FLOOR(1,0)"),
        (1, 7, "AVERAGEA(\"TRUE\",1)"),
        (1, 8, "COUNT(2,\"A\",\"\",#REF!,#DIV/0!)"),
        (1, 9, "MODE(2,\"2\",3)"),
        (1, 10, "MODE({1,1,0;2,1,0;3,1,0;4,1,0},0)"),
        (1, 11, "COUNTIF(L2:L3,1/0)"),
        (2, 12, "1/0"),
        (3, 12, "1"),
        (2, 14, "\"INTEGRAL\""),
        (2, 15, "10"),
        (3, 14, "0"),
        (3, 15, "20"),
        (4, 14, "534"),
        (4, 15, "30"),
        (1, 16, "\"0.1\""),
        (1, 17, "AND(P1,\"1\")"),
        (2, 16, "0"),
        (3, 16, "1"),
        (2, 17, "\"\""),
        (1, 18, "AND(P2:P3,Q2)"),
        (1, 19, "CEILING(1,T2)"),
        (1, 20, "CEILING(1,0)"),
        (1, 21, "MODE(2,2,U2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 1, 30.0, 0.0);
    assert_number(&calculation, 2, 10.0, 0.0);
    for (column, error) in [
        (3, ExcelError::DivisionByZero),
        (4, ExcelError::Value),
        (6, ExcelError::DivisionByZero),
        (7, ExcelError::Value),
        (9, ExcelError::NotAvailable),
        (17, ExcelError::Value),
        (20, ExcelError::DivisionByZero),
        (21, ExcelError::Value),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error)))
        );
    }
    assert_eq!(
        calculation.cell(cell_id(5)),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );
    assert_number(&calculation, 8, 1.0, 0.0);
    assert_number(&calculation, 10, 1.0, 0.0);
    assert_number(&calculation, 11, 1.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(18)),
        Some(&CalculationCellResult::Value(CellValue::Logical(false)))
    );
    assert_number(&calculation, 19, 0.0, 0.0);
}

#[test]
fn dynamic_references_add_dependencies_after_their_arguments_are_calculated() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"D1\""),
        (1, 2, "INDIRECT(A1)"),
        (2, 2, "_xlfn.INDIRECT(A1)"),
        (1, 4, "40+2"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 2, 42.0, 0.0);
    assert_number_at(&calculation, 2, 2, 42.0, 0.0);
}

#[test]
fn wildcard_and_broadcast_work_are_bounded_before_iferror_can_hide_it() {
    let wildcard_workbook = workbook_with_formulas(&[
        (1, 1, "IFERROR(COUNTIF(A2,\"*a*a*a*a*a*a*a*a*b\"),42)"),
        (2, 1, "\"aaaaaaaa\""),
    ]);
    let wildcard_limits = CalculationLimits::default()
        .with_max_function_iterations(5)
        .expect("nonzero wildcard iteration limit");
    let wildcard = calculate_workbook(
        &wildcard_workbook,
        CalculationOptions::default().with_limits(wildcard_limits),
    );
    assert_issue(&wildcard, 1, CalculationIssueCode::ResourceLimitExceeded);

    let binary_limits = CalculationLimits::default()
        .with_max_array_cells(8)
        .expect("nonzero binary array limit");
    let binary = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "IFERROR(SUM({1,2,3}+{10;20;30}),42)")]),
        CalculationOptions::default().with_limits(binary_limits),
    );
    assert_issue(&binary, 1, CalculationIssueCode::ResourceLimitExceeded);

    let sumproduct_limits = CalculationLimits::default()
        .with_max_array_cells(3)
        .expect("nonzero SUMPRODUCT array limit");
    let sumproduct = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "IFERROR(SUMPRODUCT({1,2,3,4},{10,20,30,40}),42)")]),
        CalculationOptions::default().with_limits(sumproduct_limits),
    );
    assert_issue(&sumproduct, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn eomonth_and_xirr_follow_excel_date_and_financial_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "EOMONTH(DATE(2011,1,1),1)"),
        (1, 2, "EOMONTH(DATE(2011,1,31),-1)"),
        (
            1,
            3,
            "XIRR({-10000,2750,4250,3250,2750},{39448,39508,39751,39859,39904},0.1)",
        ),
        (1, 4, "XIRR({-1,2},{39448})"),
        (1, 5, "XIRR({-1,2},{39448,39447})"),
        (1, 6, "XIRR({-1,2},{-1,39448})"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 1, 40_602.0, 0.0);
    assert_number(&calculation, 2, 40_543.0, 0.0);
    assert_number(&calculation, 3, 0.373_362_533_519, 1e-12);
    assert_eq!(
        calculation.cell(cell_id(4)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(5)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(6)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
}

#[test]
fn leading_plus_is_a_formula_prefix_without_text_coercion() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"EBITDA\""),
        (1, 2, "+A1"),
        (1, 3, "1+(+A1)"),
        (1, 4, "Z99"),
        (1, 5, "+Z99"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_eq!(
        calculation.cell(cell_id(2)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "EBITDA".to_owned()
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(3)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
    assert_number(&calculation, 4, 0.0, 0.0);
    assert_number(&calculation, 5, 0.0, 0.0);
}

#[test]
fn a1_absolute_mixed_range_and_defined_name_references_calculate() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "10"),
            (2, 1, "20"),
            (1, 2, "$A$1"),
            (1, 3, "$A$1+$A1+A$1"),
            (1, 4, "SUM(A:A)"),
            (1, 5, "SUM(2:2)"),
            (1, 6, "SUM(A1:A2)"),
            (1, 7, "Amount"),
        ],
        &[("Amount", "$A$1")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, column, expected) in [
        (1, 2, 10.0),
        (1, 3, 30.0),
        (1, 4, 30.0),
        (1, 5, 20.0),
        (1, 6, 30.0),
        (1, 7, 10.0),
    ] {
        assert_number_at(&calculation, row, column, expected, 0.0);
    }
}

#[test]
fn implicit_intersection_uses_the_formula_cell_position() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "10"),
            (2, 1, "20"),
            (3, 1, "30"),
            (1, 3, "11"),
            (2, 2, "@A1:A3"),
            (2, 3, "@A1:C1"),
            (2, 4, "_xlfn.SINGLE(A1:A3)"),
            (2, 5, "SUM(@A1:A3)"),
            (2, 6, "@OFFSET(A1,0,0,3,1)"),
            (2, 7, "@{7,8;9,10}"),
            (4, 8, "@A1:A3"),
            (2, 9, "@A1:B3"),
            (2, 10, "@42"),
            (2, 11, "@Vector"),
            // Parenthesised operands: Excel round-trips `_xlfn.SINGLE((A1:A3))` into this shape,
            // so `@` has to look through parentheses and through a nested `@` before it decides
            // what kind of operand it is intersecting. Dispatching on the unopened operand makes
            // these silently return the first element instead of the intersecting row.
            (2, 12, "@(A1:A3)"),
            (2, 13, "_xlfn.SINGLE((A1:A3))"),
            (2, 14, "@(@A1:A3)"),
            (2, 15, "@((A1:A3))"),
            (2, 16, "SUM(@(A1:A3))"),
        ],
        &[("Vector", "A1:A3")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, column, expected) in [
        (2, 2, 20.0),
        (2, 3, 11.0),
        (2, 4, 20.0),
        (2, 5, 20.0),
        (2, 6, 20.0),
        (2, 7, 7.0),
        (2, 10, 42.0),
        (2, 11, 20.0),
        (2, 12, 20.0),
        (2, 13, 20.0),
        (2, 14, 20.0),
        (2, 15, 20.0),
        (2, 16, 20.0),
    ] {
        assert_number_at(&calculation, row, column, expected, 0.0);
    }
    for (row, column) in [(4, 8), (2, 9)] {
        assert_eq!(
            calculation.cell(calculation_cell_id(row, column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
}

#[test]
fn whole_column_operators_share_one_extent_and_one_cumulative_budget() {
    let calculation_sheet = formula_sheet(
        1,
        "Calculation",
        &[
            (1, 1, "SUM(LongData!A:A*ShortData!B:B)"),
            (1, 2, "SUM(INDEX(LongData!A:A,0))"),
            (1, 3, "SUM(EmptyData!A:A*1)"),
            (1, 4, "SUM(-LongData!A:A)"),
            (1, 5, "SUM(LongData!A:A*INDEX(LongData!A:A,0))"),
            (1, 6, "SUM(ABS(LongData!A:A))"),
            (1, 7, "SUM(ABS(LongData!A:A)+1)"),
        ],
    );
    let mut long_data = Sheet::new(
        SheetId::new(2).expect("valid sheet ID"),
        SheetName::new("LongData").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (address, value) in [("A1", 1.0), ("A2", 2.0), ("A3", 3.0)] {
        insert_number(&mut long_data, address, value);
    }
    let mut short_data = Sheet::new(
        SheetId::new(3).expect("valid sheet ID"),
        SheetName::new("ShortData").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (address, value) in [("B1", 10.0), ("B2", 20.0)] {
        insert_number(&mut short_data, address, value);
    }
    let empty_data = Sheet::new(
        SheetId::new(4).expect("valid sheet ID"),
        SheetName::new("EmptyData").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let workbook = workbook_with_sheets_and_names(
        vec![calculation_sheet, long_data, short_data, empty_data],
        &[],
    );

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 50.0, 0.0);
    assert_number(&calculation, 2, 6.0, 0.0);
    assert_number(&calculation, 3, 0.0, 0.0);
    assert_number(&calculation, 4, -6.0, 0.0);
    assert_number(&calculation, 5, 14.0, 0.0);
    assert_number(&calculation, 6, 6.0, 0.0);
    assert_number(&calculation, 7, 9.0, 0.0);

    for (max_array_cells, succeeds) in [(9, true), (8, false)] {
        let limits = CalculationLimits::default()
            .with_max_array_cells(max_array_cells)
            .expect("nonzero array limit");
        let limited =
            calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
        if succeeds {
            assert_number(&limited, 1, 50.0, 0.0);
            assert_number(&limited, 5, 14.0, 0.0);
        } else {
            assert_issue(&limited, 1, CalculationIssueCode::ResourceLimitExceeded);
            assert_issue(&limited, 5, CalculationIssueCode::ResourceLimitExceeded);
        }
        assert_number(&limited, 2, 6.0, 0.0);
        assert_number(&limited, 3, 0.0, 0.0);
        assert_number(&limited, 4, -6.0, 0.0);
        assert_number(&limited, 6, 6.0, 0.0);
        assert_number(&limited, 7, 9.0, 0.0);
    }

    for (max_array_cells, succeeds) in [(6, true), (5, false)] {
        let limits = CalculationLimits::default()
            .with_max_array_cells(max_array_cells)
            .expect("nonzero array limit");
        let limited =
            calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
        if succeeds {
            assert_number(&limited, 4, -6.0, 0.0);
        } else {
            assert_issue(&limited, 4, CalculationIssueCode::ResourceLimitExceeded);
        }
    }

    for (max_array_cells, succeeds) in [(3, true), (2, false)] {
        let limits = CalculationLimits::default()
            .with_max_array_cells(max_array_cells)
            .expect("nonzero array limit");
        let limited =
            calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
        if succeeds {
            assert_number(&limited, 6, 6.0, 0.0);
            assert_number(&limited, 7, 9.0, 0.0);
        } else {
            assert_issue(&limited, 6, CalculationIssueCode::ResourceLimitExceeded);
            assert_issue(&limited, 7, CalculationIssueCode::ResourceLimitExceeded);
        }
    }
}

#[test]
fn whole_column_extent_ignores_columns_the_expression_does_not_reference() {
    // The materialized height must come from the referenced columns alone. Taking it from the
    // sheet-wide used range would make these values depend on cells no dependency rectangle
    // covers, which is what splits full and incremental recalculation.
    let calculation_sheet = formula_sheet(
        1,
        "Calculation",
        &[
            (1, 1, "COUNT(Data!A:A*Data!B:B)"),
            (1, 2, "AVERAGE(Data!A:A*Data!B:B)"),
            (1, 3, "SUM(Data!A:A*Data!B:B)"),
        ],
    );
    let mut short_columns = Sheet::new(
        SheetId::new(2).expect("valid sheet ID"),
        SheetName::new("Data").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (address, value) in [
        ("A1", 1.0),
        ("A2", 2.0),
        ("A3", 3.0),
        ("B1", 10.0),
        ("B2", 20.0),
    ] {
        insert_number(&mut short_columns, address, value);
    }
    let mut tall_unrelated_column = short_columns.clone();
    insert_number(&mut tall_unrelated_column, "Z10", 999.0);

    for data in [short_columns, tall_unrelated_column] {
        let workbook = workbook_with_sheets_and_names(vec![calculation_sheet.clone(), data], &[]);
        let calculation = calculate_workbook(&workbook, CalculationOptions::default());
        assert_number(&calculation, 1, 3.0, 0.0);
        assert_number(&calculation, 2, 50.0 / 3.0, 1e-12);
        assert_number(&calculation, 3, 50.0, 0.0);
    }
}

#[test]
fn a_defined_three_d_name_is_classified_the_same_in_either_operand_order() {
    // The sheet-range diagnosis depends on which function consumes the name, so the capability
    // scan must reach the name under both policies rather than memoizing whichever operand the
    // walk happened to visit first.
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Span3D)+COUNTBLANK(Span3D)"),
            (1, 2, "COUNTBLANK(Span3D)+SUM(Span3D)"),
            (1, 3, "SUM(Span3D)"),
        ],
        &[("Span3D", "Sheet1:Sheet3!Z1")],
    );
    let report = scan_formula_capabilities(&workbook);
    assert_eq!(report.supported_count(), 1);
    for column in 1..=2 {
        assert_capability_issue(
            &report,
            column,
            CalculationIssueCode::UnsupportedSheetRange,
            Some("Sheet1:Sheet3"),
        );
    }

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=2 {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::UnsupportedSheetRange,
        );
    }
    assert_number(&calculation, 3, 111.0, 0.0);
}

#[test]
fn the_range_operator_reports_a_three_d_operand_as_an_excel_value_error() {
    // The scanner classifies range-operator positions with the array-expression policy, so the
    // evaluator has to answer with Excel's `#VALUE!` rather than an engine-capability error.
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Sheet1:Sheet3!Z1:Sheet1:Sheet3!Z2)"),
            (1, 2, "SUM(Sheet2!Z1:Sheet1:Sheet3!Z2)"),
            (1, 3, "SUM(Span3D:Z2)"),
        ],
        &[("Span3D", "Sheet1:Sheet3!Z1")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=3 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "column {column}",
        );
    }
}

#[test]
fn cross_sheet_references_use_the_workbook_unicode_name_index() {
    let workbook = workbook_with_sheets_and_names(
        vec![
            formula_sheet(1, "Ä", &[(1, 1, "41")]),
            formula_sheet(2, "Calc", &[(1, 1, "ä!A1+1")]),
        ],
        &[],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let formula_id = CalculationCellId::new(
        SheetId::new(2).expect("valid calculation sheet ID"),
        CellAddress::from_a1("A1").expect("valid formula address"),
    );
    assert_eq!(
        calculation.cell(formula_id),
        Some(&CalculationCellResult::Value(
            CellValue::number(42.0).expect("finite expected value")
        ))
    );
}

#[test]
fn normal_formulas_apply_legacy_implicit_intersection() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "10"),
        (2, 1, "20"),
        (3, 1, "30"),
        (1, 2, "11"),
        (2, 2, "22"),
        (3, 2, "33"),
        (1, 11, "\"top\""),
        (2, 11, "\"middle\""),
        (3, 11, "\"bottom\""),
        (2, 3, "ABS(A1:A3)"),
        (4, 2, "A3:B3+1"),
        (4, 1, "COUNTIF(A1:A3,A1:B1)"),
        (4, 5, "ABS(A1:A3)"),
        (2, 6, "ABS(A1:B3)"),
        (2, 8, "T(K1:K3)"),
        (2, 9, "T(A1:B1)"),
        (2, 10, "OFFSET(A1,0,0,3,1)"),
        (6, 11, "INDEX(A1:B2,0,1)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, column, expected) in [(2, 3, 20.0), (4, 2, 34.0), (4, 1, 1.0), (2, 10, 20.0)] {
        assert_number_at(&calculation, row, column, expected, 0.0);
    }
    for (row, column) in [(4, 5), (2, 6), (6, 11)] {
        assert_eq!(
            calculation.cell(calculation_cell_id(row, column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
    assert_eq!(
        calculation.cell(calculation_cell_id(2, 8)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "top".to_owned()
        )))
    );
    assert_eq!(
        calculation.cell(calculation_cell_id(2, 9)),
        Some(&CalculationCellResult::Value(
            CellValue::Text(String::new())
        ))
    );
}

#[test]
fn index_zero_returns_complete_references_with_legacy_intersection() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "10"),
        (2, 1, "20"),
        (3, 1, "30"),
        (1, 2, "11"),
        (2, 2, "22"),
        (3, 2, "33"),
        (2, 12, "INDEX(A1:B3,0,1)"),
        (3, 12, "@INDEX(A1:B3,0,1)"),
        (5, 1, "INDEX(A1:B3,2,0)"),
        (5, 3, "SUM(INDEX(A1:B3,0,1))"),
        (5, 4, "SUM(INDEX(A1:B3,2,0))"),
        (5, 5, "SUM(INDEX(A1:B3,0,0))"),
        (5, 6, "SUM(INDEX(A1:B1,0))"),
        (5, 7, "SUM(INDEX(A1:A3,0))"),
        (5, 8, "INDEX(A1:B3,-1,1)"),
        (5, 9, "INDEX(A1:B3,4,1)"),
        (5, 10, "INDEX(A1:B3,0,0)"),
        (6, 11, "INDEX(A1:B2,0,1)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, column, expected) in [
        (2, 12, 20.0),
        (3, 12, 30.0),
        (5, 1, 20.0),
        (5, 3, 60.0),
        (5, 4, 42.0),
        (5, 5, 126.0),
        (5, 6, 21.0),
        (5, 7, 60.0),
    ] {
        assert_number_at(&calculation, row, column, expected, 0.0);
    }
    for (row, column, expected) in [
        (5, 8, ExcelError::Value),
        (5, 9, ExcelError::Reference),
        (5, 10, ExcelError::Value),
        (6, 11, ExcelError::Value),
    ] {
        assert_eq!(
            calculation.cell(calculation_cell_id(row, column)),
            Some(&CalculationCellResult::Value(CellValue::Error(expected)))
        );
    }
}

#[test]
fn index_zero_materializes_row_column_and_complete_dynamic_arrays() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, value) in [
        ("A1", 1.0),
        ("A2", 2.0),
        ("A3", 3.0),
        ("B1", 10.0),
        ("B2", 20.0),
        ("B3", 30.0),
    ] {
        draft
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1(address).expect("valid input address"),
                CellValue::number(value).expect("finite input"),
            )
            .expect("input mutation");
    }
    for (address, formula) in [
        ("D1", "=INDEX(A1:B3,0,2)"),
        ("F1", "=INDEX(A1:B3,2,0)"),
        ("H1", "=INDEX(A1:B3,0,0)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid INDEX formula"),
                None,
            )
            .expect("dynamic INDEX mutation");
    }
    assert!(scan_formula_capabilities(draft.workbook()).is_supported());

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("D1", 10.0),
        ("D2", 20.0),
        ("D3", 30.0),
        ("F1", 2.0),
        ("G1", 20.0),
        ("H1", 1.0),
        ("I1", 10.0),
        ("H2", 2.0),
        ("I2", 20.0),
        ("H3", 3.0),
        ("I3", 30.0),
    ] {
        assert_materialized_number(&calculation, sheet_id, address, expected);
    }
}

#[test]
fn lookup_work_respects_the_function_iteration_budget() {
    let limits = CalculationLimits::default()
        .with_max_function_iterations(7)
        .expect("nonzero LOOKUP iteration limit");
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "LOOKUP(4,{1,2,3,4})")]),
        CalculationOptions::default().with_limits(limits),
    );

    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn range_operator_accepts_reference_returning_expressions() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "1"),
        (2, 1, "2"),
        (3, 1, "3"),
        (4, 1, "4"),
        (1, 2, "SUM(A1:OFFSET(A1,2,0))"),
        (2, 2, "SUM(OFFSET(A1,1,0):OFFSET(A1,3,0))"),
        (3, 2, "AVERAGE(OFFSET(A1,0,0):OFFSET(A1,3,0))"),
        (4, 2, "SUM(A3:OFFSET(A1,0,0))"),
        (5, 2, "PRODUCT(1+OFFSET(A1,0,0):OFFSET(A1,2,0))"),
        (6, 2, "SUM(INDEX(A1:A4,2):INDEX(A1:A4,4))"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, expected) in [(1, 6.0), (2, 9.0), (3, 2.5), (4, 6.0), (5, 24.0), (6, 9.0)] {
        assert_number_at(&calculation, row, 2, expected, 0.0);
    }
}

#[test]
fn aggregates_accept_offset_rectangles_with_omitted_dimensions() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "1"),
        (1, 2, "2"),
        (2, 1, "3"),
        (2, 2, "4"),
        (1, 3, "SUM(OFFSET(A1:B2,0,0,1,))"),
        (2, 3, "SUM(OFFSET(A1:B2,0,0,,1))"),
        (3, 3, "SUM(OFFSET(A1:B2,0,0,1))"),
        (4, 3, "SUM(OFFSET(A1:B2,0,0))"),
        (5, 3, "SUM(OFFSET(A1:B2,0,0,,))"),
        (6, 3, "SUM(OFFSET(A1:B2,0,0,))"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (row, expected) in [
        (1, 3.0),
        (2, 4.0),
        (3, 3.0),
        (4, 10.0),
        (5, 10.0),
        (6, 10.0),
    ] {
        assert_number_at(&calculation, row, 3, expected, 0.0);
    }
}

#[test]
fn search_reports_positions_in_the_original_text_after_case_folding() {
    // Lowercasing "İ" (U+0130) yields two characters, so a position computed in
    // the folded text drifts from the caller's text unless it is mapped back.
    let workbook = workbook_with_formulas(&[
        (1, 1, "SEARCH(\"x\",\"İX\")"),
        (1, 2, "SEARCH(\"b\",\"İxAB\")"),
        (1, 3, "SEARCH(\"a\",\"İxAB\",3)"),
        (1, 4, "FIND(\"X\",\"İX\")"),
        (1, 5, "SEARCH(\"x\",\"aXc\")"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 2.0, 0.0);
    assert_number(&calculation, 2, 4.0, 0.0);
    assert_number(&calculation, 3, 3.0, 0.0);
    assert_number(&calculation, 4, 2.0, 0.0);
    assert_number(&calculation, 5, 2.0, 0.0);
}

#[test]
fn count_right_and_legacy_statistical_aliases_follow_excel_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "1"),
        (1, 2, "\"2\""),
        (1, 3, "TRUE"),
        (1, 4, "\"x\""),
        (1, 5, "COUNT(A1:D1)"),
        (1, 6, "COUNT(1,\"2\",TRUE,\"x\")"),
        (1, 7, "RIGHT(\"가나다\",2)"),
        (1, 8, "PERCENTILE({1,2,3,4},0.5)"),
        (1, 9, "STDEV({1,2,3})"),
        (1, 10, "SUM(1,\"2\",TRUE)"),
        (1, 11, "SUM({1,\"2\",TRUE})"),
        (1, 12, "MIN(IF({1,0,1},{3,4,2},\"\"))"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 5, 1.0, 0.0);
    assert_number(&calculation, 6, 3.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(7)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "나다".to_owned()
        )))
    );
    assert_number(&calculation, 8, 2.5, 0.0);
    assert_number(&calculation, 9, 1.0, 0.0);
    assert_number(&calculation, 10, 4.0, 0.0);
    assert_number(&calculation, 11, 1.0, 0.0);
    assert_number(&calculation, 12, 2.0, 0.0);
}

#[test]
fn xls_corpus_math_and_normal_distribution_functions_follow_excel_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "EXP(1)"),
        (1, 2, "LN(EXP(3))"),
        (1, 3, "LN(0)"),
        (1, 4, "EXP(1000)"),
        (1, 5, "NORMSDIST(1.333333)"),
        (1, 6, "NORM.S.DIST(1.333333,TRUE)"),
        (1, 7, "NORM.S.DIST(1.333333,FALSE)"),
        (1, 8, "NORMSDIST(\"not a number\")"),
        (1, 9, "NORM.S.DIST(0)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, std::f64::consts::E, 1e-15);
    assert_number(&calculation, 2, 3.0, 1e-15);
    assert_number(&calculation, 5, 0.908_788_725_604_095_5, 1e-15);
    assert_number(&calculation, 6, 0.908_788_725_604_095_5, 1e-15);
    assert_number(&calculation, 7, 0.164_010_147_569_367_22, 1e-15);
    for (column, error) in [
        (3, ExcelError::Number),
        (4, ExcelError::Number),
        (8, ExcelError::Value),
        (9, ExcelError::Value),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error)))
        );
    }
}

#[test]
fn yearfrac_supports_excel_day_count_bases() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30))"),
        (1, 2, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),1)"),
        (1, 3, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),2)"),
        (1, 4, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),3)"),
        (1, 5, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),4)"),
        (1, 6, "YEARFRAC(DATE(2012,7,30),DATE(2012,1,1),3)"),
        (1, 7, "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),5)"),
        (1, 8, "YEARFRAC(DATE(2019,7,1),DATE(2020,6,30),1)"),
        (1, 9, "YEARFRAC(DATE(2020,7,1),DATE(2021,6,30),1)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [
        (1, 209.0 / 360.0),
        (2, 211.0 / 366.0),
        (3, 211.0 / 360.0),
        (4, 211.0 / 365.0),
        (5, 209.0 / 360.0),
        (6, -211.0 / 365.0),
        (8, 365.0 / 366.0),
        (9, 364.0 / 365.0),
    ] {
        assert_number(&calculation, column, expected, 1e-12);
    }
    assert_eq!(
        calculation.cell(cell_id(7)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
}

#[test]
fn paired_statistics_and_percent_rank_follow_excel_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "CORREL({3,2,4,5,6},{9,7,12,15,17})"),
        (1, 2, "SLOPE({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})"),
        (1, 3, "CORREL({1,\"ignored\",3},{2,4,6})"),
        (1, 4, "SLOPE({1,2},{1,1})"),
        (1, 5, "CORREL({1,2},{1})"),
        (1, 6, "PERCENTRANK.INC({13,12,11,8,4,3,2,1,1,1},2)"),
        (1, 7, "PERCENTRANK({13,12,11,8,4,3,2,1,1,1},4)"),
        (1, 8, "PERCENTRANK.INC({13,12,11,8,4,3,2,1,1,1},5)"),
        (1, 9, "PERCENTRANK.INC({1,2,3},2,4)"),
        (1, 10, "PERCENTRANK.INC({1,2,3},4)"),
        (1, 11, "PERCENTRANK.INC({\"ignored\"},1)"),
        (1, 12, "PERCENTRANK.INC({1,2,3},1.3334)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected, tolerance) in [
        (1, 0.997_054_485_501_581_5, 1e-15),
        (2, 0.305_555_555_555_555_6, 1e-15),
        (3, 1.0, 1e-15),
        (6, 0.333, 0.0),
        (7, 0.555, 0.0),
        (8, 0.583, 0.0),
        (9, 0.5, 0.0),
        (12, 0.166, 0.0),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
    for (column, error) in [
        (4, ExcelError::DivisionByZero),
        (5, ExcelError::NotAvailable),
        (10, ExcelError::NotAvailable),
        (11, ExcelError::Number),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error)))
        );
    }
}

#[test]
fn matrix_and_array_functions_preserve_shape_and_excel_errors() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(MMULT({1,2;3,4},{5,6;7,8}))"),
        (1, 2, "MMULT({1,2},{3;4})"),
        (1, 3, "SUM(TRANSPOSE({1,2;3,4}))"),
        (1, 4, "SUM(SEQUENCE(2,3,10,2))"),
        (1, 5, "SUM(SEQUENCE(3))"),
        (1, 6, "MMULT({1,2},{3,4})"),
        (1, 7, "MMULT({1,\"text\"},{3;4})"),
        (1, 8, "SEQUENCE(0)"),
        (1, 9, "SUM(IF({1,0,2}>0,1,0))"),
        (1, 10, "SUM(SEQUENCE(,3))"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [
        (1, 134.0),
        (2, 11.0),
        (3, 10.0),
        (4, 90.0),
        (5, 6.0),
        (9, 2.0),
        (10, 6.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for column in [6, 7] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }
    assert_eq!(
        calculation.cell(cell_id(8)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Number
        )))
    );
}

#[test]
fn if_distinguishes_empty_arguments_from_omitted_and_empty_text_results() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "IF(TRUE,,1)"),
        (1, 2, "IF(FALSE,1,)"),
        (1, 3, "IF(FALSE,1)"),
        (1, 4, "IF(FALSE,1,\"\")"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 0.0, 0.0);
    assert_number(&calculation, 2, 0.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(3)),
        Some(&CalculationCellResult::Value(CellValue::Logical(false)))
    );
    assert_eq!(
        calculation.cell(cell_id(4)),
        Some(&CalculationCellResult::Value(
            CellValue::Text(String::new())
        ))
    );
}

#[test]
fn xlookup_supports_exact_vector_lookup_and_excel_resave_prefixes() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "1"),
        (2, 1, "2"),
        (3, 1, "3"),
        (4, 1, "2"),
        (1, 2, "\"one\""),
        (2, 2, "\"two\""),
        (3, 2, "\"three\""),
        (4, 2, "\"last\""),
        (1, 3, "_xludf.XLOOKUP(2,A1:A3,B1:B3)"),
        (1, 4, "XLOOKUP(4,A1:A3,B1:B3,\"missing\")"),
        (1, 5, "XLOOKUP(4,A1:A3,B1:B3)"),
        (1, 6, "XLOOKUP(2,A1:A4,B1:B4,,0,-1)"),
        (1, 7, "XLOOKUP(\"TWO\",B1:B3,A1:A3)"),
        (
            1,
            8,
            "IFERROR(__xludf.DUMMYFUNCTION(\"COMPUTED_VALUE\"),\"fallback\")",
        ),
        (1, 9, "__xludf.DUMMYFUNCTION(\"COMPUTED_VALUE\")"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [(3, "two"), (4, "missing"), (6, "last")] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            )))
        );
    }
    assert_eq!(
        calculation.cell(cell_id(5)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        )))
    );
    assert_number(&calculation, 7, 2.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(8)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "fallback".to_owned()
        )))
    );
    assert_eq!(
        calculation.cell(cell_id(9)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Name
        )))
    );
}

#[test]
fn map_binds_lambda_parameters_with_array_and_iteration_limits() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "SUMPRODUCT(MAP({1,2,3},LAMBDA(x,x*2)))"),
            (
                1,
                2,
                "_xlfn.SUMPRODUCT(_xlfn.MAP({1,2},_xlfn.LAMBDA(_xlpm.x,_xlpm.x+1)))",
            ),
            (1, 3, "SUMPRODUCT(MAP({1,2},{10,20},LAMBDA(x,y,x+y)))"),
            (1, 4, "SUMPRODUCT(MAP({1,2},LAMBDA(x,IF(x=1,10,x+20))))"),
            (1, 5, "SUMPRODUCT(MAP({1,2},LAMBDA(x,y,x+y)))"),
            (1, 6, "SUMPRODUCT(MAP({1,2},{10;20},LAMBDA(x,y,x+y)))"),
            (1, 7, "SUMPRODUCT(MAP({1,2},LAMBDA(x,x*Factor)))"),
        ],
        &[("x", "100"), ("Factor", "10")],
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [(1, 12.0), (2, 5.0), (3, 33.0), (4, 32.0), (7, 30.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for column in [5, 6] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
    }

    let limits = CalculationLimits::default()
        .with_max_function_iterations(2)
        .expect("nonzero MAP iteration limit");
    let limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "SUMPRODUCT(MAP({1,2,3},LAMBDA(x,x)))")]),
        CalculationOptions::default().with_limits(limits),
    );
    assert_issue(&limited, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn let_preserves_scalar_array_reference_and_lexical_scope_semantics() {
    let workbook = workbook_with_formulas_and_names(
        &[
            (1, 1, "1"),
            (1, 2, "2"),
            (1, 3, "LET(x,2,x*3)"),
            (1, 4, "LET(source,A1:B1,SUM(source))"),
            (1, 5, "SUM(LET(items,{1,2,3},items))"),
            (1, 6, "LET(x,1,LET(x,2,x)+x)"),
            (1, 7, "LET(Factor,3,Factor*2)"),
            (1, 8, "_xlfn.LET(_xlpm.Total,4,_xlpm.total+1)"),
            (1, 9, "LET(x,2,y,x+1,y*3)"),
            (1, 10, "ISREF(LET(source,A1:B1,source))"),
            (1, 11, "LET(x,y,y,2,x)"),
        ],
        &[("Factor", "10")],
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [
        (3, 6.0),
        (4, 3.0),
        (5, 6.0),
        (6, 3.0),
        (7, 6.0),
        (8, 5.0),
        (9, 9.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_eq!(
        calculation.cell(cell_id(10)),
        Some(&CalculationCellResult::Value(CellValue::Logical(true)))
    );
    assert_issue(&calculation, 11, CalculationIssueCode::UnsupportedName);
}

#[test]
fn let_validates_names_duplicates_and_arity_before_evaluation() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "LET(x,1)"),
        (1, 2, "LET(x,1,X,2,x)"),
        (1, 3, "LET(R1C1,1,R1C1)"),
        (1, 4, "LET(c,1,c)"),
        (1, 5, "LET(A1,1,A1)"),
        (1, 6, "LET(r,1,r)"),
        (1, 7, "LET(valid,1,valid)"),
        (1, 8, "LET(Δ,2,δ+1)"),
        (1, 9, r"LET(\rate,3,\RATE+1)"),
        (1, 10, "LET(x,MYSTERY())"),
        (1, 11, "LET(1,MYSTERY(),0)"),
        (1, 12, "LET(x,MYSTERY(),X,2,x)"),
        (1, 13, "LET(1,NoSuchName,0)"),
        (1, 14, "LET(R1C1,MYSTERY(),R1C1)"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    assert_eq!(
        scan_function_usage(&workbook)
            .entries()
            .iter()
            .map(|entry| entry.name())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["LET"]),
        "invalid LET arguments are unreachable and must not leak nested function usage",
    );

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=6 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "unexpected invalid LET result in column {column}",
        );
    }
    assert_number(&calculation, 7, 1.0, 0.0);
    assert_number(&calculation, 8, 3.0, 0.0);
    assert_number(&calculation, 9, 4.0, 0.0);
    for column in 10..=14 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "invalid LET must reject its call shape before inspecting column {column}",
        );
    }
}

#[test]
fn function_specific_argument_limits_are_enforced_before_evaluation() {
    let concat = format!("CONCAT({})", vec!["\"x\""; 254].join(","));
    let text_join = format!("TEXTJOIN(\"\",TRUE,{})", vec!["\"x\""; 253].join(","));
    let switch = format!(
        "SWITCH(1,{})",
        (1..=127)
            .flat_map(|value| [value.to_string(), value.to_string()])
            .collect::<Vec<_>>()
            .join(",")
    );
    let max_ifs = format!("MAXIFS({})", vec!["1"; 255].join(","));
    let min_ifs = format!("MINIFS({})", vec!["1"; 255].join(","));
    let formulas = [concat, text_join, switch, max_ifs, min_ifs];
    let cells = formulas
        .iter()
        .enumerate()
        .map(|(index, formula)| (1, index as u32 + 1, formula.as_str()))
        .collect::<Vec<_>>();
    let workbook = workbook_with_formulas(&cells);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=5 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "argument limit was not enforced for column {column}",
        );
    }
}

#[test]
fn invalid_intrinsic_arguments_are_unreachable_to_dependency_analysis() {
    let over_limit_sum = format!("SUM(C1,{})", vec!["1"; 255].join(","));
    let workbook = workbook_with_formulas(&[
        (1, 1, "COUNTBLANK(A1,A2)"),
        (1, 2, "ABS(B1,1)"),
        (1, 3, over_limit_sum.as_str()),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=3 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "invalid intrinsic arguments must be rejected before dependency analysis in column {column}",
        );
    }
}

#[test]
fn let_and_map_preserve_decimal_traces_without_overriding_arithmetic_mode() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "0.1+0.2-0.3"),
        (1, 2, "LET(x,0.1+0.2,x-0.3)"),
        (1, 3, "_xlfn.LET(_xlpm.x,0.1+0.2,_xlpm.x-0.3)"),
        (1, 4, "SUM({0.1,0.2}+{0.2,0.1})-0.6"),
        (1, 5, "SUM(LET(items,{0.1,0.2}+{0.2,0.1},items))-0.6"),
        (1, 6, "SUMPRODUCT(MAP({0.1},LAMBDA(x,x+0.2-0.3)))"),
        (
            1,
            7,
            "SUM(_xlfn.MAP({0.1},_xlfn.LAMBDA(_xlpm.x,_xlpm.x+0.2-0.3)))",
        ),
        (2, 1, "0.1+0.2"),
        (2, 2, "A2-0.3"),
        (2, 3, "LET(x,A2,x-0.3)"),
        (2, 4, "LET(source,A2:A2,source-0.3)"),
    ]);

    let excel = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&excel, 1, 0.0, 0.0);
    assert_equal_number_bits(&excel, 1, 2);
    assert_equal_number_bits(&excel, 1, 3);
    assert_equal_number_bits(&excel, 4, 5);
    assert_equal_number_bits(&excel, 1, 6);
    assert_equal_number_bits(&excel, 1, 7);
    assert_number_at(&excel, 2, 2, 0.0, 0.0);
    assert_equal_number_bits_at(&excel, 2, 2, 2, 3);
    assert_equal_number_bits_at(&excel, 2, 2, 2, 4);

    let ieee = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_arithmetic_semantics(ArithmeticSemantics::Ieee754),
    );
    let residue = 0.1_f64 + 0.2_f64 - 0.3_f64;
    assert_number(&ieee, 1, residue, f64::EPSILON);
    assert_equal_number_bits(&ieee, 1, 2);
    assert_equal_number_bits(&ieee, 1, 3);
    assert_equal_number_bits(&ieee, 4, 5);
    assert_equal_number_bits(&ieee, 1, 6);
    assert_equal_number_bits(&ieee, 1, 7);
    assert_number_at(&ieee, 2, 2, residue, f64::EPSILON);
    assert_equal_number_bits_at(&ieee, 2, 2, 2, 3);
    assert_equal_number_bits_at(&ieee, 2, 2, 2, 4);
}

#[test]
fn let_and_lambda_limits_have_distinct_units_and_stable_details() {
    assert_eq!(CalculationLimits::default().max_let_bindings(), 126);
    assert_eq!(CalculationLimits::default().max_lambda_depth(), 256);
    assert_eq!(
        CalculationLimits::default().max_lambda_invocations(),
        1_000_000
    );

    let let_limits = CalculationLimits::default()
        .with_max_let_bindings(1)
        .expect("positive LET binding limit");
    let let_limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "LET(first,1,second,2,first+second)")]),
        CalculationOptions::default().with_limits(let_limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = let_limited.cell(cell_id(1)) else {
        panic!("LET binding limit must withhold the result");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_let_bindings"));

    let invocation_limits = CalculationLimits::default()
        .with_max_lambda_invocations(2)
        .expect("positive lambda invocation limit");
    let invocation_limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "SUMPRODUCT(MAP({1,2,3},LAMBDA(item,item)))")]),
        CalculationOptions::default().with_limits(invocation_limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = invocation_limited.cell(cell_id(1))
    else {
        panic!("lambda invocation limit must withhold the result");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_lambda_invocations"));

    let depth_limits = CalculationLimits::default()
        .with_max_lambda_depth(1)
        .expect("positive lambda depth limit");
    let depth_limited = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "SUMPRODUCT(MAP({1},LAMBDA(x,SUMPRODUCT(MAP({1},LAMBDA(y,x+y))))))",
        )]),
        CalculationOptions::default().with_limits(depth_limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) = depth_limited.cell(cell_id(1)) else {
        panic!("lambda depth limit must withhold the result");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_lambda_depth"));

    let let_only = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "LET(x,1,LET(y,2,x+y))")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_lambda_invocations(1)
                .expect("positive lambda invocation limit"),
        ),
    );
    assert_number(&let_only, 1, 3.0, 0.0);

    let lazy_if = calculate_workbook(
        &workbook_with_formulas(&[(
            1,
            1,
            "SUM(IF({TRUE},MAP({1},LAMBDA(x,x)),MAP({2},LAMBDA(x,x))))",
        )]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_lambda_invocations(1)
                .expect("positive lambda invocation limit"),
        ),
    );
    assert_number(&lazy_if, 1, 1.0, 0.0);
}

#[test]
fn let_binding_limit_covers_the_excel_boundary() {
    fn formula(binding_count: usize) -> String {
        let mut formula = String::from("LET(");
        for index in 1..=binding_count {
            if index > 1 {
                formula.push(',');
            }
            formula.push_str(&format!("_n{index},{index}"));
        }
        formula.push_str(&format!(",_n{binding_count})"));
        formula
    }

    let formulas = [formula(125), formula(126), formula(127)];
    let workbook = workbook_with_formulas(&[
        (1, 1, formulas[0].as_str()),
        (1, 2, formulas[1].as_str()),
        (1, 3, formulas[2].as_str()),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 125.0, 0.0);
    assert_number(&calculation, 2, 126.0, 0.0);
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(3)) else {
        panic!("127 LET bindings must exceed the default limit");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some("max_let_bindings"));
}

#[test]
fn single_cell_legacy_array_metadata_uses_array_root_evaluation() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (address, formula, end) in [
        ((1, 1), "SUM({1,2,3})", (1, 1)),
        ((1, 2), "SUM({4,5})", (2, 2)),
    ] {
        let address = CellAddress::from_indices(address.0, address.1).expect("formula address");
        let end = CellAddress::from_indices(end.0, end.1).expect("array end address");
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx(formula).expect("valid formula text"),
            SavedResult::Missing,
            FormulaMetadata::Array {
                range: CellRange::new(address, end).expect("valid array range"),
                always_calculate: false,
            },
        );
        sheet
            .insert_cell(address, CellContent::Formula(formula))
            .expect("unique formula address");
    }
    let provider =
        ProviderIdentity::new("calculation-test", "1").expect("valid test provider identity");
    let workbook = WorkbookSnapshot::new_with_metadata(
        vec![sheet],
        Vec::new(),
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(provider, None),
    )
    .expect("valid calculation test workbook");

    let capability = scan_formula_capabilities(&workbook);
    assert_eq!(capability.supported_count(), 2);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&calculation, 1, 6.0, 0.0);
    assert_number(&calculation, 2, 9.0, 0.0);
}

/// A cell an array formula materializes over no longer holds the literal underneath it, and the
/// near-zero policy must read its exact decimal from the same place it reads its value.
///
/// Here `B1` and `C1` carry literals that cancel exactly while the array puts `2` and `3` on top of
/// them. Taking the decimals from the literals proves a cancellation that the summed values never
/// performed, and `=SUM(B1:C1)` collapses from `5` to `0`.
#[test]
fn literals_under_an_array_formula_do_not_supply_the_near_zero_decision() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let anchor = CellAddress::from_a1("A1").expect("array anchor");
    let end = CellAddress::from_a1("C1").expect("array end");
    sheet
        .insert_cell(
            anchor,
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx("SEQUENCE(1,3)").expect("valid array formula text"),
                SavedResult::Missing,
                FormulaMetadata::Array {
                    range: CellRange::new(anchor, end).expect("valid array range"),
                    always_calculate: false,
                },
            )),
        )
        .expect("unique array anchor");
    for (address, literal) in [("B1", 0.1), ("C1", -0.1)] {
        sheet
            .insert_cell(
                CellAddress::from_a1(address).expect("array follower address"),
                CellContent::Literal(CellValue::number(literal).expect("finite stale literal")),
            )
            .expect("unique array follower");
    }
    sheet
        .insert_cell(
            CellAddress::from_a1("A2").expect("dependent formula address"),
            CellContent::Formula(FormulaCell::new(
                FormulaDialect::ExcelA1,
                FormulaText::from_xlsx("SUM(B1:C1)").expect("valid dependent formula text"),
                SavedResult::Missing,
                FormulaMetadata::Normal,
            )),
        )
        .expect("unique dependent formula");
    let workbook = WorkbookSnapshot::new_with_metadata(
        vec![sheet],
        Vec::new(),
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("calculation-test", "1").expect("valid test provider identity"),
            None,
        ),
    )
    .expect("valid calculation test workbook");

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number_at(&calculation, 2, 1, 5.0, 0.0);
}

#[test]
fn multi_cell_legacy_array_results_feed_dependent_formulas() {
    let sheet_id = SheetId::new(1).expect("valid sheet ID");
    let mut sheet = Sheet::new(
        sheet_id,
        SheetName::new("Sheet1").expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    let anchor = CellAddress::from_a1("A1").expect("array anchor");
    let end = CellAddress::from_a1("B2").expect("array end");
    let array_formula = FormulaCell::new(
        FormulaDialect::ExcelA1,
        FormulaText::from_xlsx("SEQUENCE(2,2)").expect("valid formula text"),
        SavedResult::Missing,
        FormulaMetadata::Array {
            range: CellRange::new(anchor, end).expect("valid array range"),
            always_calculate: false,
        },
    );
    sheet
        .insert_cell(anchor, CellContent::Formula(array_formula))
        .expect("unique array anchor");
    for address in ["B1", "A2", "B2"] {
        sheet
            .insert_cell(
                CellAddress::from_a1(address).expect("array follower address"),
                CellContent::Literal(
                    CellValue::number(99.0).expect("finite stale saved array result"),
                ),
            )
            .expect("unique array follower");
    }
    for (address, text) in [("C1", "SUM(A1:B2)"), ("C2", "B2")] {
        let address = CellAddress::from_a1(address).expect("dependent formula address");
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx(text).expect("valid dependent formula text"),
            SavedResult::Missing,
            FormulaMetadata::Normal,
        );
        sheet
            .insert_cell(address, CellContent::Formula(formula))
            .expect("unique dependent formula");
    }
    let provider =
        ProviderIdentity::new("calculation-test", "1").expect("valid test provider identity");
    let workbook = WorkbookSnapshot::new_with_metadata(
        vec![sheet],
        Vec::new(),
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(provider, None),
    )
    .expect("valid calculation test workbook");

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number_at(&calculation, 1, 1, 1.0, 0.0);
    assert_number_at(&calculation, 1, 3, 10.0, 0.0);
    assert_number_at(&calculation, 2, 3, 4.0, 0.0);
    assert_eq!(
        calculation.cell(calculation_cell_id(2, 2)),
        None,
        "literal array followers remain internal materialized results"
    );
    for (address, expected) in [("A1", 1.0), ("B1", 2.0), ("A2", 3.0), ("B2", 4.0)] {
        let address = CellAddress::from_a1(address).expect("materialized address");
        let id = CalculationCellId::new(sheet_id, address);
        let materialized = calculation
            .materialized_cell(id)
            .expect("legacy-array cell must be materialized");
        assert_eq!(
            materialized.origin(),
            MaterializedResultOrigin::LegacyArray {
                anchor: CalculationCellId::new(sheet_id, anchor),
                range: CellRange::new(anchor, end).expect("valid materialized range"),
            }
        );
        assert_eq!(
            materialized.result(),
            &CalculationCellResult::Value(
                CellValue::number(expected).expect("finite materialized value")
            )
        );
    }
}

fn materialized_result<'a>(
    calculation: &'a cellrune::CalculationSnapshot,
    sheet_id: SheetId,
    address: &str,
) -> Option<&'a CalculationCellResult> {
    let id = CalculationCellId::new(
        sheet_id,
        CellAddress::from_a1(address).expect("valid materialized address"),
    );
    calculation.materialized_cell(id).map(|cell| cell.result())
}

fn assert_materialized_number(
    calculation: &cellrune::CalculationSnapshot,
    sheet_id: SheetId,
    address: &str,
    expected: f64,
) {
    assert_eq!(
        materialized_result(calculation, sheet_id, address),
        Some(&CalculationCellResult::Value(
            CellValue::number(expected).expect("finite materialized expectation")
        )),
        "unexpected materialized value at {address}",
    );
}

fn assert_positive_zero(calculation: &cellrune::CalculationSnapshot, column: u32) {
    let Some(CalculationCellResult::Value(CellValue::Number(number))) =
        calculation.cell(cell_id(column))
    else {
        panic!("expected a numeric calculation result in column {column}");
    };
    assert_eq!(number.get(), 0.0, "unexpected magnitude in column {column}");
    assert!(
        !number.get().is_sign_negative(),
        "column {column} produced negative zero, which Excel reports as 0",
    );
}

fn assert_equal_number_bits(
    calculation: &cellrune::CalculationSnapshot,
    left_column: u32,
    right_column: u32,
) {
    let value = |column: u32| {
        let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
            calculation.cell(cell_id(column))
        else {
            panic!(
                "expected numeric calculation result in column {column}, got {:?}",
                calculation.cell(cell_id(column))
            );
        };
        actual.get()
    };
    assert_eq!(
        value(left_column).to_bits(),
        value(right_column).to_bits(),
        "columns {left_column} and {right_column} changed arithmetic semantics",
    );
}

fn assert_equal_number_bits_at(
    calculation: &cellrune::CalculationSnapshot,
    left_row: u32,
    left_column: u32,
    right_row: u32,
    right_column: u32,
) {
    let value = |row: u32, column: u32| {
        let id = calculation_cell_id(row, column);
        let Some(CalculationCellResult::Value(CellValue::Number(actual))) = calculation.cell(id)
        else {
            panic!(
                "expected numeric calculation result at row {row}, column {column}, got {:?}",
                calculation.cell(id)
            );
        };
        actual.get()
    };
    assert_eq!(
        value(left_row, left_column).to_bits(),
        value(right_row, right_column).to_bits(),
        "cells ({left_row},{left_column}) and ({right_row},{right_column}) changed arithmetic semantics",
    );
}

fn assert_number_at(
    calculation: &cellrune::CalculationSnapshot,
    row: u32,
    column: u32,
    expected: f64,
    tolerance: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.cell(calculation_cell_id(row, column))
    else {
        panic!("expected numeric calculation result at row {row}, column {column}");
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected result at row {row}, column {column}: expected {expected}, got {}",
        actual.get(),
    );
}

fn assert_capability_issue(
    report: &cellrune::FormulaCapabilityReport,
    column: u32,
    expected: CalculationIssueCode,
    detail: Option<&str>,
) {
    let entry = report
        .entries()
        .iter()
        .find(|entry| entry.cell() == cell_id(column))
        .expect("formula capability entry");
    let FormulaCapability::Unsupported(issues) = entry.capability() else {
        panic!("formula capability must be unsupported in column {column}");
    };
    assert!(
        issues
            .iter()
            .any(|issue| issue.code() == expected && issue.detail() == detail),
        "missing issue {expected:?} in column {column}: {issues:?}",
    );
}

fn assert_capability_issue_code(
    report: &cellrune::FormulaCapabilityReport,
    column: u32,
    expected: CalculationIssueCode,
) {
    let entry = report
        .entries()
        .iter()
        .find(|entry| entry.cell() == cell_id(column))
        .expect("formula capability entry");
    let FormulaCapability::Unsupported(issues) = entry.capability() else {
        panic!("formula capability must be unsupported in column {column}");
    };
    assert!(
        issues.iter().any(|issue| issue.code() == expected),
        "missing issue {expected:?} in column {column}: {issues:?}",
    );
}

fn assert_capability_issue_count(
    report: &cellrune::FormulaCapabilityReport,
    column: u32,
    expected: usize,
) {
    let entry = report
        .entries()
        .iter()
        .find(|entry| entry.cell() == cell_id(column))
        .expect("formula capability entry");
    let FormulaCapability::Unsupported(issues) = entry.capability() else {
        panic!("formula capability must be unsupported in column {column}");
    };
    assert_eq!(
        issues.len(),
        expected,
        "unexpected issue count in column {column}: {issues:?}",
    );
}

fn calculation_cell_id(row: u32, column: u32) -> CalculationCellId {
    CalculationCellId::new(
        SheetId::new(1).expect("valid sheet ID"),
        CellAddress::from_indices(row, column).expect("valid test address"),
    )
}

fn formula_sheet(id: u32, name: &str, formulas: &[(u32, u32, &str)]) -> Sheet {
    let mut sheet = Sheet::new(
        SheetId::new(id).expect("valid sheet ID"),
        SheetName::new(name).expect("valid sheet name"),
        SheetVisibility::Visible,
    );
    for (row, column, formula) in formulas {
        let address = CellAddress::from_indices(*row, *column).expect("valid formula address");
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx(*formula).expect("valid formula text"),
            SavedResult::Missing,
            FormulaMetadata::Normal,
        );
        sheet
            .insert_cell(address, CellContent::Formula(formula))
            .expect("unique formula address");
    }
    sheet
}

fn three_sheet_workbook(
    sheet1_formulas: &[(u32, u32, &str)],
    names: &[(&str, &str)],
) -> WorkbookSnapshot {
    let mut first = formula_sheet(1, "Sheet1", sheet1_formulas);
    insert_number(&mut first, "Z1", 1.0);
    insert_number(&mut first, "Z2", 2.0);
    workbook_with_sheets_and_names(
        vec![
            first,
            numeric_sheet(2, "Sheet2", 10.0, SheetVisibility::Hidden),
            numeric_sheet(3, "Sheet3", 100.0, SheetVisibility::Visible),
        ],
        names,
    )
}

fn numeric_sheet(id: u32, name: &str, scale: f64, visibility: SheetVisibility) -> Sheet {
    let mut sheet = Sheet::new(
        SheetId::new(id).expect("valid sheet ID"),
        SheetName::new(name).expect("valid sheet name"),
        visibility,
    );
    for (address, value) in [
        ("B1", scale),
        ("B2", scale * 2.0),
        ("Z1", scale),
        ("Z2", scale * 2.0),
    ] {
        insert_number(&mut sheet, address, value);
    }
    sheet
}

fn insert_number(sheet: &mut Sheet, address: &str, value: f64) {
    sheet
        .insert_cell(
            CellAddress::from_a1(address).expect("valid literal address"),
            CellContent::Literal(CellValue::number(value).expect("finite literal")),
        )
        .expect("unique literal address");
}

fn workbook_with_formulas_and_names(
    formulas: &[(u32, u32, &str)],
    names: &[(&str, &str)],
) -> WorkbookSnapshot {
    workbook_with_sheets_and_names(vec![formula_sheet(1, "Sheet1", formulas)], names)
}

fn workbook_with_sheets_and_names(sheets: Vec<Sheet>, names: &[(&str, &str)]) -> WorkbookSnapshot {
    let provider =
        ProviderIdentity::new("calculation-test", "1").expect("valid test provider identity");
    let defined_names = names
        .iter()
        .map(|(name, formula)| {
            DefinedName::new(
                *name,
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx(*formula).expect("valid defined name formula"),
                false,
            )
            .expect("valid defined name")
        })
        .collect();
    WorkbookSnapshot::new_with_metadata(
        sheets,
        defined_names,
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(provider, None),
    )
    .expect("valid calculation test workbook")
}
