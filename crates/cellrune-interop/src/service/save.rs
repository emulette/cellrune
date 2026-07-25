//! Verified package preparation and atomic destination commits.

use std::path::Path;

use cellrune::{RecalculatedWorkbook, WriteOptions, write_xlsx_draft_bytes};

use super::WorkbookSession;
use crate::convert::{write_options, write_report};
use crate::{InteropError, WriteOptionsDto, WriteReportDto};

impl WorkbookSession {
    /// Produces verified XLSX or XLSM bytes from the current calculated revision.
    ///
    /// # Errors
    ///
    /// Returns a state error when calculation is missing, or a typed write error when the result
    /// cannot be safely materialized and verified.
    pub fn save_bytes(
        &self,
        options: WriteOptionsDto,
    ) -> Result<(Vec<u8>, WriteReportDto), InteropError> {
        let prepared = self.prepare_save(options)?;
        let PreparedWorkbookSave { output, report, .. } = prepared;
        let bytes = output.into_bytes();
        Ok((bytes, report))
    }

    /// Prepares and verifies a workbook package without touching a destination path.
    ///
    /// # Errors
    ///
    /// Returns the same state and write errors as [`Self::save_path`].
    pub fn prepare_save(
        &self,
        options: WriteOptionsDto,
    ) -> Result<PreparedWorkbookSave, InteropError> {
        let calculation = self
            .current_calculation()
            .ok_or_else(InteropError::calculation_required)?;
        let converted_options = write_options(options, options.replace_existing);
        let output = write_xlsx_draft_bytes(self.engine.draft(), calculation, converted_options)?;
        let report = write_report(self.engine.workbook(), output.report());
        Ok(PreparedWorkbookSave {
            output,
            report,
            write_options: converted_options.write_options(),
        })
    }

    /// Saves a verified XLSX or XLSM package to a path.
    ///
    /// # Errors
    ///
    /// Returns a state error when calculation is missing, or a typed write error for preparation,
    /// output-kind, destination, atomic-write, or verification failures.
    pub fn save_path(
        &self,
        path: impl AsRef<Path>,
        options: WriteOptionsDto,
    ) -> Result<WriteReportDto, InteropError> {
        self.prepare_save(options)?.commit_path(path)
    }
}

/// A verified workbook package prepared for an atomic path commit.
#[derive(Debug)]
pub struct PreparedWorkbookSave {
    output: RecalculatedWorkbook,
    report: WriteReportDto,
    write_options: WriteOptions,
}

impl PreparedWorkbookSave {
    /// Returns the exact write report that a successful path commit will return.
    pub const fn report(&self) -> &WriteReportDto {
        &self.report
    }

    /// Atomically installs the prepared package at a destination path.
    ///
    /// # Errors
    ///
    /// Returns a typed output-kind, destination, or atomic-write error.
    pub fn commit_path(self, path: impl AsRef<Path>) -> Result<WriteReportDto, InteropError> {
        self.output.save_path(path, self.write_options)?;
        Ok(self.report)
    }

    /// Atomically installs the prepared package beneath an open directory capability.
    ///
    /// The destination must be exactly one relative file name. The caller can retain the same
    /// directory identity from policy validation through this commit.
    ///
    /// # Errors
    ///
    /// Returns a typed output-kind, destination, or atomic-write error.
    #[cfg(feature = "capability-fs")]
    pub fn commit_in_directory(
        self,
        directory: &cap_std::fs::Dir,
        destination: impl AsRef<Path>,
    ) -> Result<WriteReportDto, InteropError> {
        self.output
            .save_in_directory(directory, destination, self.write_options)?;
        Ok(self.report)
    }
}
