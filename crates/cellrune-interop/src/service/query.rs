//! Bounded workbook reads and deterministic metadata reports.

use cellrune::{
    CellAddress, CellRange, DateSystem, DefinedNameAnalysis, DefinedNameDynamicKind,
    DefinedNameExternalTargetKind, DefinedNameInvalidReason, DefinedNameReferenceArea,
    DefinedNameSheetSpan, DefinedNameUnsupportedReason, FormulaCapability, FunctionSupport,
    SheetId, WorkbookSnapshot, analyze_defined_name, scan_formula_capabilities_with_options,
    scan_function_usage, supported_function_catalog,
};

use super::WorkbookSession;
use crate::convert::{
    calculation_options, cell_dto, cell_reference, count_u64, document_kind, range_text,
    visibility_name,
};
use crate::{
    CalculationOptionsDto, CapabilityEntryDto, CapabilityPageDto, DefinedNameDynamicKindDto,
    DefinedNameExternalTargetKindDto, DefinedNameInspectionDto, DefinedNameInspectionRequestDto,
    DefinedNameInspectionResultDto, DefinedNameInvalidReasonDto, DefinedNameReferenceAreaDto,
    DefinedNameSheetSpanDto, DefinedNameUnsupportedReasonDto, FunctionCatalogEntryDto,
    FunctionCatalogReportDto, FunctionUsageEntryDto, FunctionUsageReportDto,
    INTEROP_SCHEMA_VERSION, InteropError, RangePageDto, RangeRequestDto, SheetSummaryDto,
    TableColumnDto, TableSummaryDto, WorkbookSummaryDto,
};

const UNKNOWN_RESULT_VARIANT: &str = "core_result_variant";
const UNKNOWN_AREA_VARIANT: &str = "core_reference_area_variant";
const UNKNOWN_DYNAMIC_KIND: &str = "core_dynamic_kind_variant";
const UNKNOWN_EXTERNAL_TARGET: &str = "core_external_target_variant";
const UNKNOWN_INVALID_REASON: &str = "core_invalid_reason_variant";
const UNKNOWN_UNSUPPORTED_REASON: &str = "core_unsupported_reason_variant";

fn table_summary(table: &cellrune::Table) -> TableSummaryDto {
    TableSummaryDto {
        id: table.id().get(),
        name: table.name().as_str().to_owned(),
        display_name: table.display_name().as_str().to_owned(),
        range: range_text(table.range().start(), table.range().end()),
        header_row_count: table.header_row_count(),
        totals_row_count: table.totals_row_count(),
        columns: table
            .columns()
            .iter()
            .map(|column| TableColumnDto {
                id: column.id(),
                name: column.name().to_owned(),
                totals_row_function: column
                    .totals_row_function()
                    .map(|function| function.as_str().to_owned()),
            })
            .collect(),
    }
}

/// Default number of cells returned by one range or capability page.
pub const DEFAULT_PAGE_SIZE: u32 = 1_000;
/// Hard maximum number of entries returned by one interop page.
pub const MAX_PAGE_SIZE: u32 = 10_000;

