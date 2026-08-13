use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationOptions,
    CancellationToken, CellAddress, CellContent, CellValue, EditBatch, ExcelError, FormulaText,
    RecalculationMode, SavedResult, WorkbookCalculationSession, WorkbookChange, WorkbookDraft,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
};

/// The frozen Excel oracle nominal inputs and results for all twenty-six 0.1.15 fixed-income
/// functions. Dates are self-contained `DATE(...)` expressions so the generated workbook needs no
/// shared truth sheet. Expected values are truncated to well within the comparison tolerance.
const NOMINAL_FORMULAS: [(&str, &str, f64); 26] = [
    (
        "B1",
        "=ACCRINT(DATE(2024,1,1),DATE(2024,7,1),DATE(2025,1,1),A1,1000,2)",
        50.0,
    ),
    (
        "B2",
        "=ACCRINTM(DATE(2024,1,1),DATE(2025,1,1),0.05,1000,0)",
        50.0,
    ),
    ("B3", "=COUPDAYBS(DATE(2025,3,15),DATE(2027,1,1),2,0)", 74.0),
    ("B4", "=COUPDAYS(DATE(2025,3,15),DATE(2027,1,1),2,0)", 180.0),
    (
        "B5",
        "=COUPDAYSNC(DATE(2025,3,15),DATE(2027,1,1),2,0)",
        106.0,
    ),
    (
        "B6",
        "=COUPNCD(DATE(2025,3,15),DATE(2027,1,1),2,0)",
        45_839.0,
    ),
    ("B7", "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),2,0)", 4.0),
    (
        "B8",
        "=COUPPCD(DATE(2025,3,15),DATE(2027,1,1),2,0)",
        45_658.0,
    ),
    ("B9", "=DISC(DATE(2025,1,1),DATE(2025,7,1),97,100,0)", 0.06),
    (
        "B10",
        "=DURATION(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,2,0)",
        4.498_903_644_526_2,
    ),
    (
        "B11",
        "=INTRATE(DATE(2025,1,1),DATE(2025,7,1),970,1000,0)",
        0.061_855_670_103_093,
    ),
    (
        "B12",
        "=MDURATION(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,2,0)",
        4.410_689_847_574_7,
    ),
    (
        "B13",
        "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,100,2,0)",
        95.673_855_249_015,
    ),
    (
        "B14",
        "=ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,95,100,2,0)",
        0.061_605_745_668_863,
    ),
    (
        "B15",
        "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,100,2,0)",
        99.603_747_781_038,
    ),
    (
        "B16",
        "=ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,99,100,2,0)",
        0.076_504_400_859_952,
    ),
    (
        "B17",
        "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,100,2,0)",
        104.491_292_503_12,
    ),
    (
        "B18",
        "=PRICEDISC(DATE(2025,1,1),DATE(2025,7,1),0.04,100,0)",
        98.0,
    ),
    (
        "B19",
        "=PRICEMAT(DATE(2025,1,1),DATE(2025,7,1),DATE(2024,7,1),0.05,0.04,0)",
        100.441_176_470_59,
    ),
    (
        "B20",
        "=RECEIVED(DATE(2025,1,1),DATE(2025,7,1),970,0.05,0)",
        994.871_794_871_79,
    ),
    (
        "B21",
        "=TBILLEQ(DATE(2025,1,1),DATE(2025,7,1),0.04)",
        0.041_387_912_461_7,
    ),
    (
        "B22",
        "=TBILLPRICE(DATE(2025,1,1),DATE(2025,7,1),0.04)",
        97.988_888_888_889,
    ),
    (
        "B23",
        "=TBILLYIELD(DATE(2025,1,1),DATE(2025,7,1),98)",
        0.040_590_821_964_144,
    ),
    (
        "B24",
        "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,95,100,2,0)",
        0.061_776_246_409_029,
    ),
    (
        "B25",
        "=YIELDDISC(DATE(2025,1,1),DATE(2025,7,1),97,100,0)",
        0.061_855_670_103_093,
    ),
    (
        "B26",
        "=YIELDMAT(DATE(2025,1,1),DATE(2025,7,1),DATE(2024,7,1),0.05,98,0)",
        0.089_552_238_805_970,
    ),
];

