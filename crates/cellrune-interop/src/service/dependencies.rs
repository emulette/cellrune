//! Bounded pages of a cell's precedents and dependents.

use cellrune::{CalculationCellId, CellAddress, PrecedentKind};

use super::WorkbookSession;
use super::query::normalized_limit;
use crate::convert::{cell_reference, count_u64, range_text};
use crate::{
    CellDependencyRequestDto, DependentDto, DependentPageDto, INTEROP_SCHEMA_VERSION, InteropError,
    PrecedentDto, PrecedentPageDto,
};

impl WorkbookSession {
    /// Reads one page of the rectangles the formula at a cell reads, from the dependency graph of
    /// the calculation installed for the current revision.
    ///
    /// # Errors
    ///
    /// Returns `interop.calculation.required` when the current revision has not been calculated,
    /// or a typed input error for an unknown sheet, invalid address, excessive page size, or
    /// out-of-range offset.
    pub fn precedents(
        &self,
        request: &CellDependencyRequestDto,
    ) -> Result<PrecedentPageDto, InteropError> {
        let cell = self.dependency_cell(request)?;
        let precedents = self
            .engine
            .precedents(cell)
            .ok_or_else(InteropError::calculation_required)?;
        let (total, next_offset, entries) = page(request, precedents.precedents(), |precedent| {
            let workbook = self.engine.workbook();
            let sheet_name = workbook
                .sheet_by_id(precedent.sheet_id())
                .map_or_else(String::new, |sheet| sheet.name().as_str().to_owned());
            let range = precedent.range();
            PrecedentDto {
                sheet_id: precedent.sheet_id().get(),
                sheet_name,
                range: if range.start() == range.end() {
                    range.start().to_string()
                } else {
                    range_text(range.start(), range.end())
                },
                kind: match precedent.kind() {
                    PrecedentKind::Value => "value",
                    PrecedentKind::Spill => "spill",
                    PrecedentKind::FormulaText => "formula_text",
                    _ => "value_dependent",
                }
                .to_owned(),
            }
        })?;
        Ok(PrecedentPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            cell: cell_reference(self.engine.workbook(), cell),
            dynamic: precedents.is_dynamic(),
            total,
            offset: request.offset,
            next_offset,
            entries,
        })
    }

    /// Reads one page of the formulas that read a cell directly, from the dependency graph of the
    /// calculation installed for the current revision.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::precedents`].
    pub fn dependents(
        &self,
        request: &CellDependencyRequestDto,
    ) -> Result<DependentPageDto, InteropError> {
        let cell = self.dependency_cell(request)?;
        let dependents = self
            .engine
            .dependents(cell)
            .ok_or_else(InteropError::calculation_required)?;
        let (total, next_offset, entries) =
            page(request, dependents.dependents(), |dependent| DependentDto {
                cell: cell_reference(self.engine.workbook(), dependent.cell()),
                dynamic: dependent.is_dynamic(),
            })?;
        Ok(DependentPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            cell: cell_reference(self.engine.workbook(), cell),
            unlisted_dynamic_formulas: count_u64(dependents.unlisted_dynamic_formulas()),
            total,
            offset: request.offset,
            next_offset,
            entries,
        })
    }

    fn dependency_cell(
        &self,
        request: &CellDependencyRequestDto,
    ) -> Result<CalculationCellId, InteropError> {
        let sheet = self
            .engine
            .workbook()
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        Ok(CalculationCellId::new(
            sheet.id(),
            CellAddress::from_a1(&request.address)?,
        ))
    }
}

/// Returns the total, the next offset, and the converted entries of one page of `all`.
fn page<T, D>(
    request: &CellDependencyRequestDto,
    all: &[T],
    convert: impl Fn(&T) -> D,
) -> Result<(u64, Option<u64>, Vec<D>), InteropError> {
    let total = count_u64(all.len());
    if request.offset > total {
        return Err(InteropError::page_offset());
    }
    let limit = normalized_limit(request.limit)?;
    let entries = all
        .iter()
        .skip(usize::try_from(request.offset).unwrap_or(usize::MAX))
        .take(limit as usize)
        .map(convert)
        .collect::<Vec<_>>();
    let consumed = request.offset + count_u64(entries.len());
    Ok((total, (consumed < total).then_some(consumed), entries))
}