impl WorkbookSession {
    /// Returns bounded workbook metadata without cell contents.
    pub fn summary(&self) -> WorkbookSummaryDto {
        let workbook = self.engine.workbook();
        WorkbookSummaryDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            semantic_revision: workbook.semantic_revision(),
            document_backed: self.engine.draft().is_document_backed(),
            document_kind: document_kind(self.engine.draft()).to_owned(),
            date_system: match workbook.date_system() {
                DateSystem::Excel1900 => "excel_1900",
                DateSystem::Excel1904 => "excel_1904",
            }
            .to_owned(),
            diagnostic_count: count_u64(workbook.diagnostics().len()),
            sheets: workbook
                .sheets()
                .iter()
                .map(|sheet| SheetSummaryDto {
                    id: sheet.id().get(),
                    name: sheet.name().as_str().to_owned(),
                    visibility: visibility_name(sheet.visibility()).to_owned(),
                    cell_count: count_u64(sheet.len()),
                    used_range: sheet
                        .used_range()
                        .map(|range| range_text(range.start(), range.end())),
                    merged_ranges: sheet
                        .merged_ranges()
                        .iter()
                        .map(|range| range_text(range.start(), range.end()))
                        .collect(),
                    tables: sheet.tables().iter().map(table_summary).collect(),
                })
                .collect(),
        }
    }

    /// Reads a bounded row-major page, including blank cells and current calculated results.
    ///
    /// # Errors
    ///
    /// Returns a typed input or validation error for an unknown sheet, invalid range, excessive
    /// page size, or out-of-range offset.
    pub fn read_range(&self, request: &RangeRequestDto) -> Result<RangePageDto, InteropError> {
        let workbook = self.engine.workbook();
        let sheet = workbook
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let start = CellAddress::from_a1(&request.start)?;
        let end = CellAddress::from_a1(&request.end)?;
        let range = CellRange::new(start, end)?;
        let total_cells = u64::from(range.height()) * u64::from(range.width());
        if request.offset > total_cells {
            return Err(InteropError::page_offset());
        }
        let limit = normalized_limit(request.limit)?;
        let remaining = total_cells - request.offset;
        let returned = remaining.min(u64::from(limit));
        let mut cells = Vec::with_capacity(returned as usize);
        for page_index in 0..returned {
            let flat_index = request.offset + page_index;
            let row_offset = flat_index / u64::from(range.width());
            let column_offset = flat_index % u64::from(range.width());
            let address = CellAddress::from_indices(
                start.row().get() + row_offset as u32,
                start.column().get() + column_offset as u32,
            )?;
            cells.push(cell_dto(
                workbook,
                self.current_calculation(),
                sheet.id(),
                address,
            ));
        }
        let consumed = request.offset + returned;
        Ok(RangePageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: sheet.name().as_str().to_owned(),
            start: start.to_string(),
            end: end.to_string(),
            total_cells,
            offset: request.offset,
            next_offset: (consumed < total_cells).then_some(consumed),
            cells,
        })
    }

    /// Inspects one workbook or sheet-local defined name without running calculation.
    ///
    /// # Errors
    ///
    /// Returns a typed input error when `current_sheet` is unknown, or a typed state error when
    /// bounded analysis cannot complete.
    pub fn inspect_defined_name(
        &self,
        request: &DefinedNameInspectionRequestDto,
    ) -> Result<DefinedNameInspectionDto, InteropError> {
        let workbook = self.engine.workbook();
        let current_sheet = request
            .current_sheet
            .as_deref()
            .map(|name| {
                workbook
                    .sheet_by_name(name)
                    .map(|sheet| sheet.id())
                    .ok_or_else(InteropError::sheet_not_found)
            })
            .transpose()?;
        let result = analyze_defined_name(workbook, &request.name, current_sheet)?;
        Ok(DefinedNameInspectionDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            result: defined_name_result(workbook, result)?,
        })
    }

    /// Scans formula capabilities and returns a bounded deterministic page.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for an excessive page size or out-of-range offset, or a
    /// validation error for a non-finite deterministic input.
    pub fn capabilities(
        &self,
        options: CalculationOptionsDto,
        offset: u64,
        limit: u32,
    ) -> Result<CapabilityPageDto, InteropError> {
        let options = calculation_options(options)?;
        let report = scan_formula_capabilities_with_options(self.engine.workbook(), options);
        let total = count_u64(report.formula_count());
        if offset > total {
            return Err(InteropError::page_offset());
        }
        let limit = normalized_limit(limit)?;
        let entries = report
            .entries()
            .iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|entry| {
                let issues = match entry.capability() {
                    FormulaCapability::Supported => Vec::new(),
                    FormulaCapability::Unsupported(issues) => issues
                        .iter()
                        .map(|issue| issue.code().as_str().to_owned())
                        .collect(),
                };
                CapabilityEntryDto {
                    cell: cell_reference(self.engine.workbook(), entry.cell()),
                    supported: issues.is_empty(),
                    issue_codes: issues,
                }
            })
            .collect::<Vec<_>>();
        let consumed = offset + count_u64(entries.len());
        Ok(CapabilityPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            formula_count: total,
            supported_count: count_u64(report.supported_count()),
            offset,
            next_offset: (consumed < total).then_some(consumed),
            entries,
        })
    }

    /// Reports normalized function demand for the current workbook.
    pub fn function_usage(&self) -> FunctionUsageReportDto {
        let workbook = self.engine.workbook();
        let report = scan_function_usage(workbook);
        FunctionUsageReportDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            formula_count: count_u64(report.formula_count()),
            parsed_formula_count: count_u64(report.parsed_formula_count()),
            unparsed_formula_count: count_u64(report.unparsed_formula_count()),
            entries: report
                .entries()
                .iter()
                .map(|entry| FunctionUsageEntryDto {
                    name: entry.name().to_owned(),
                    supported: entry.support() == FunctionSupport::Supported,
                    call_count: entry.call_count(),
                    formula_count: entry.formula_count(),
                    sample_cells: entry
                        .sample_cells()
                        .iter()
                        .map(|cell| cell_reference(workbook, *cell))
                        .collect(),
                })
                .collect(),
        }
    }
}

