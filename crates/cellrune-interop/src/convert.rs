use cellrune::{
    ArithmeticSemantics, CalculationCellId, CalculationCellResult, CalculationDecisionReason,
    CalculationDelta, CalculationDeltaPage, CalculationExecutionMode, CalculationOptions,
    CalculationSnapshot, CellAddress, CellContent, CellValue, EditReceipt, ExcelError,
    FinancialSolverSemantics, FiniteNumber, MaterializedResultOrigin, RecalculationMode,
    RecalculationWriteOptions, RecalculationWritePolicy, SavedResult, SheetId, SheetVisibility,
    WorkbookDraft, WorkbookSnapshot, WriteOptions, XlsxDocumentKind,
};

use crate::{
    ArithmeticSemanticsDto, CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto,
    CalculationOptionsDto, CalculationReportDto, CalculationResultDto, CellDto, CellReferenceDto,
    CellValueDto, EditReceiptDto, FinancialSolverSemanticsDto, INTEROP_SCHEMA_VERSION,
    InteropError, RecalculationModeDto, SavedValueStateDto, WritableCellValueDto, WriteOptionsDto,
    WriteReportDto,
};

pub(crate) fn calculation_options(
    options: CalculationOptionsDto,
) -> Result<CalculationOptions, InteropError> {
    let mut converted = CalculationOptions::default()
        .with_arithmetic_semantics(match options.arithmetic_semantics {
            ArithmeticSemanticsDto::ExcelNearZero => ArithmeticSemantics::ExcelNearZero,
            ArithmeticSemanticsDto::Ieee754 => ArithmeticSemantics::Ieee754,
        })
        .with_financial_solver_semantics(match options.financial_solver_semantics {
            FinancialSolverSemanticsDto::ExcelIterationBudget => {
                FinancialSolverSemantics::ExcelIterationBudget
            }
            FinancialSolverSemanticsDto::ExtendedSearch => FinancialSolverSemantics::ExtendedSearch,
        });
    if let Some(value) = options.today_serial {
        converted = converted.with_today_serial(FiniteNumber::new(value)?);
    }
    if let Some(value) = options.now_serial {
        converted = converted.with_now_serial(FiniteNumber::new(value)?);
    }
    Ok(converted)
}

pub(crate) const fn recalculation_mode(mode: RecalculationModeDto) -> RecalculationMode {
    match mode {
        RecalculationModeDto::Auto => RecalculationMode::Auto,
        RecalculationModeDto::Incremental => RecalculationMode::Incremental,
        RecalculationModeDto::Full => RecalculationMode::Full,
    }
}

pub(crate) fn write_options(
    options: WriteOptionsDto,
    replace_existing: bool,
) -> RecalculationWriteOptions {
    let policy = if options.invalidate_unavailable {
        RecalculationWritePolicy::InvalidateUnavailable
    } else {
        RecalculationWritePolicy::RequireComplete
    };
    RecalculationWriteOptions::new(WriteOptions::default().with_replace_existing(replace_existing))
        .with_policy(policy)
}

pub(crate) fn value_from_dto(value: WritableCellValueDto) -> Result<CellValue, InteropError> {
    match value {
        WritableCellValueDto::Blank => Ok(CellValue::Blank),
        WritableCellValueDto::Number { value } => Ok(CellValue::Number(FiniteNumber::new(value)?)),
        WritableCellValueDto::Text { value } => Ok(CellValue::Text(value)),
        WritableCellValueDto::Logical { value } => Ok(CellValue::Logical(value)),
        WritableCellValueDto::Error { value } => Ok(CellValue::Error(parse_excel_error(&value)?)),
    }
}

pub(crate) fn cell_dto(
    workbook: &WorkbookSnapshot,
    calculation: Option<&CalculationSnapshot>,
    sheet_id: SheetId,
    address: CellAddress,
) -> CellDto {
    let cell = workbook
        .sheet_by_id(sheet_id)
        .and_then(|sheet| sheet.cell(address));
    let (formula, source_value, source_value_state) = match cell.map(|cell| cell.content()) {
        Some(CellContent::Literal(value)) => {
            (None, value_to_dto(value), SavedValueStateDto::Literal)
        }
        Some(CellContent::Formula(formula)) => {
            let formula_text = formula.text().map(|text| format!("={}", text.as_str()));
            match formula.saved_result() {
                SavedResult::Present(value) => {
                    (formula_text, value_to_dto(value), SavedValueStateDto::Saved)
                }
                SavedResult::Missing => (
                    formula_text,
                    CellValueDto::Blank,
                    SavedValueStateDto::Missing,
                ),
                SavedResult::Invalid(_) => (
                    formula_text,
                    CellValueDto::Blank,
                    SavedValueStateDto::Invalid,
                ),
            }
        }
        None => (None, CellValueDto::Blank, SavedValueStateDto::Literal),
    };
    let id = CalculationCellId::new(sheet_id, address);
    let calculated = calculation.and_then(|calculation| {
        calculation
            .materialized_cell(id)
            .map(|cell| result_to_dto(cell.result()))
    });
    CellDto {
        address: address.to_string(),
        formula,
        source_value,
        source_value_state,
        calculated,
    }
}

