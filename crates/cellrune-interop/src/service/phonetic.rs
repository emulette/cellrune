//! Bounded pages of a sheet's phonetic annotations.

use super::WorkbookSession;
use super::query::normalized_limit;
use crate::convert::count_u64;
use crate::phonetic::phonetic_cell_dto;
use crate::{INTEROP_SCHEMA_VERSION, InteropError, PhoneticPageDto, SheetPageRequestDto};

impl WorkbookSession {
    /// Reads one page of the sheet's cells that carry phonetic runs, in row-major address order.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for an unknown sheet, excessive page size, or out-of-range
    /// offset.
    pub fn phonetics(
        &self,
        request: &SheetPageRequestDto,
    ) -> Result<PhoneticPageDto, InteropError> {
        let sheet = self
            .engine
            .workbook()
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let presentation = self.engine.draft().presentation();
        let total = count_u64(presentation.phonetic_cell_entries(sheet.id()).count());
        if request.offset > total {
            return Err(InteropError::page_offset());
        }
        let limit = normalized_limit(request.limit)?;
        let entries = presentation
            .phonetic_cell_entries(sheet.id())
            .skip(usize::try_from(request.offset).unwrap_or(usize::MAX))
            .take(limit as usize)
            .map(|(address, phonetics)| phonetic_cell_dto(address.to_string(), phonetics))
            .collect::<Vec<_>>();
        let consumed = request.offset + count_u64(entries.len());
        Ok(PhoneticPageDto {
            schema_version: INTEROP_SCHEMA_VERSION,
            sheet: sheet.name().as_str().to_owned(),
            total,
            offset: request.offset,
            next_offset: (consumed < total).then_some(consumed),
            entries,
        })
    }
}
