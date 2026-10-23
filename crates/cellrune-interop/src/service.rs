//! Stateful workbook facade and cohesive transport-service operations.

mod calculation;
mod edit;
mod preview;
mod query;
mod save;
mod targeted;
pub use targeted::{CompletedTargetRequest, PreparedTargetRequest};

use std::collections::BTreeMap;
use std::path::Path;

use cellrune::{
    CalculationCellId, CalculationSnapshot, CancellationToken, OpenOptions, ReadLimits,
    ReadOptions, SheetId, WorkbookCalculationSession, WorkbookDraft, open_xlsx_document_bytes,
    open_xlsx_document_path,
};

use crate::convert::{cell_reference, read_limits};
use crate::{CellReferenceDto, InteropError, ReadLimitOptionsDto};

pub use calculation::{CompletedRecalculation, PreparedRecalculation};
pub use edit::{PreparedChanges, PreparedChangesV2};
pub use preview::{
    CompletedPreview, DEFAULT_PREVIEW_PAGE_SIZE, MAX_PREVIEW_PAGE_SIZE, PreparedPreview,
};
pub use query::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, function_catalog};
pub use save::{PreparedEditsSave, PreparedWorkbookSave};

/// An owned high-level workbook editing, calculation, and save session.
#[derive(Debug)]
pub struct WorkbookSession {
    engine: WorkbookCalculationSession,
    active_calculation: Option<(u64, CancellationToken)>,
    next_calculation_id: u64,
    published_preview: Option<preview::PublishedPreview>,
    next_preview_id: u64,
    /// Names of the sheets installed edits deleted. A sheet ID is never reused, so a delta that
    /// reports a deleted sheet's cells, such as its removed results, still names the sheet.
    deleted_sheet_names: BTreeMap<SheetId, String>,
}

impl WorkbookSession {
    /// Creates a new workbook containing one visible `Sheet1`.
    pub fn create() -> Self {
        Self::from_engine(WorkbookCalculationSession::create())
    }

    /// Opens an XLSX or XLSM package from memory with default bounded read options.
    ///
    /// # Errors
    ///
    /// Returns a typed read error if the package cannot be trusted.
    pub fn open_bytes(bytes: &[u8]) -> Result<Self, InteropError> {
        Self::open_bytes_with_archive_limit(bytes, ReadLimits::default().max_archive_bytes())
    }

    /// Opens an XLSX or XLSM package from memory with a transport-selected archive byte limit.
    ///
    /// Only the archive byte limit is replaced; all other package and semantic read limits retain
    /// their core defaults.
    ///
    /// # Errors
    ///
    /// Returns a stable input error when `max_archive_bytes` is zero, or a typed read error if the
    /// package cannot be trusted within the configured limits.
    pub fn open_bytes_with_archive_limit(
        bytes: &[u8],
        max_archive_bytes: u64,
    ) -> Result<Self, InteropError> {
        let limits = ReadLimits::default()
            .with_max_archive_bytes(max_archive_bytes)
            .map_err(|_| InteropError::invalid_archive_limit())?;
        let document = open_xlsx_document_bytes(bytes, OpenOptions::new(ReadOptions::new(limits)))?;
        Ok(Self::from_engine(WorkbookCalculationSession::new(
            WorkbookDraft::from_document(&document),
        )))
    }

    /// Opens an XLSX or XLSM package from memory with the supplied read limits; each omitted
    /// limit keeps its core default.
    ///
    /// # Errors
    ///
    /// Returns `interop.limit.invalid` for a zero limit, or a typed read error if the package
    /// cannot be trusted within the limits.
    pub fn open_bytes_with_limits(
        bytes: &[u8],
        limits: ReadLimitOptionsDto,
    ) -> Result<Self, InteropError> {
        let limits = read_limits(limits)?;
        let document = open_xlsx_document_bytes(bytes, OpenOptions::new(ReadOptions::new(limits)))?;
        Ok(Self::from_engine(WorkbookCalculationSession::new(
            WorkbookDraft::from_document(&document),
        )))
    }

    /// Opens an XLSX or XLSM package from a path with the supplied read limits; each omitted
    /// limit keeps its core default.
    ///
    /// # Errors
    ///
    /// Returns `interop.limit.invalid` for a zero limit, or a typed read error if the file cannot
    /// be opened or trusted within the limits.
    pub fn open_path_with_limits(
        path: impl AsRef<Path>,
        limits: ReadLimitOptionsDto,
    ) -> Result<Self, InteropError> {
        let limits = read_limits(limits)?;
        let document = open_xlsx_document_path(path, OpenOptions::new(ReadOptions::new(limits)))?;
        Ok(Self::from_engine(WorkbookCalculationSession::new(
            WorkbookDraft::from_document(&document),
        )))
    }