const ERROR_FORMULAS: [(&str, &str, ExcelError); 3] = [
    (
        "D1",
        "=COUPDAYBS(DATE(2025,3,15),DATE(2027,1,1),2,5)",
        ExcelError::Number,
    ),
    (
        "D2",
        "=ACCRINT(DATE(2024,1,1),DATE(2024,7,1),DATE(2025,1,1),0.05,1000,3)",
        ExcelError::Number,
    ),
    (
        "D3",
        "=ODDFPRICE(DATE(2025,2,1),DATE(2025,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,100,2,0)",
        ExcelError::Number,
    ),
];

#[test]
fn v015_fixed_income_full_incremental_write_reopen_pipeline() {
    let source_draft = generated_workbook();
    let source_calculation =
        calculate_workbook(source_draft.workbook(), CalculationOptions::default());
    let source_output = write_xlsx_draft_bytes(
        &source_draft,
        &source_calculation,
        cellrune::RecalculationWriteOptions::default(),
    )
    .expect("write generated fixed-income input workbook");
    let source_document =
        open_xlsx_document_bytes(source_output.bytes(), cellrune::OpenOptions::default())
            .expect("read generated fixed-income input workbook");
    let mut session =
        WorkbookCalculationSession::new(WorkbookDraft::from_document(&source_document));
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
            EditBatch::new([set_number(sheet, "A1", 0.06)]),
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
    assert_eq!(incremental.dirty_count(), 1);
    assert_eq!(incremental.evaluated_count(), 1);
    assert_eq!(incremental.changed_cells().len(), 1);
    assert_eq!(
        incremental.changed_cells()[0].cell().address(),
        address_of("B1")
    );
    let incremental_calculation = session.calculation().expect("incremental calculation");
    let fresh_calculation = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_eq!(
        incremental_calculation.cells().collect::<Vec<_>>(),
        fresh_calculation.cells().collect::<Vec<_>>(),
        "incremental calculation must match a fresh full calculation",
    );
    assert_eq!(
        incremental_calculation.provenance(),
        fresh_calculation.provenance(),
        "incremental and fresh calculations must carry identical provenance",
    );
    assert_eq!(
        cell_result(incremental_calculation, sheet, "B1"),
        Some(&CalculationCellResult::Value(
            CellValue::number(60.0).expect("finite changed accrual")
        )),
        "the referenced rate edit must re-evaluate ACCRINT",
    );

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

    assert_eq!(
        reopened_sheet
            .cell_by_a1("A1")
            .expect("valid input address")
            .expect("reopened input cell")
            .content(),
        &CellContent::Literal(CellValue::number(0.06).expect("finite input")),
        "input preservation at A1",
    );
    for (address, formula) in NOMINAL_FORMULAS
        .iter()
        .map(|(address, formula, _)| (*address, *formula))
        .chain(
            ERROR_FORMULAS
                .iter()
                .map(|(address, formula, _)| (*address, *formula)),
        )
    {
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
    draft
        .set_cell_value(
            sheet,
            address_of("A1"),
            CellValue::number(0.05).expect("finite generated input"),
        )
        .expect("generated input cell");
    for (address, formula, _) in NOMINAL_FORMULAS {
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
    for (address, _, expected) in NOMINAL_FORMULAS {
        let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
            cell_result(calculation, sheet, address)
        else {
            panic!("expected numeric result at {address}");
        };
        let tolerance = 5e-9 + 5e-9 * expected.abs();
        assert!(
            (actual.get() - expected).abs() <= tolerance,
            "unexpected result at {address}: expected {expected}, got {}",
            actual.get(),
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
