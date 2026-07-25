use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationIssueCode,
    CalculationLimits, CalculationOptions, CalculationOptionsError, CellAddress, CellContent,
    CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope, ExcelError, FiniteNumber,
    FormulaCapability, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
    MaterializedResultOrigin, Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName,
    SheetVisibility, WorkbookDraft, WorkbookSnapshot, WorkbookSource, calculate_workbook,
    scan_formula_capabilities, scan_formula_capabilities_with_options, scan_function_usage,
    supported_function_catalog,
};

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
fn unsupported_reference_and_lambda_surfaces_are_reported_explicitly() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Table1[Amount])"),
        (1, 2, "A1#"),
        (1, 3, "LAMBDA(x,x+1)"),
        (1, 4, "SUMPRODUCT(MAP({1,2},LAMBDA(x,x+1)))"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    assert_capability_issue_code(&report, 1, CalculationIssueCode::ParseError);
    assert_capability_issue_code(&report, 2, CalculationIssueCode::ParseError);
    assert_capability_issue(
        &report,
        3,
        CalculationIssueCode::UnsupportedFunction,
        Some("LAMBDA"),
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
    assert_issue(&calculation, 1, CalculationIssueCode::ParseError);
    assert_issue(&calculation, 2, CalculationIssueCode::ParseError);
    assert_issue(&calculation, 3, CalculationIssueCode::UnsupportedFunction);
}

#[test]
fn three_d_sheet_range_references_are_reported_explicitly() {
    let workbook = three_sheet_workbook(
        &[
            (1, 1, "SUM(Sheet1:Sheet3!B1)"),
            (1, 2, "SUM('Sheet1:Sheet3'!B1)"),
            (1, 3, "SUM(Sheet1:'Sheet3'!B1)"),
            (1, 4, "IFERROR(SUM('Sheet1:Sheet3'!B1),42)"),
            (1, 5, "'Sheet2'!B1+1"),
        ],
        // A defined name that shadows the start sheet must not turn the 3-D
        // reference into an ordinary range over the named rect.
        &[("Sheet1", "Sheet3!B1:B2")],
    );
    let report = scan_formula_capabilities(&workbook);
    for column in 1..=4 {
        assert_capability_issue(
            &report,
            column,
            CalculationIssueCode::UnsupportedSheetRange,
            Some("Sheet1:Sheet3"),
        );
    }

    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=4 {
        assert_issue(
            &calculation,
            column,
            CalculationIssueCode::UnsupportedSheetRange,
        );
    }
    assert_number(&calculation, 5, 11.0, 0.0);
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
    // `[1]Sheet1!A1` never reaches the parser because the lexer has no bracket token, but Excel
    // also stores the quoted spelling, which arrives as one sheet-name token. Resolving it as an
    // ordinary missing sheet would yield `#REF!` — a spreadsheet error value that `IFERROR` is
    // allowed to hide — and would report the formula as Supported.
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
fn parse_error_details_locate_lex_failures_by_character_and_parse_failures_by_token() {
    // A lex failure happens before any token exists, so it can only be located by character
    // offset. Reporting it as a token index mislabels the position.
    let workbook = workbook_with_formulas(&[
        (1, 1, "SUM(Table1[Amount])"),
        (1, 2, "\"unterminated"),
        (1, 3, "1+"),
        (1, 4, "SUM(1))"),
    ]);
    let report = scan_formula_capabilities(&workbook);

    assert_capability_issue(
        &report,
        1,
        CalculationIssueCode::ParseError,
        Some("character 10: unexpected character in formula"),
    );
    assert_capability_issue(
        &report,
        2,
        CalculationIssueCode::ParseError,
        Some("character 0: unterminated string literal"),
    );
    assert_capability_issue(
        &report,
        3,
        CalculationIssueCode::ParseError,
        Some("token 2: unexpected end of formula"),
    );
    assert_capability_issue(
        &report,
        4,
        CalculationIssueCode::ParseError,
        Some("token 4: unexpected token"),
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
        278
    );
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
fn modern_array_function_work_is_bounded_before_materialization() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "CHOOSECOLS({1;2},1,1,1)"),
        (1, 2, "TAKE({1,2,3;4,5,6},2,3)"),
        (1, 3, "HSTACK({1;2;3},{4})"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(5)
        .expect("positive iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
    for column in 1..=3 {
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
fn calculation_limits_reject_zero_values() {
    assert_eq!(
        CalculationLimits::default().with_max_formula_tokens(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_formula_tokens",
        })
    );
    assert_eq!(
        CalculationLimits::default().with_max_dependency_edges(0),
        Err(CalculationOptionsError::ZeroLimit {
            name: "max_dependency_edges",
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
fn conditional_aggregates_use_excel_range_rules_and_clamp_whole_columns() {
    let workbook = workbook_with_formulas(&[
        (2, 1, "1"),
        (3, 1, "2"),
        (4, 1, "3"),
        (2, 2, "10"),
        (3, 2, "20"),
        (4, 2, "30"),
        (1, 3, "SUMIF(A2:A4,\">1\",B2)"),
        (1, 4, "AVERAGEIF(A2:A4,\">1\",B2)"),
        (1, 5, "SUMIFS(B2:B4,A2:A4,\">1\")"),
        (1, 6, "SUMIFS(B2:B3,A2:A4,\">1\")"),
        (1, 7, "MODE.SNGL({1,1,2,2})"),
        (1, 8, "VLOOKUP(1,A:B,2,FALSE)"),
        (1, 9, "SUMIFS(B:B,A:A,1)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero function iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    for (column, expected) in [
        (3, 50.0),
        (4, 25.0),
        (5, 50.0),
        (7, 1.0),
        (8, 10.0),
        (9, 10.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_eq!(
        calculation.cell(cell_id(6)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
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

fn assert_issue(
    calculation: &cellrune::CalculationSnapshot,
    column: u32,
    expected: CalculationIssueCode,
) {
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(column)) else {
        panic!("expected unavailable calculation result in column {column}");
    };
    assert_eq!(issue.code(), expected);
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

fn assert_number(
    calculation: &cellrune::CalculationSnapshot,
    column: u32,
    expected: f64,
    tolerance: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.cell(cell_id(column))
    else {
        panic!(
            "expected numeric calculation result in column {column}, got {:?}",
            calculation.cell(cell_id(column))
        );
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected result in column {column}: expected {expected}, got {}",
        actual.get(),
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

fn cell_id(column: u32) -> CalculationCellId {
    calculation_cell_id(1, column)
}

fn calculation_cell_id(row: u32, column: u32) -> CalculationCellId {
    CalculationCellId::new(
        SheetId::new(1).expect("valid sheet ID"),
        CellAddress::from_indices(row, column).expect("valid test address"),
    )
}

fn workbook_with_formulas(formulas: &[(u32, u32, &str)]) -> WorkbookSnapshot {
    workbook_with_formulas_and_names(formulas, &[])
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
    workbook_with_sheets_and_names(
        vec![
            formula_sheet(1, "Sheet1", sheet1_formulas),
            formula_sheet(2, "Sheet2", &[(1, 2, "10"), (2, 2, "20")]),
            formula_sheet(3, "Sheet3", &[(1, 2, "100"), (2, 2, "200")]),
        ],
        names,
    )
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
