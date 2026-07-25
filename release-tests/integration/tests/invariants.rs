use cellrune::{
    CalculationHints, CellAddress, CellContent, CellRange, CellValue, Column, DateSystem,
    DefinedName, DefinedNameScope, Diagnostic, DiagnosticCode, DiagnosticSeverity,
    EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, ExcelError, FormulaCell, FormulaDialect, FormulaMetadata,
    FormulaText, InputHash, Provenance, ProviderIdentity, Row, SavedResult, SavedResultIssue,
    Sheet, SheetId, SheetName, SheetVisibility, SourceId, SourceLocation, ValidationError,
    WorkbookSnapshot, WorkbookSource, WorkbookSourceKind,
};

#[test]
fn row_and_column_boundaries_are_validated() {
    assert_eq!(Row::new(1).expect("minimum row").get(), 1);
    assert_eq!(
        Row::new(EXCEL_MAX_ROWS).expect("maximum row").get(),
        EXCEL_MAX_ROWS
    );
    assert_eq!(Column::new(1).expect("minimum column").get(), 1);
    assert_eq!(
        Column::new(EXCEL_MAX_COLUMNS)
            .expect("maximum column")
            .get(),
        EXCEL_MAX_COLUMNS
    );
    assert_eq!(
        Row::new(0),
        Err(ValidationError::RowOutOfRange { value: 0 })
    );
    assert_eq!(
        Row::new(EXCEL_MAX_ROWS + 1),
        Err(ValidationError::RowOutOfRange {
            value: EXCEL_MAX_ROWS + 1
        })
    );
    assert_eq!(
        Column::new(0),
        Err(ValidationError::ColumnOutOfRange { value: 0 })
    );
    assert_eq!(
        Column::new(EXCEL_MAX_COLUMNS + 1),
        Err(ValidationError::ColumnOutOfRange {
            value: EXCEL_MAX_COLUMNS + 1
        })
    );
}

#[test]
fn addresses_and_ranges_preserve_excel_bounds() {
    let minimum = CellAddress::from_indices(1, 1).expect("A1");
    let maximum = CellAddress::from_indices(EXCEL_MAX_ROWS, EXCEL_MAX_COLUMNS).expect("XFD max");
    assert_eq!(minimum.to_string(), "A1");
    assert_eq!(maximum.to_string(), "XFD1048576");
    assert_eq!(CellAddress::from_a1("a1"), Ok(minimum));
    assert_eq!("XFD1048576".parse::<CellAddress>(), Ok(maximum));
    assert_eq!(
        CellAddress::from_a1(""),
        Err(ValidationError::CellAddressInvalid)
    );
    assert_eq!(
        CellAddress::from_a1("$A$1"),
        Err(ValidationError::CellAddressInvalid)
    );
    assert_eq!(
        CellAddress::from_a1("A1x"),
        Err(ValidationError::CellAddressInvalid)
    );
    assert_eq!(
        CellAddress::from_a1("XFE1"),
        Err(ValidationError::ColumnOutOfRange {
            value: EXCEL_MAX_COLUMNS + 1,
        })
    );
    assert_eq!(
        CellAddress::from_a1("A1048577"),
        Err(ValidationError::RowOutOfRange {
            value: EXCEL_MAX_ROWS + 1,
        })
    );

    let range = CellRange::new(minimum, CellAddress::from_indices(3, 2).expect("B3"))
        .expect("ordered range");
    assert_eq!(range.height(), 3);
    assert_eq!(range.width(), 2);
    assert!(range.contains(CellAddress::from_indices(2, 2).expect("B2")));
    assert_eq!(
        CellRange::new(maximum, minimum),
        Err(ValidationError::RangeStartAfterEnd)
    );
}

#[test]
fn sheet_ids_and_names_reject_invalid_external_input() {
    assert_eq!(SheetId::new(0), Err(ValidationError::SheetIdZero));
    assert_eq!(SheetName::new(""), Err(ValidationError::SheetNameEmpty));
    assert_eq!(
        SheetName::new("a".repeat(32)),
        Err(ValidationError::SheetNameTooLong { utf16_len: 32 })
    );
    assert_eq!(
        SheetName::new("😀".repeat(16)),
        Err(ValidationError::SheetNameTooLong { utf16_len: 32 })
    );
    assert_eq!(
        SheetName::new("Bad/Name"),
        Err(ValidationError::SheetNameInvalidCharacter { character: '/' })
    );
    assert_eq!(
        SheetName::new("'Quoted"),
        Err(ValidationError::SheetNameApostropheBoundary)
    );
    assert_eq!(
        SheetName::new("Quoted'"),
        Err(ValidationError::SheetNameApostropheBoundary)
    );
    assert_eq!(
        SheetName::new("O'Brien")
            .expect("internal apostrophe")
            .as_str(),
        "O'Brien"
    );
}