fn defined_name_result(
    workbook: &WorkbookSnapshot,
    result: DefinedNameAnalysis,
) -> Result<DefinedNameInspectionResultDto, InteropError> {
    match result {
        DefinedNameAnalysis::Rectangular { sheet_id, range } => {
            let (sheet_id, sheet_name) = sheet_identity(workbook, sheet_id)?;
            Ok(DefinedNameInspectionResultDto::Rectangular {
                sheet_id,
                sheet_name,
                range: range_text(range.start(), range.end()),
            })
        }
        DefinedNameAnalysis::ThreeDimensional { sheet_span, range } => {
            Ok(DefinedNameInspectionResultDto::ThreeDimensional {
                sheet_span: defined_name_sheet_span(workbook, sheet_span)?,
                range: range_text(range.start(), range.end()),
            })
        }
        DefinedNameAnalysis::NonRectangular { areas } => {
            let mut converted = Vec::with_capacity(areas.len());
            for area in areas {
                let Some(area) = defined_name_area(workbook, area)? else {
                    return Ok(unsupported_defined_name(UNKNOWN_AREA_VARIANT));
                };
                converted.push(area);
            }
            Ok(DefinedNameInspectionResultDto::NonRectangular { areas: converted })
        }
        DefinedNameAnalysis::EmptyReference => Ok(DefinedNameInspectionResultDto::EmptyReference),
        DefinedNameAnalysis::DynamicFormula { kind, formula } => {
            let Some(dynamic_kind) = defined_name_dynamic_kind(kind) else {
                return Ok(unsupported_defined_name(UNKNOWN_DYNAMIC_KIND));
            };
            Ok(DefinedNameInspectionResultDto::DynamicFormula {
                dynamic_kind,
                formula: format!("={}", formula.as_str()),
            })
        }
        DefinedNameAnalysis::Constant { formula } => Ok(DefinedNameInspectionResultDto::Constant {
            formula: format!("={}", formula.as_str()),
        }),
        DefinedNameAnalysis::ExternalReference { detail } => {
            let Some(target_kind) = defined_name_external_target(detail.target()) else {
                return Ok(unsupported_defined_name(UNKNOWN_EXTERNAL_TARGET));
            };
            Ok(DefinedNameInspectionResultDto::ExternalReference {
                locator: detail.locator().map(str::to_owned),
                workbook: detail.workbook().to_owned(),
                sheet: detail.sheet().map(str::to_owned),
                sheet_end: detail.sheet_end().map(str::to_owned),
                target_kind,
                target_text: detail.target_text().to_owned(),
            })
        }
        DefinedNameAnalysis::Invalid { reason, detail } => {
            let Some(reason) = defined_name_invalid_reason(reason) else {
                return Ok(unsupported_defined_name(UNKNOWN_INVALID_REASON));
            };
            Ok(DefinedNameInspectionResultDto::Invalid {
                reason,
                detail: detail.map(Into::into),
            })
        }
        DefinedNameAnalysis::Unsupported { reason, detail } => {
            let Some(reason) = defined_name_unsupported_reason(reason) else {
                return Ok(unsupported_defined_name(UNKNOWN_UNSUPPORTED_REASON));
            };
            Ok(DefinedNameInspectionResultDto::Unsupported {
                reason,
                detail: detail.map(Into::into),
            })
        }
        DefinedNameAnalysis::NotFound => Ok(DefinedNameInspectionResultDto::NotFound),
        _ => Ok(unsupported_defined_name(UNKNOWN_RESULT_VARIANT)),
    }
}

