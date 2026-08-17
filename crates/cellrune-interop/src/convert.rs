use cellrune::{
    ArithmeticSemantics, CalculationCellId, CalculationCellResult, CalculationDecisionReason,
    CalculationDelta, CalculationDeltaPage, CalculationExecutionMode, CalculationIssue,
    CalculationOptions, CalculationSnapshot, CellAddress, CellContent, CellValue, EditReceipt,
    ExcelError, FinancialSolverSemantics, FiniteNumber, InstallDeltaBasisReason,
    MaterializedResultOrigin, RecalculationMode, RecalculationWriteOptions,
    RecalculationWritePolicy, SavedResult, SheetId, SheetVisibility, TransactionDetailItem,
    TransactionDetailSection, TransactionImpactCause, TransactionImpactCoverage,
    TransactionIssueChangeKind, WorkbookDraft, WorkbookFingerprint, WorkbookSnapshot,
    WorkbookTransactionReceipt, WorkbookTransactionReport, WriteOptions, XlsxDocumentKind,
};

use crate::{
    ArithmeticSemanticsDto, CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto,
    CalculationIssueDto, CalculationLimitsDto, CalculationOptionsDto, CalculationOptionsReportDto,
    CalculationReportDto, CalculationResultDto, CellDto, CellReferenceDto, CellValueDto,
    EditReceiptDto, FinancialSolverSemanticsDto, INTEROP_SCHEMA_VERSION, InteropError,
    MaterializedResultOriginDto, ProviderIdentityDto, RecalculationModeDto, SavedValueStateDto,
    TransactionDetailCountsDto, TransactionDetailItemDto, TransactionDetailSectionDto,
    TransactionImpactCoverageDto, TransactionImpactPageDto, WorkbookFingerprintDto,
    WorkbookTransactionReceiptDto, WorkbookTransactionReportDto, WritableCellValueDto,
    WriteOptionsDto, WriteReportDto,
};

pub(crate) fn workbook_fingerprint(value: WorkbookFingerprint) -> WorkbookFingerprintDto {
    WorkbookFingerprintDto {
        schema_version: value.schema_version(),
        digest_hex: value.to_hex(),
    }
}

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