#[test]
fn numbers_reject_nan_and_infinity() {
    assert_eq!(
        CellValue::number(f64::NAN),
        Err(ValidationError::NonFiniteNumber)
    );
    assert_eq!(
        CellValue::number(f64::INFINITY),
        Err(ValidationError::NonFiniteNumber)
    );
    assert_eq!(
        CellValue::number(f64::NEG_INFINITY),
        Err(ValidationError::NonFiniteNumber)
    );
    let CellValue::Number(number) = CellValue::number(42.5).expect("finite number") else {
        panic!("numeric constructor must return a numeric value");
    };
    assert_eq!(number.get(), 42.5);
}

#[test]
fn formula_boundaries_normalize_only_the_leading_equals_sign() {
    let stored = FormulaText::from_xlsx("SUM(A1:A3)").expect("stored formula");
    let entered = FormulaText::from_user_input("=SUM(A1:A3)").expect("user formula");
    assert_eq!(stored, entered);
    assert_eq!(stored.as_str(), "SUM(A1:A3)");

    assert_eq!(
        FormulaText::from_xlsx("=A1"),
        Err(ValidationError::XlsxFormulaHasLeadingEquals)
    );
    assert_eq!(
        FormulaText::from_user_input("A1"),
        Err(ValidationError::UserFormulaMissingLeadingEquals)
    );
    assert_eq!(
        FormulaText::from_user_input("=   "),
        Err(ValidationError::FormulaEmpty)
    );
}

#[test]
fn formula_saved_result_states_remain_distinct() {
    let text = FormulaText::from_xlsx("1+1").expect("formula");
    let missing = FormulaCell::new(
        FormulaDialect::ExcelA1,
        text.clone(),
        SavedResult::Missing,
        FormulaMetadata::Normal,
    );
    assert!(matches!(missing.saved_result(), SavedResult::Missing));

    let present_blank = FormulaCell::new(
        FormulaDialect::ExcelA1,
        text.clone(),
        SavedResult::Present(CellValue::Blank),
        FormulaMetadata::Normal,
    );
    assert!(matches!(
        present_blank.saved_result(),
        SavedResult::Present(CellValue::Blank)
    ));

    let code = DiagnosticCode::new("xlsx.invalid_saved_result").expect("stable code");
    let issue = SavedResultIssue::new(code.clone(), Some("not-a-number".to_owned()));
    let invalid = FormulaCell::new(
        FormulaDialect::ExcelA1,
        text,
        SavedResult::Invalid(issue),
        FormulaMetadata::Normal,
    );
    let SavedResult::Invalid(issue) = invalid.saved_result() else {
        panic!("invalid saved result must remain invalid");
    };
    assert_eq!(issue.code(), &code);
    assert_eq!(issue.raw_value(), Some("not-a-number"));
}

#[test]
fn sparse_cells_are_unique_and_iterate_in_row_major_order() {
    let mut sheet = sheet(1, "Data");
    for (row, column) in [(3, 3), (1, 2), (1, 1)] {
        sheet
            .insert_cell(
                CellAddress::from_indices(row, column).expect("valid address"),
                CellContent::Literal(CellValue::Text(format!("{row}:{column}"))),
            )
            .expect("unique cell");
    }

    let addresses: Vec<String> = sheet
        .cells()
        .map(|cell| cell.address().to_string())
        .collect();
    assert_eq!(addresses, ["A1", "B1", "C3"]);
    assert_eq!(sheet.len(), 3);

    let used = sheet.used_range().expect("used range");
    assert_eq!(used.start().to_string(), "A1");
    assert_eq!(used.end().to_string(), "C3");

    let duplicate = CellAddress::from_indices(1, 1).expect("A1");
    assert_eq!(
        sheet.insert_cell(duplicate, CellContent::Literal(CellValue::Blank)),
        Err(ValidationError::DuplicateCell { row: 1, column: 1 })
    );
}

#[test]
fn workbook_enforces_sheet_identity_and_case_insensitive_name_uniqueness() {
    let duplicate_ids = WorkbookSnapshot::new(
        vec![sheet(1, "First"), sheet(1, "Second")],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        provenance(),
    );
    assert!(matches!(
        duplicate_ids,
        Err(ValidationError::DuplicateSheetId { value: 1 })
    ));

    let duplicate_names = WorkbookSnapshot::new(
        vec![sheet(1, "Data"), sheet(2, "dAtA")],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        provenance(),
    );
    assert!(matches!(
        duplicate_names,
        Err(ValidationError::DuplicateSheetName { .. })
    ));

    let duplicate_unicode_names = WorkbookSnapshot::new(
        vec![sheet(1, "Ä"), sheet(2, "ä")],
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        provenance(),
    );
    assert!(matches!(
        duplicate_unicode_names,
        Err(ValidationError::DuplicateSheetName { .. })
    ));
}