pub(crate) fn calculation_report(
    workbook: &WorkbookSnapshot,
    calculation: &CalculationSnapshot,
) -> CalculationReportDto {
    let value_count = calculation
        .cells()
        .filter(|(_, result)| matches!(result, CalculationCellResult::Value(_)))
        .count();
    CalculationReportDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        semantic_revision: workbook.semantic_revision(),
        formula_count: count_u64(calculation.len()),
        value_count: count_u64(value_count),
        unavailable_count: count_u64(calculation.len() - value_count),
        materialized_cell_count: count_u64(calculation.materialized_cells().len()),
    }
}

pub(crate) fn edit_receipt(workbook: &WorkbookSnapshot, receipt: &EditReceipt) -> EditReceiptDto {
    EditReceiptDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        base_revision: receipt.base_revision(),
        result_revision: receipt.result_revision(),
        applied_change_count: count_u64(receipt.applied_change_count()),
        changed_cells: receipt
            .changed_cells()
            .iter()
            .map(|cell| cell_reference(workbook, *cell))
            .collect(),
        calculation_changed_cells: receipt
            .calculation_changed_cells()
            .iter()
            .map(|cell| cell_reference(workbook, *cell))
            .collect(),
        created_sheet_ids: receipt
            .created_sheet_ids()
            .iter()
            .map(|sheet_id| sheet_id.get())
            .collect(),
        topology_changed: receipt.topology_changed(),
        calculation_metadata_changed: receipt.calculation_metadata_changed(),
    }
}

pub(crate) fn calculation_delta(
    workbook: &WorkbookSnapshot,
    delta: &CalculationDelta,
) -> CalculationDeltaDto {
    CalculationDeltaDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        cursor: delta.cursor(),
        base_revision: delta.base_revision(),
        result_revision: delta.result_revision(),
        mode: execution_mode_name(delta.mode()).to_owned(),
        reason: decision_reason_name(delta.reason()).to_owned(),
        dirty_count: count_u64(delta.dirty_count()),
        evaluated_count: count_u64(delta.evaluated_count()),
        parsed_formula_count: count_u64(delta.parsed_formula_count()),
        changed_cells: delta
            .changed_cells()
            .iter()
            .map(|change| {
                let (origin, anchor, range) = match change.origin() {
                    MaterializedResultOrigin::DirectFormula => ("direct_formula", None, None),
                    MaterializedResultOrigin::LegacyArray { anchor, range } => (
                        "legacy_array",
                        Some(cell_reference(workbook, anchor)),
                        Some(range_text(range.start(), range.end())),
                    ),
                    MaterializedResultOrigin::DynamicSpill { anchor, range } => (
                        "dynamic_spill",
                        Some(cell_reference(workbook, anchor)),
                        Some(range_text(range.start(), range.end())),
                    ),
                    _ => ("unknown", None, None),
                };
                CalculationDeltaCellDto {
                    cell: cell_reference(workbook, change.cell()),
                    origin: origin.to_owned(),
                    anchor,
                    range,
                    result: result_to_dto(change.result()),
                }
            })
            .collect(),
        removed_materialized_cells: delta
            .removed_materialized_cells()
            .iter()
            .map(|cell| cell_reference(workbook, *cell))
            .collect(),
    }
}

pub(crate) fn calculation_delta_page(
    workbook: &WorkbookSnapshot,
    page: &CalculationDeltaPage,
) -> CalculationDeltaPageDto {
    CalculationDeltaPageDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        requested_cursor: page.requested_cursor(),
        next_cursor: page.next_cursor(),
        deltas: page
            .deltas()
            .iter()
            .map(|delta| calculation_delta(workbook, delta))
            .collect(),
    }
}

pub(crate) fn cell_reference(
    workbook: &WorkbookSnapshot,
    id: CalculationCellId,
) -> CellReferenceDto {
    let sheet_name = workbook
        .sheet_by_id(id.sheet_id())
        .map_or_else(String::new, |sheet| sheet.name().as_str().to_owned());
    CellReferenceDto {
        sheet_id: id.sheet_id().get(),
        sheet_name,
        address: id.address().to_string(),
    }
}