pub(crate) fn edit_receipt_v2(
    workbook: &WorkbookSnapshot,
    receipt: &EditReceipt,
) -> crate::EditReceiptV2Dto {
    let mut projected = edit_receipt(workbook, receipt);
    projected.schema_version = crate::INTEROP_EDIT_SCHEMA_V2;
    crate::EditReceiptV2Dto {
        receipt: projected,
        changed_table_ids: receipt
            .changed_table_ids()
            .iter()
            .map(|table_id| table_id.get())
            .collect(),
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

pub(crate) fn transaction_report(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    report: &WorkbookTransactionReport,
) -> WorkbookTransactionReportDto {
    let install_delta = report.install_delta();
    WorkbookTransactionReportDto {
        contract_version: report.contract_version(),
        base_revision: report.base_revision(),
        result_revision: report.result_revision(),
        base_fingerprint: workbook_fingerprint(report.base_fingerprint()),
        result_fingerprint: workbook_fingerprint(report.result_fingerprint()),
        input_sha256: report.input_hash().map(|value| hex_bytes(value.as_bytes())),
        calculator_provider: ProviderIdentityDto {
            name: report.calculator_provider().name().to_owned(),
            version: report.calculator_provider().version().to_owned(),
        },
        calculation_options: calculation_options_report(report.calculation_options()),
        base_calculation_reused: report.base_calculation_reused(),
        base_execution_mode: execution_mode_name(report.base_execution_mode()).to_owned(),
        base_decision_reason: decision_reason_name(report.base_decision_reason()).to_owned(),
        candidate_requested_mode: recalculation_mode_dto(report.candidate_requested_mode()),
        candidate_execution_mode: execution_mode_name(report.candidate_execution_mode()).to_owned(),
        candidate_decision_reason: decision_reason_name(report.candidate_decision_reason())
            .to_owned(),
        edit_receipt: transaction_edit_receipt(base, candidate, report.edit_receipt()),
        impact_coverage: match report.impact_coverage() {
            TransactionImpactCoverage::Exact => TransactionImpactCoverageDto::Exact,
            TransactionImpactCoverage::ConservativeFull => {
                TransactionImpactCoverageDto::ConservativeFull
            }
            _ => TransactionImpactCoverageDto::ConservativeFull,
        },
        direct_affected_count: count_u64(report.direct_affected_count()),
        transitive_affected_count: count_u64(report.transitive_affected_count()),
        conservative_affected_count: count_u64(report.conservative_affected_count()),
        base_evaluated_count: count_u64(report.base_evaluated_count()),
        candidate_evaluated_count: count_u64(report.candidate_evaluated_count()),
        parsed_formula_count: count_u64(report.parsed_formula_count()),
        function_iteration_count: report.function_iteration_count(),
        reference_cell_count: report.reference_cell_count(),
        preview_changed_count: count_u64(report.preview_changed_count()),
        preview_removed_count: count_u64(report.preview_removed_count()),
        introduced_issue_count: count_u64(report.introduced_issue_count()),
        resolved_issue_count: count_u64(report.resolved_issue_count()),
        changed_issue_count: count_u64(report.changed_issue_count()),
        install_delta_count: count_u64(
            install_delta
                .changed_cells()
                .len()
                .saturating_add(install_delta.removed_materialized_cells().len()),
        ),
        installed_calculation_revision: report.installed_calculation_revision(),
        installed_calculation_fingerprint: report
            .installed_calculation_fingerprint()
            .map(workbook_fingerprint),
        installed_calculation_options: report
            .installed_calculation_options()
            .map(calculation_options_report),
        install_delta_basis_differs_from_preview_base: report
            .install_delta_basis_differs_from_preview_base(),
        install_delta_basis_reasons: report
            .install_delta_basis_reasons()
            .iter()
            .map(|reason| install_delta_basis_reason_name(*reason).to_owned())
            .collect(),
        detail_counts: TransactionDetailCountsDto {
            affected: count_u64(report.detail_count(TransactionDetailSection::Affected)),
            evaluated: count_u64(report.detail_count(TransactionDetailSection::Evaluated)),
            preview_results: count_u64(
                report.detail_count(TransactionDetailSection::PreviewResults),
            ),
            preview_issues: count_u64(report.detail_count(TransactionDetailSection::PreviewIssues)),
            install_results: count_u64(
                report.detail_count(TransactionDetailSection::InstallResults),
            ),
        },
    }
}

pub(crate) fn transaction_page(
    preview_id: u64,
    section: TransactionDetailSectionDto,
    total_count: usize,
    items: impl IntoIterator<Item = TransactionDetailItem>,
    next_cursor_token: Option<String>,
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
) -> TransactionImpactPageDto {
    TransactionImpactPageDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        preview_id,
        section,
        items: items
            .into_iter()
            .map(|item| transaction_detail_item(base, candidate, item))
            .collect(),
        next_cursor: next_cursor_token.map(|token| crate::PreviewCursorDto { preview_id, token }),
        total_count: count_u64(total_count),
    }
}

pub(crate) fn transaction_receipt(
    workbook: &WorkbookSnapshot,
    receipt: &WorkbookTransactionReceipt,
) -> WorkbookTransactionReceiptDto {
    WorkbookTransactionReceiptDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        edit: edit_receipt(workbook, receipt.edit()),
        calculation_delta: calculation_delta(workbook, receipt.calculation_delta()),
        base_fingerprint: workbook_fingerprint(receipt.base_fingerprint()),
        result_fingerprint: workbook_fingerprint(receipt.result_fingerprint()),
    }
}

pub(crate) fn preview_transaction_receipt(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    report: &WorkbookTransactionReport,
) -> WorkbookTransactionReceiptDto {
    WorkbookTransactionReceiptDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        edit: transaction_edit_receipt(base, candidate, report.edit_receipt()),
        calculation_delta: calculation_delta(candidate, report.install_delta()),
        base_fingerprint: workbook_fingerprint(report.base_fingerprint()),
        result_fingerprint: workbook_fingerprint(report.result_fingerprint()),
    }
}

