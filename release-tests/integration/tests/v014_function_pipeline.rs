use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationOptions,
    CancellationToken, CellAddress, CellContent, CellValue, EditBatch, ExcelError, FormulaText,
    RecalculationMode, SavedResult, WorkbookCalculationSession, WorkbookChange, WorkbookDraft,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

const FUNCTION_FORMULAS: [(&str, &str); 19] = [
    ("B1", "=BESSELI(A1,A2)"),
    ("B2", "=BESSELJ(A1,A2)"),
    ("B3", "=BESSELK(A1,A2)"),
    ("B4", "=BESSELY(A1,A2)"),
    ("B5", "=CONVERT(A3,\"km\",\"m\")"),
    ("B6", "=COMPLEX(A4,A5,\"i\")"),
    ("B7", "=IMABS(B6)"),
    ("B8", "=IMAGINARY(B6)"),
    ("B9", "=IMARGUMENT(B6)"),
    ("B10", "=IMCONJUGATE(B6)"),
    ("B11", "=IMDIV(B6,\"1-2i\")"),
    ("B12", "=IMEXP(B6)"),
    ("B13", "=IMLN(B6)"),
    ("B14", "=IMPOWER(B6,3)"),
    ("B15", "=IMPRODUCT(B6,\"1-2i\")"),
    ("B16", "=IMREAL(B6)"),
    ("B17", "=IMSQRT(B6)"),
    ("B18", "=IMSUB(B6,\"1-2i\")"),
    ("B19", "=IMSUM(B6,\"1-2i\")"),
];

const ERROR_FORMULAS: [(&str, &str, ExcelError); 4] = [
    ("D1", "=BESSELK(0,A2)", ExcelError::Number),
    ("D2", "=CONVERT(1,\"km\",\"s\")", ExcelError::NotAvailable),
    ("D3", "=IMABS(\"not a complex\")", ExcelError::Number),
    ("D4", "=COMPLEX(1,2,\"x\")", ExcelError::Value),
];

#[test]
fn v014_generated_workbook_full_incremental_write_reopen_pipeline() {
    let mut session = WorkbookCalculationSession::new(generated_workbook());
    let sheet = session.workbook().sheets()[0].id();

    let initial = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial full calculation");
    assert_eq!(initial.mode(), CalculationExecutionMode::Full);
    let initial_calculation = session.calculation().expect("initial calculation");
    assert_nominal_results(initial_calculation, sheet);
    assert_error_results(initial_calculation, sheet);

    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([
                set_number(sheet, "A1", 2.0),
                set_number(sheet, "A3", 12.0),
                set_number(sheet, "A4", 5.0),
                set_number(sheet, "A5", -2.0),
            ]),
        )
        .expect("input edit");
    let incremental = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental calculation after input edit");
    assert_eq!(incremental.mode(), CalculationExecutionMode::Incremental);
    let incremental_calculation = session.calculation().expect("incremental calculation");
    let fresh_calculation = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_eq!(
        incremental_calculation.cells().collect::<Vec<_>>(),
        fresh_calculation.cells().collect::<Vec<_>>(),
        "incremental calculation must match a fresh full calculation",
    );
    assert_eq!(
        cell_result(incremental_calculation, sheet, "B5"),
        Some(&CalculationCellResult::Value(
            CellValue::number(12_000.0).expect("finite")
        )),
    );
    assert_eq!(
        cell_result(incremental_calculation, sheet, "B6"),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "5-2i".to_owned()
        ))),
    );
    assert_error_results(incremental_calculation, sheet);

    let output = write_xlsx_draft_bytes(
        session.draft(),
        incremental_calculation,
        cellrune::RecalculationWriteOptions::default(),
    )
    .expect("write incrementally recalculated workbook");
    let reopened = open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default())
        .expect("reopen written workbook");
    let reopened_sheet = reopened
        .workbook()
        .sheet_by_id(sheet)
        .expect("reopened Sheet1");

    for (address, expected) in [
        ("A1", 2.0),
        ("A2", 2.0),
        ("A3", 12.0),
        ("A4", 5.0),
        ("A5", -2.0),
    ] {
        assert_eq!(
            reopened_sheet
                .cell_by_a1(address)
                .expect("valid input address")
                .expect("reopened input cell")
                .content(),
            &CellContent::Literal(CellValue::number(expected).expect("finite input")),
            "input preservation at {address}",
        );
    }
    for (address, formula) in FUNCTION_FORMULAS.iter().copied().chain(
        ERROR_FORMULAS
            .iter()
            .map(|(address, formula, _)| (*address, *formula)),
    ) {
        let cell = reopened_sheet
            .cell_by_a1(address)
            .expect("valid formula address")
            .expect("reopened formula cell");
        let CellContent::Formula(reopened_formula) = cell.content() else {
            panic!("formula was not preserved at {address}");
        };
        assert_eq!(
            reopened_formula.text().expect("formula text").as_str(),
            formula
                .strip_prefix('=')
                .expect("generated formula has leading equals"),
            "formula preservation at {address}",
        );
        assert_eq!(
            reopened_formula.saved_result(),
            &SavedResult::Present(saved_value(incremental_calculation, sheet, address)),
            "saved-result preservation at {address}",
        );
    }

    let reopened_calculation =
        calculate_workbook(reopened.workbook(), CalculationOptions::default());
    assert_eq!(
        reopened_calculation.cells().collect::<Vec<_>>(),
        incremental_calculation.cells().collect::<Vec<_>>(),
        "reopened calculation must match the incremental result",
    );
}

