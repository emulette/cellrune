//! Bounded workbook reads and deterministic metadata reports.

use cellrune::{
    CellAddress, CellRange, DateSystem, FormulaCapability, FunctionSupport,
    scan_formula_capabilities_with_options, scan_function_usage, supported_function_catalog,
};

use super::WorkbookSession;
use crate::convert::{
    calculation_options, cell_dto, cell_reference, count_u64, document_kind, range_text,
    visibility_name,
};
use crate::{
    CalculationOptionsDto, CapabilityEntryDto, CapabilityPageDto, FunctionCatalogEntryDto,
    FunctionCatalogReportDto, FunctionUsageEntryDto, FunctionUsageReportDto,
    INTEROP_SCHEMA_VERSION, InteropError, RangePageDto, RangeRequestDto, SheetSummaryDto,
    WorkbookSummaryDto,
};

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