#[test]
fn workbook_validates_defined_name_scope_and_case_insensitive_uniqueness() {
    let formula = FormulaText::from_xlsx("Data!$A$1").expect("name formula");
    let first = DefinedName::new(
        "TaxRate",
        DefinedNameScope::Workbook,
        formula.clone(),
        false,
    )
    .expect("defined name");
    let duplicate = DefinedName::new(
        "taxrate",
        DefinedNameScope::Workbook,
        formula.clone(),
        false,
    )
    .expect("case variant");
    let snapshot = WorkbookSnapshot::new_with_metadata(
        vec![sheet(1, "Data")],
        vec![first, duplicate],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        provenance(),
    );
    assert!(matches!(
        snapshot,
        Err(ValidationError::DuplicateDefinedName { .. })
    ));

    let unknown_scope = DefinedName::new(
        "LocalValue",
        DefinedNameScope::Sheet(SheetId::new(2).expect("sheet ID")),
        formula,
        true,
    )
    .expect("sheet-local name");
    let snapshot = WorkbookSnapshot::new_with_metadata(
        vec![sheet(1, "Data")],
        vec![unknown_scope],
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        provenance(),
    );
    assert_eq!(
        snapshot.expect_err("unknown scope"),
        ValidationError::DefinedNameUnknownSheet { sheet_id: 2 }
    );
}

#[test]
fn workbook_preserves_sheet_order_and_original_names() {
    let snapshot = WorkbookSnapshot::new(
        vec![sheet(2, "Summary"), sheet(7, "RawData"), sheet(8, "Ä")],
        DateSystem::Excel1904,
        CalculationHints::default(),
        WorkbookSource::new(WorkbookSourceKind::Bytes, Some(128)),
        provenance(),
    )
    .expect("valid workbook");

    let names: Vec<&str> = snapshot
        .sheets()
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect();
    assert_eq!(names, ["Summary", "RawData", "Ä"]);
    assert_eq!(
        snapshot
            .sheet_by_name("rawdata")
            .expect("case-insensitive lookup")
            .name()
            .as_str(),
        "RawData"
    );
    assert_eq!(
        snapshot
            .sheet_by_id(SheetId::new(2).expect("sheet ID"))
            .expect("ID lookup")
            .name()
            .as_str(),
        "Summary"
    );
    assert_eq!(
        snapshot
            .sheet_by_name("ä")
            .expect("Unicode case-insensitive lookup")
            .name()
            .as_str(),
        "Ä"
    );
    assert_eq!(snapshot.date_system(), DateSystem::Excel1904);
    assert_eq!(snapshot.source().byte_length(), Some(128));
}

#[test]
fn diagnostic_and_provenance_inputs_are_validated() {
    assert_eq!(SourceId::new(""), Err(ValidationError::SourceIdEmpty));
    for invalid in ["core", "Core.invalid", "core.", "core.invalid-code"] {
        assert_eq!(
            DiagnosticCode::new(invalid),
            Err(ValidationError::DiagnosticCodeInvalid)
        );
    }

    let source = SourceId::new("xl/worksheets/sheet1.xml").expect("source ID");
    let location = SourceLocation::cell(
        source,
        SheetId::new(1).expect("sheet ID"),
        CellAddress::from_indices(2, 3).expect("C2"),
    )
    .with_byte_offset(42);
    let diagnostic = Diagnostic::new(
        DiagnosticCode::new("xlsx.unsupported_feature").expect("diagnostic code"),
        DiagnosticSeverity::Warning,
        "Feature metadata was preserved but is not supported",
        Some(location),
    )
    .expect("diagnostic");
    assert_eq!(
        diagnostic
            .location()
            .and_then(SourceLocation::cell_address)
            .expect("cell location")
            .to_string(),
        "C2"
    );
    assert_eq!(
        Diagnostic::new(
            DiagnosticCode::new("core.invalid_input").expect("code"),
            DiagnosticSeverity::Error,
            "",
            None,
        ),
        Err(ValidationError::DiagnosticMessageEmpty)
    );

    assert_eq!(
        ProviderIdentity::new("", "0.1.0"),
        Err(ValidationError::ProviderNameEmpty)
    );
    assert_eq!(
        ProviderIdentity::new("workbook", ""),
        Err(ValidationError::ProviderVersionEmpty)
    );
    let provenance = provenance();
    assert_eq!(provenance.provider().name(), "workbook-tests");
    assert_eq!(
        provenance.input_hash().expect("input hash").as_bytes(),
        &[7; 32]
    );
}

#[test]
fn excel_errors_do_not_conflate_unsupported_engine_features() {
    let error_values = [
        ExcelError::Null,
        ExcelError::DivisionByZero,
        ExcelError::Value,
        ExcelError::Reference,
        ExcelError::Name,
        ExcelError::Number,
        ExcelError::NotAvailable,
        ExcelError::GettingData,
        ExcelError::Spill,
        ExcelError::Calculation,
    ];
    assert!(
        !error_values
            .iter()
            .any(|error| error.as_str().contains("UNSUPPORTED"))
    );
}

fn sheet(id: u32, name: &str) -> Sheet {
    Sheet::new(
        SheetId::new(id).expect("valid sheet ID"),
        SheetName::new(name).expect("valid sheet name"),
        SheetVisibility::Visible,
    )
}

fn provenance() -> Provenance {
    Provenance::new(
        ProviderIdentity::new("workbook-tests", "0.1.0").expect("provider"),
        Some(InputHash::sha256([7; 32])),
    )
}