pub(crate) const fn transaction_detail_section(
    section: TransactionDetailSectionDto,
) -> TransactionDetailSection {
    match section {
        TransactionDetailSectionDto::Affected => TransactionDetailSection::Affected,
        TransactionDetailSectionDto::Evaluated => TransactionDetailSection::Evaluated,
        TransactionDetailSectionDto::PreviewResults => TransactionDetailSection::PreviewResults,
        TransactionDetailSectionDto::PreviewIssues => TransactionDetailSection::PreviewIssues,
        TransactionDetailSectionDto::InstallResults => TransactionDetailSection::InstallResults,
    }
}

fn transaction_edit_receipt(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    receipt: &EditReceipt,
) -> EditReceiptDto {
    EditReceiptDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        base_revision: receipt.base_revision(),
        result_revision: receipt.result_revision(),
        applied_change_count: count_u64(receipt.applied_change_count()),
        changed_cells: receipt
            .changed_cells()
            .iter()
            .map(|cell| transaction_cell_reference(base, candidate, *cell))
            .collect(),
        calculation_changed_cells: receipt
            .calculation_changed_cells()
            .iter()
            .map(|cell| transaction_cell_reference(base, candidate, *cell))
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

fn transaction_detail_item(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    item: TransactionDetailItem,
) -> TransactionDetailItemDto {
    match item {
        TransactionDetailItem::Affected(affected) => TransactionDetailItemDto::Affected {
            cell: transaction_cell_reference(base, candidate, affected.cell()),
            cause: match affected.cause() {
                TransactionImpactCause::Direct => "direct",
                TransactionImpactCause::Transitive => "transitive",
                TransactionImpactCause::Conservative => "conservative",
                _ => "conservative",
            }
            .to_owned(),
        },
        TransactionDetailItem::Evaluated(cell) => TransactionDetailItemDto::Evaluated {
            cell: transaction_cell_reference(base, candidate, cell),
        },
        TransactionDetailItem::PreviewResult(change) => TransactionDetailItemDto::PreviewResult {
            cell: transaction_cell_reference(base, candidate, change.cell()),
            previous_origin: change
                .previous_origin()
                .map(|origin| transaction_origin(base, candidate, origin)),
            previous_result: change.previous_result().map(result_to_dto),
            result_origin: change
                .result_origin()
                .map(|origin| transaction_origin(base, candidate, origin)),
            result: change.result().map(result_to_dto),
        },
        TransactionDetailItem::PreviewIssue(change) => TransactionDetailItemDto::PreviewIssue {
            cell: transaction_cell_reference(base, candidate, change.cell()),
            change_kind: match change.kind() {
                TransactionIssueChangeKind::Introduced => "introduced",
                TransactionIssueChangeKind::Resolved => "resolved",
                TransactionIssueChangeKind::Changed => "changed",
                _ => "changed",
            }
            .to_owned(),
            previous: change.previous().map(issue_to_dto),
            current: change.current().map(issue_to_dto),
        },
        TransactionDetailItem::InstallResult(change) => TransactionDetailItemDto::InstallResult {
            cell: transaction_cell_reference(base, candidate, change.cell()),
            origin: change
                .origin()
                .map(|origin| transaction_origin(base, candidate, origin)),
            result: change.result().map(result_to_dto),
        },
        _ => TransactionDetailItemDto::Unknown,
    }
}

fn transaction_cell_reference(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    id: CalculationCellId,
) -> CellReferenceDto {
    let workbook = if candidate.sheet_by_id(id.sheet_id()).is_some() {
        candidate
    } else {
        base
    };
    cell_reference(workbook, id)
}

fn transaction_origin(
    base: &WorkbookSnapshot,
    candidate: &WorkbookSnapshot,
    origin: MaterializedResultOrigin,
) -> MaterializedResultOriginDto {
    match origin {
        MaterializedResultOrigin::DirectFormula => MaterializedResultOriginDto {
            kind: "direct_formula".to_owned(),
            anchor: None,
            range: None,
        },
        MaterializedResultOrigin::LegacyArray { anchor, range } => MaterializedResultOriginDto {
            kind: "legacy_array".to_owned(),
            anchor: Some(transaction_cell_reference(base, candidate, anchor)),
            range: Some(range_text(range.start(), range.end())),
        },
        MaterializedResultOrigin::DynamicSpill { anchor, range } => MaterializedResultOriginDto {
            kind: "dynamic_spill".to_owned(),
            anchor: Some(transaction_cell_reference(base, candidate, anchor)),
            range: Some(range_text(range.start(), range.end())),
        },
        _ => MaterializedResultOriginDto {
            kind: "unknown".to_owned(),
            anchor: None,
            range: None,
        },
    }
}

fn issue_to_dto(issue: &CalculationIssue) -> CalculationIssueDto {
    CalculationIssueDto {
        code: issue.code().as_str().to_owned(),
        message: issue.message().to_owned(),
        detail: issue.detail().map(str::to_owned),
    }
}

fn calculation_options_report(options: CalculationOptions) -> CalculationOptionsReportDto {
    let limits = options.limits();
    CalculationOptionsReportDto {
        today_serial: options.today_serial().map(FiniteNumber::get),
        now_serial: options.now_serial().map(FiniteNumber::get),
        arithmetic_semantics: match options.arithmetic_semantics() {
            ArithmeticSemantics::ExcelNearZero => ArithmeticSemanticsDto::ExcelNearZero,
            ArithmeticSemantics::Ieee754 => ArithmeticSemanticsDto::Ieee754,
            _ => ArithmeticSemanticsDto::ExcelNearZero,
        },
        financial_solver_semantics: match options.financial_solver_semantics() {
            FinancialSolverSemantics::ExcelIterationBudget => {
                FinancialSolverSemanticsDto::ExcelIterationBudget
            }
            FinancialSolverSemantics::ExtendedSearch => FinancialSolverSemanticsDto::ExtendedSearch,
            _ => FinancialSolverSemanticsDto::ExcelIterationBudget,
        },
        limits: CalculationLimitsDto {
            max_formula_tokens: limits.max_formula_tokens(),
            max_formula_source_bytes: limits.max_formula_source_bytes(),
            max_formula_ast_nodes: limits.max_formula_ast_nodes(),
            max_formula_nesting_depth: limits.max_formula_nesting_depth(),
            max_dependency_edges: limits.max_dependency_edges(),
            max_reference_areas: limits.max_reference_areas(),
            max_array_cells: limits.max_array_cells(),
            max_text_bytes: limits.max_text_bytes(),
            max_function_iterations: limits.max_function_iterations(),
            max_let_bindings: limits.max_let_bindings(),
            max_lambda_depth: limits.max_lambda_depth(),
            max_lambda_invocations: limits.max_lambda_invocations(),
        },
    }
}

const fn recalculation_mode_dto(mode: RecalculationMode) -> RecalculationModeDto {
    match mode {
        RecalculationMode::Auto => RecalculationModeDto::Auto,
        RecalculationMode::Incremental => RecalculationModeDto::Incremental,
        RecalculationMode::Full => RecalculationModeDto::Full,
    }
}

const fn install_delta_basis_reason_name(reason: InstallDeltaBasisReason) -> &'static str {
    match reason {
        InstallDeltaBasisReason::NoInstalledCalculation => "no_installed_calculation",
        InstallDeltaBasisReason::PriorPendingEdits => "prior_pending_edits",
        InstallDeltaBasisReason::CalculationOptionsChanged => "calculation_options_changed",
        InstallDeltaBasisReason::InstalledCalculationIdentityMismatch => {
            "installed_calculation_identity_mismatch"
        }
        _ => "installed_calculation_identity_mismatch",
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    value
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