pub(crate) fn write_report(
    workbook: &WorkbookSnapshot,
    report: &cellrune::WriteReport,
) -> WriteReportDto {
    WriteReportDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        complete: report.is_complete(),
        policy: match report.policy() {
            RecalculationWritePolicy::RequireComplete => "require_complete",
            RecalculationWritePolicy::InvalidateUnavailable => "invalidate_unavailable",
            _ => "unknown",
        }
        .to_owned(),
        materialized_count: count_u64(report.materialized_count()),
        invalidated_cells: report
            .invalidated_cells()
            .iter()
            .map(|cell| cell_reference(workbook, *cell))
            .collect(),
        changed_parts: report
            .changed_parts()
            .iter()
            .map(|source| source.as_str().to_owned())
            .collect(),
        removed_parts: report
            .removed_parts()
            .iter()
            .map(|source| source.as_str().to_owned())
            .collect(),
        diagnostic_count: count_u64(report.diagnostics().len()),
    }
}

pub(crate) fn document_kind(draft: &WorkbookDraft) -> &'static str {
    match draft.document_kind() {
        Some(XlsxDocumentKind::Xlsx) => "xlsx",
        Some(XlsxDocumentKind::Xlsm) => "xlsm",
        None => "new_xlsx",
        Some(_) => "open_xml",
    }
}

pub(crate) const fn visibility_name(visibility: SheetVisibility) -> &'static str {
    match visibility {
        SheetVisibility::Visible => "visible",
        SheetVisibility::Hidden => "hidden",
        SheetVisibility::VeryHidden => "very_hidden",
    }
}

pub(crate) fn range_text(start: CellAddress, end: CellAddress) -> String {
    format!("{start}:{end}")
}

pub(crate) fn count_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn parse_excel_error(value: &str) -> Result<ExcelError, InteropError> {
    match value {
        "#NULL!" => Ok(ExcelError::Null),
        "#DIV/0!" => Ok(ExcelError::DivisionByZero),
        "#VALUE!" => Ok(ExcelError::Value),
        "#REF!" => Ok(ExcelError::Reference),
        "#NAME?" => Ok(ExcelError::Name),
        "#NUM!" => Ok(ExcelError::Number),
        "#N/A" => Ok(ExcelError::NotAvailable),
        "#GETTING_DATA" => Ok(ExcelError::GettingData),
        "#SPILL!" => Ok(ExcelError::Spill),
        "#CALC!" => Ok(ExcelError::Calculation),
        _ => Err(InteropError::excel_error(value.to_owned())),
    }
}

pub(crate) fn value_to_dto(value: &CellValue) -> CellValueDto {
    match value {
        CellValue::Blank => CellValueDto::Blank,
        CellValue::Number(value) => CellValueDto::Number { value: value.get() },
        CellValue::Text(value) => CellValueDto::Text {
            value: value.clone(),
        },
        CellValue::Logical(value) => CellValueDto::Logical { value: *value },
        CellValue::Error(value) => CellValueDto::Error {
            value: value.as_str().to_owned(),
        },
        _ => CellValueDto::Unsupported,
    }
}

pub(crate) fn result_to_dto(result: &CalculationCellResult) -> CalculationResultDto {
    match result {
        CalculationCellResult::Value(value) => CalculationResultDto::Value {
            value: value_to_dto(value),
        },
        CalculationCellResult::Unavailable(issue) => CalculationResultDto::Unavailable {
            code: issue.code().as_str().to_owned(),
            message: issue.message().to_owned(),
            detail: issue.detail().map(str::to_owned),
        },
    }
}

const fn execution_mode_name(mode: CalculationExecutionMode) -> &'static str {
    match mode {
        CalculationExecutionMode::Incremental => "incremental",
        CalculationExecutionMode::Full => "full",
    }
}

const fn decision_reason_name(reason: CalculationDecisionReason) -> &'static str {
    match reason {
        CalculationDecisionReason::InitialCalculation => "initial_calculation",
        CalculationDecisionReason::FullRequested => "full_requested",
        CalculationDecisionReason::IncrementalRequested => "incremental_requested",
        CalculationDecisionReason::DirtySubset => "dirty_subset",
        CalculationDecisionReason::NoDirtyFormulas => "no_dirty_formulas",
        CalculationDecisionReason::TopologyChanged => "topology_changed",
        CalculationDecisionReason::OptionsChanged => "options_changed",
        CalculationDecisionReason::DynamicTopology => "dynamic_topology",
        CalculationDecisionReason::DirtySetCoversWorkbook => "dirty_set_covers_workbook",
        _ => "unknown",
    }
}
