use cellrune::{
    CalculationOptions, CancellationToken, CellAddress, CellValue, EditBatch, FiniteNumber,
    FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
};
use cellrune_interop::{
    CalculationOptionsDto, EditBatchV2Dto, RecalculationModeDto, WorkbookChangeDto,
    WorkbookChangeV2Dto, WorkbookSession, WritableCellValueDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scenario {
    CurrentCalculatedBase,
    MissingBaseCalculation,
    StalePendingUncalculatedEdit,
    OptionsMismatchedBase,
    TopologyFullFallback,
}

impl Scenario {
    pub(super) const ALL: [Self; 5] = [
        Self::CurrentCalculatedBase,
        Self::MissingBaseCalculation,
        Self::StalePendingUncalculatedEdit,
        Self::OptionsMismatchedBase,
        Self::TopologyFullFallback,
    ];

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::CurrentCalculatedBase => "current_calculated_base",
            Self::MissingBaseCalculation => "missing_base_calculation",
            Self::StalePendingUncalculatedEdit => "stale_pending_uncalculated_edit",
            Self::OptionsMismatchedBase => "options_mismatched_base",
            Self::TopologyFullFallback => "topology_full_fallback",
        }
    }

    pub(super) fn parse(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.as_str() == value)
            .unwrap_or_else(|| panic!("unknown benchmark scenario: {value}"))
    }
}

pub(super) fn core_session(formulas: u32, scenario: Scenario) -> WorkbookCalculationSession {
    let mut changes = Vec::with_capacity(formulas as usize + 2);
    changes.push(WorkbookChange::set_cell_value(
        sheet(),
        address(1, 1),
        number(1.0),
    ));
    changes.push(WorkbookChange::set_cell_value(
        sheet(),
        address(2, 1),
        number(1.0),
    ));
    for row in 1..=formulas {
        let input_row = if row.is_multiple_of(2) { 2 } else { 1 };
        changes.push(WorkbookChange::set_cell_formula(
            sheet(),
            address(row, 2),
            FormulaText::from_xlsx(format!("$A${input_row}+{row}"))
                .expect("generated fanout formula"),
        ));
    }
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(0, EditBatch::new(changes))
        .expect("initial benchmark workbook");
    match scenario {
        Scenario::MissingBaseCalculation => {}
        Scenario::OptionsMismatchedBase => {
            recalculate_core(&mut session, mismatched_core_options());
        }
        _ => recalculate_core(&mut session, CalculationOptions::default()),
    }
    if scenario == Scenario::StalePendingUncalculatedEdit {
        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet(),
                    address(1, 1),
                    number(2.0),
                )]),
            )
            .expect("pending uncalculated edit");
    }
    session
}

pub(super) fn core_transaction_batch(scenario: Scenario) -> EditBatch {
    let change = match scenario {
        Scenario::CurrentCalculatedBase
        | Scenario::MissingBaseCalculation
        | Scenario::OptionsMismatchedBase => {
            WorkbookChange::set_cell_value(sheet(), address(1, 1), number(2.0))
        }
        Scenario::StalePendingUncalculatedEdit => {
            WorkbookChange::set_cell_value(sheet(), address(1, 1), number(3.0))
        }
        Scenario::TopologyFullFallback => WorkbookChange::set_cell_formula(
            sheet(),
            address(1, 2),
            FormulaText::from_xlsx("$A$1+1000001").expect("topology formula"),
        ),
    };
    EditBatch::new([change])
}

pub(super) fn interop_session(formulas: u32, scenario: Scenario) -> WorkbookSession {
    let mut changes = Vec::with_capacity(formulas as usize + 2);
    changes.push(value_change("A1", 1.0));
    changes.push(value_change("A2", 1.0));
    for row in 1..=formulas {
        let input_row = if row.is_multiple_of(2) { 2 } else { 1 };
        changes.push(formula_change(
            &format!("B{row}"),
            &format!("=$A${input_row}+{row}"),
        ));
    }
    let mut session = WorkbookSession::create();
    session
        .apply_changes_v2(0, EditBatchV2Dto { changes })
        .expect("initial interop benchmark workbook");
    match scenario {
        Scenario::MissingBaseCalculation => {}
        Scenario::OptionsMismatchedBase => {
            session
                .recalculate(RecalculationModeDto::Full, mismatched_interop_options())
                .expect("mismatched interop benchmark calculation");
        }
        _ => {
            session
                .recalculate(RecalculationModeDto::Full, CalculationOptionsDto::default())
                .expect("initial interop benchmark calculation");
        }
    }
    if scenario == Scenario::StalePendingUncalculatedEdit {
        session
            .apply_changes_v2(
                session.summary().semantic_revision,
                EditBatchV2Dto {
                    changes: vec![value_change("A1", 2.0)],
                },
            )
            .expect("pending interop edit");
    }
    session
}

pub(super) fn interop_transaction_batch(scenario: Scenario) -> EditBatchV2Dto {
    let change = match scenario {
        Scenario::CurrentCalculatedBase
        | Scenario::MissingBaseCalculation
        | Scenario::OptionsMismatchedBase => value_change("A1", 2.0),
        Scenario::StalePendingUncalculatedEdit => value_change("A1", 3.0),
        Scenario::TopologyFullFallback => formula_change("B1", "=$A$1+1000001"),
    };
    EditBatchV2Dto {
        changes: vec![change],
    }
}

fn recalculate_core(session: &mut WorkbookCalculationSession, options: CalculationOptions) {
    session
        .recalculate(RecalculationMode::Full, options, CancellationToken::new())
        .expect("initial benchmark calculation");
}

fn mismatched_core_options() -> CalculationOptions {
    CalculationOptions::default()
        .with_today_serial(FiniteNumber::new(45_000.0).expect("finite benchmark TODAY serial"))
}

fn mismatched_interop_options() -> CalculationOptionsDto {
    CalculationOptionsDto {
        today_serial: Some(45_000.0),
        ..CalculationOptionsDto::default()
    }
}

fn value_change(address: &str, value: f64) -> WorkbookChangeV2Dto {
    WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetValue {
        sheet: "Sheet1".to_owned(),
        address: address.to_owned(),
        value: WritableCellValueDto::Number { value },
    })
}

fn formula_change(address: &str, formula: &str) -> WorkbookChangeV2Dto {
    WorkbookChangeV2Dto::V1(WorkbookChangeDto::SetFormula {
        sheet: "Sheet1".to_owned(),
        address: address.to_owned(),
        formula: formula.to_owned(),
        dynamic_range: None,
    })
}

fn sheet() -> SheetId {
    SheetId::new(1).expect("default sheet")
}

fn address(row: u32, column: u32) -> CellAddress {
    CellAddress::from_indices(row, column).expect("benchmark address")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite benchmark number"))
}