fn generated_workbook() -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (address, value) in [
        ("A1", 1.5),
        ("A2", 2.0),
        ("A3", 2.0),
        ("A4", 3.0),
        ("A5", 4.0),
    ] {
        draft
            .set_cell_value(
                sheet,
                address_of(address),
                CellValue::number(value).expect("finite generated input"),
            )
            .expect("generated input cell");
    }
    for (address, formula) in FUNCTION_FORMULAS {
        draft
            .set_cell_formula(
                sheet,
                address_of(address),
                FormulaText::from_user_input(formula).expect("generated function formula"),
            )
            .expect("generated function cell");
    }
    for (address, formula, _) in ERROR_FORMULAS {
        draft
            .set_cell_formula(
                sheet,
                address_of(address),
                FormulaText::from_user_input(formula).expect("generated error formula"),
            )
            .expect("generated error cell");
    }
    draft
}

fn assert_nominal_results(calculation: &cellrune::CalculationSnapshot, sheet: cellrune::SheetId) {
    for (address, expected) in [
        ("B1", 0.337_834_618_335_680_74_f64),
        ("B2", 0.232_087_672_144_214_72),
        ("B3", 0.583_655_963_256_650_8),
        ("B4", -0.932_193_759_762_973_9),
        ("B5", 2_000.0),
        ("B7", 5.0),
        ("B8", 4.0),
        ("B9", 0.927_295_218_001_612_2),
        ("B16", 3.0),
    ] {
        let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
            cell_result(calculation, sheet, address)
        else {
            panic!("expected numeric result at {address}");
        };
        let tolerance = 5e-13 + 5e-13 * expected.abs();
        assert!(
            (actual.get() - expected).abs() <= tolerance,
            "unexpected result at {address}: expected {expected}, got {}",
            actual.get(),
        );
    }
    for (address, expected) in [
        ("B6", "3+4i"),
        ("B10", "3-4i"),
        ("B11", "-1+2i"),
        ("B12", "-13.1287830814622-15.200784463068i"),
        ("B13", "1.6094379124341+0.927295218001612i"),
        ("B14", "-117+44i"),
        ("B15", "11-2i"),
        ("B17", "2+i"),
        ("B18", "2+6i"),
        ("B19", "4+2i"),
    ] {
        assert_eq!(
            cell_result(calculation, sheet, address),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned(),
            ))),
            "unexpected result at {address}",
        );
    }
}

fn assert_error_results(calculation: &cellrune::CalculationSnapshot, sheet: cellrune::SheetId) {
    for (address, _, expected) in ERROR_FORMULAS {
        assert_eq!(
            cell_result(calculation, sheet, address),
            Some(&CalculationCellResult::Value(CellValue::Error(expected))),
            "unexpected error at {address}",
        );
    }
}

fn cell_result<'a>(
    calculation: &'a cellrune::CalculationSnapshot,
    sheet: cellrune::SheetId,
    address: &str,
) -> Option<&'a CalculationCellResult> {
    calculation.cell(CalculationCellId::new(sheet, address_of(address)))
}

fn saved_value(
    calculation: &cellrune::CalculationSnapshot,
    sheet: cellrune::SheetId,
    address: &str,
) -> CellValue {
    match cell_result(calculation, sheet, address) {
        Some(CalculationCellResult::Value(value)) => value.clone(),
        Some(CalculationCellResult::Unavailable(issue)) => {
            panic!("expected a saved value at {address}, got unavailable {issue:?}")
        }
        None => panic!("expected a saved value at {address}, got no result"),
    }
}

fn set_number(sheet: cellrune::SheetId, address: &str, value: f64) -> WorkbookChange {
    WorkbookChange::set_cell_value(
        sheet,
        address_of(address),
        CellValue::number(value).expect("finite input edit"),
    )
}

fn address_of(address: &str) -> CellAddress {
    CellAddress::from_a1(address).expect("valid generated workbook address")
}