fn defined_name_area(
    workbook: &WorkbookSnapshot,
    area: DefinedNameReferenceArea,
) -> Result<Option<DefinedNameReferenceAreaDto>, InteropError> {
    match area {
        DefinedNameReferenceArea::Rectangular { sheet_id, range } => {
            let (sheet_id, sheet_name) = sheet_identity(workbook, sheet_id)?;
            Ok(Some(DefinedNameReferenceAreaDto::Rectangular {
                sheet_id,
                sheet_name,
                range: range_text(range.start(), range.end()),
            }))
        }
        DefinedNameReferenceArea::ThreeDimensional { sheet_span, range } => {
            Ok(Some(DefinedNameReferenceAreaDto::ThreeDimensional {
                sheet_span: defined_name_sheet_span(workbook, sheet_span)?,
                range: range_text(range.start(), range.end()),
            }))
        }
        _ => Ok(None),
    }
}

fn unsupported_defined_name(detail: &str) -> DefinedNameInspectionResultDto {
    DefinedNameInspectionResultDto::Unsupported {
        reason: DefinedNameUnsupportedReasonDto::UnsupportedExpression,
        detail: Some(detail.to_owned()),
    }
}

fn defined_name_dynamic_kind(kind: DefinedNameDynamicKind) -> Option<DefinedNameDynamicKindDto> {
    match kind {
        DefinedNameDynamicKind::Offset => Some(DefinedNameDynamicKindDto::Offset),
        DefinedNameDynamicKind::Indirect => Some(DefinedNameDynamicKindDto::Indirect),
        DefinedNameDynamicKind::Spill => Some(DefinedNameDynamicKindDto::Spill),
        DefinedNameDynamicKind::Mixed => Some(DefinedNameDynamicKindDto::Mixed),
        _ => None,
    }
}

fn defined_name_external_target(
    target: DefinedNameExternalTargetKind,
) -> Option<DefinedNameExternalTargetKindDto> {
    match target {
        DefinedNameExternalTargetKind::Reference => {
            Some(DefinedNameExternalTargetKindDto::Reference)
        }
        DefinedNameExternalTargetKind::DefinedName => {
            Some(DefinedNameExternalTargetKindDto::DefinedName)
        }
        DefinedNameExternalTargetKind::StructuredReference => {
            Some(DefinedNameExternalTargetKindDto::StructuredReference)
        }
        _ => None,
    }
}

fn defined_name_invalid_reason(
    reason: DefinedNameInvalidReason,
) -> Option<DefinedNameInvalidReasonDto> {
    match reason {
        DefinedNameInvalidReason::ParseError => Some(DefinedNameInvalidReasonDto::ParseError),
        DefinedNameInvalidReason::CircularReference => {
            Some(DefinedNameInvalidReasonDto::CircularReference)
        }
        DefinedNameInvalidReason::UnresolvedName => {
            Some(DefinedNameInvalidReasonDto::UnresolvedName)
        }
        DefinedNameInvalidReason::InvalidReference => {
            Some(DefinedNameInvalidReasonDto::InvalidReference)
        }
        _ => None,
    }
}

fn defined_name_unsupported_reason(
    reason: DefinedNameUnsupportedReason,
) -> Option<DefinedNameUnsupportedReasonDto> {
    match reason {
        DefinedNameUnsupportedReason::NonReferenceExpression => {
            Some(DefinedNameUnsupportedReasonDto::NonReferenceExpression)
        }
        DefinedNameUnsupportedReason::ContextDependent => {
            Some(DefinedNameUnsupportedReasonDto::ContextDependent)
        }
        DefinedNameUnsupportedReason::UnsupportedExpression => {
            Some(DefinedNameUnsupportedReasonDto::UnsupportedExpression)
        }
        _ => None,
    }
}

fn defined_name_sheet_span(
    workbook: &WorkbookSnapshot,
    span: DefinedNameSheetSpan,
) -> Result<DefinedNameSheetSpanDto, InteropError> {
    let (start_sheet_id, start_sheet_name) = sheet_identity(workbook, span.start())?;
    let (end_sheet_id, end_sheet_name) = sheet_identity(workbook, span.end())?;
    Ok(DefinedNameSheetSpanDto {
        start_sheet_id,
        start_sheet_name,
        end_sheet_id,
        end_sheet_name,
    })
}

fn sheet_identity(
    workbook: &WorkbookSnapshot,
    sheet_id: SheetId,
) -> Result<(u32, String), InteropError> {
    let sheet = workbook
        .sheet_by_id(sheet_id)
        .ok_or_else(InteropError::defined_name_sheet_identity)?;
    Ok((sheet.id().get(), sheet.name().as_str().to_owned()))
}

