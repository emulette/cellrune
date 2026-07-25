//! Stateful workbook facade and cohesive transport-service operations.

mod calculation;
mod edit;
mod query;
mod save;

use std::path::Path;

use cellrune::{
    CalculationSnapshot, CancellationToken, OpenOptions, ReadLimits, ReadOptions,
    WorkbookCalculationSession, WorkbookDraft, open_xlsx_document_bytes, open_xlsx_document_path,
};

use crate::InteropError;

pub use calculation::{CompletedRecalculation, PreparedRecalculation};
pub use edit::PreparedChanges;
pub use query::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, function_catalog};
pub use save::PreparedWorkbookSave;

/// An owned high-level workbook editing, calculation, and save session.
#[derive(Debug)]
pub struct WorkbookSession {
    engine: WorkbookCalculationSession,
    active_calculation: Option<(u64, CancellationToken)>,
    next_calculation_id: u64,
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
        }
    }

    fn current_calculation(&self) -> Option<&CalculationSnapshot> {
        self.engine.calculation().filter(|calculation| {
            calculation.source_revision() == self.engine.workbook().semantic_revision()
        })
    }
}

impl Default for WorkbookSession {
    fn default() -> Self {
        Self::create()
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