    /// Opens an XLSX or XLSM package from a path with default bounded read options.
    ///
    /// # Errors
    ///
    /// Returns a typed read error if the file cannot be opened or trusted.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, InteropError> {
        let document = open_xlsx_document_path(path, OpenOptions::default())?;
        Ok(Self::from_engine(WorkbookCalculationSession::new(
            WorkbookDraft::from_document(&document),
        )))
    }

    fn from_engine(engine: WorkbookCalculationSession) -> Self {
        Self {
            engine,
            active_calculation: None,
            next_calculation_id: 1,
            published_preview: None,
            next_preview_id: 1,
            deleted_sheet_names: BTreeMap::new(),
        }
    }

    /// Returns the current sheets' IDs and names, to compare once an edit is installed.
    fn sheet_names(&self) -> Vec<(SheetId, String)> {
        self.engine
            .workbook()
            .sheets()
            .iter()
            .map(|sheet| (sheet.id(), sheet.name().as_str().to_owned()))
            .collect()
    }

    /// Remembers the names of the `previous` sheets the installed workbook no longer has.
    fn remember_deleted_sheets(&mut self, previous: Vec<(SheetId, String)>) {
        for (sheet_id, name) in previous {
            if self.engine.workbook().sheet_by_id(sheet_id).is_none() {
                self.deleted_sheet_names.insert(sheet_id, name);
            }
        }
    }

    /// Names a cell by its current sheet or, for a deleted sheet, by the name it last had.
    fn result_cell_reference(&self, id: CalculationCellId) -> CellReferenceDto {
        let mut reference = cell_reference(self.engine.workbook(), id);
        if let Some(name) = self.deleted_sheet_names.get(&id.sheet_id())
            && self.engine.workbook().sheet_by_id(id.sheet_id()).is_none()
        {
            reference.sheet_name.clone_from(name);
        }
        reference
    }

    fn current_calculation(&self) -> Option<&CalculationSnapshot> {
        self.engine.calculation().filter(|calculation| {
            calculation.source_revision() == self.engine.workbook().semantic_revision()
        })
    }

    fn invalidate_preview_for_mutation(&mut self) {
        if let Some(mut preview) = self.published_preview.take() {
            preview.discard();
        }
    }
}

impl Default for WorkbookSession {
    fn default() -> Self {
        Self::create()
    }
}

impl Drop for WorkbookSession {
    fn drop(&mut self) {
        if let Some((_, token)) = self.active_calculation.take() {
            token.cancel();
        }
        if let Some(mut preview) = self.published_preview.take() {
            preview.discard();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{CalculationOptionsDto, InteropErrorKind, WorkbookSession, WriteOptionsDto};

    fn generated_workbook_bytes() -> Vec<u8> {
        let mut session = WorkbookSession::create();
        session
            .calculate(CalculationOptionsDto::default())
            .expect("generated workbook must calculate");
        session
            .save_bytes(WriteOptionsDto::default())
            .expect("generated workbook must save")
            .0
    }

    #[test]
    fn custom_archive_limit_is_applied_without_changing_default_open() {
        let bytes = generated_workbook_bytes();
        let exact_limit = bytes.len() as u64;

        WorkbookSession::open_bytes(&bytes).expect("default open must remain supported");
        WorkbookSession::open_bytes_with_archive_limit(&bytes, exact_limit)
            .expect("the exact custom limit must be accepted");
        let error = WorkbookSession::open_bytes_with_archive_limit(&bytes, exact_limit - 1)
            .expect_err("a smaller custom limit must be enforced by the parser");

        assert_eq!(error.kind(), InteropErrorKind::Read);
        assert_eq!(error.code(), "xlsx.archive_too_large");
    }

    #[test]
    fn zero_archive_limit_is_a_stable_transport_error() {
        let error = WorkbookSession::open_bytes_with_archive_limit(&[], 0)
            .expect_err("a zero archive limit must fail before parsing");

        assert_eq!(error.kind(), InteropErrorKind::Input);
        assert_eq!(error.code(), "interop.workbook.archive_limit_invalid");
    }
}