/// Returns the versioned catalog of accepted calculation function names.
pub fn function_catalog() -> FunctionCatalogReportDto {
    FunctionCatalogReportDto {
        schema_version: INTEROP_SCHEMA_VERSION,
        entries: supported_function_catalog()
            .into_iter()
            .map(|entry| FunctionCatalogEntryDto {
                name: entry.name().to_owned(),
                canonical_name: entry.canonical_name().to_owned(),
                alias: entry.is_alias(),
                returns_array: entry.returns_array(),
                official: entry.is_official(),
            })
            .collect(),
    }
}

fn normalized_limit(limit: u32) -> Result<u32, InteropError> {
    let limit = if limit == 0 { DEFAULT_PAGE_SIZE } else { limit };
    if limit > MAX_PAGE_SIZE {
        return Err(InteropError::page_limit());
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditBatchDto, InteropErrorKind, WorkbookChangeDto};

    fn session_with_defined_names() -> WorkbookSession {
        let mut session = WorkbookSession::create();
        session.add_sheet("Middle").expect("add middle sheet");
        session.add_sheet("Sheet3").expect("add final sheet");
        let revision = session.summary().semantic_revision;
        session
            .apply_changes(
                revision,
                EditBatchDto {
                    changes: vec![
                        WorkbookChangeDto::SetDefinedName {
                            name: "Areas".to_owned(),
                            scope_sheet: None,
                            formula: "=(Sheet1!A1,Sheet3!B2,Sheet1:Sheet3!C3)".to_owned(),
                            hidden: false,
                        },
                        WorkbookChangeDto::SetDefinedName {
                            name: "Dynamic".to_owned(),
                            scope_sheet: None,
                            formula: "=OFFSET(Sheet1!A1,1,0)".to_owned(),
                            hidden: false,
                        },
                    ],
                },
            )
            .expect("install test names");
        session
    }

    #[test]
    fn defined_name_query_preserves_area_and_sheet_span_identity() {
        let session = session_with_defined_names();
        let report = session
            .inspect_defined_name(&DefinedNameInspectionRequestDto {
                name: "areas".to_owned(),
                current_sheet: None,
            })
            .expect("inspection succeeds");

        assert_eq!(report.schema_version, INTEROP_SCHEMA_VERSION);
        assert_eq!(
            report.result,
            DefinedNameInspectionResultDto::NonRectangular {
                areas: vec![
                    DefinedNameReferenceAreaDto::Rectangular {
                        sheet_id: 1,
                        sheet_name: "Sheet1".to_owned(),
                        range: "A1:A1".to_owned(),
                    },
                    DefinedNameReferenceAreaDto::Rectangular {
                        sheet_id: 3,
                        sheet_name: "Sheet3".to_owned(),
                        range: "B2:B2".to_owned(),
                    },
                    DefinedNameReferenceAreaDto::ThreeDimensional {
                        sheet_span: DefinedNameSheetSpanDto {
                            start_sheet_id: 1,
                            start_sheet_name: "Sheet1".to_owned(),
                            end_sheet_id: 3,
                            end_sheet_name: "Sheet3".to_owned(),
                        },
                        range: "C3:C3".to_owned(),
                    },
                ],
            }
        );
    }

    #[test]
    fn defined_name_query_serializes_a_stable_tagged_contract() {
        let session = session_with_defined_names();
        let report = session
            .inspect_defined_name(&DefinedNameInspectionRequestDto {
                name: "Dynamic".to_owned(),
                current_sheet: Some("sheet1".to_owned()),
            })
            .expect("inspection succeeds");
        assert_eq!(
            serde_json::to_value(report).expect("DTO serializes"),
            serde_json::json!({
                "schema_version": 1,
                "result": {
                    "kind": "dynamic_formula",
                    "dynamic_kind": "offset",
                    "formula": "=OFFSET(Sheet1!A1,1,0)"
                }
            })
        );
    }

    #[test]
    fn defined_name_query_rejects_an_unknown_current_sheet() {
        let session = session_with_defined_names();
        let error = session
            .inspect_defined_name(&DefinedNameInspectionRequestDto {
                name: "Areas".to_owned(),
                current_sheet: Some("missing".to_owned()),
            })
            .expect_err("unknown current sheet is caller input failure");

        assert_eq!(error.kind(), InteropErrorKind::Input);
        assert_eq!(error.code(), "interop.sheet.not_found");
    }
}
