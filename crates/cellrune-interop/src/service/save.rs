//! Verified package preparation and atomic destination commits.

use std::path::Path;

use cellrune::{
    EditedWorkbook, RecalculatedWorkbook, WriteOptions, write_xlsx_draft_bytes,
    write_xlsx_draft_edits_bytes,
};

use super::WorkbookSession;
use crate::convert::{edit_write_report, write_options, write_report};
use crate::{
    EditWriteOptionsDto, EditWriteReportDto, InteropError, WriteOptionsDto, WriteReportDto,
};

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
        let output = write_xlsx_draft_bytes(self.engine.draft(), calculation, converted_options)
            .map_err(|error| InteropError::from_write(error, self.engine.workbook()))?;
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

    /// Produces the session's source package with its edits written in, without calculating.
    ///
    /// Formulas the edits can affect lose their saved results and only then is the spreadsheet
    /// app asked to recalculate on open; every other saved result and untouched part keeps its
    /// source bytes. No calculation is required, and formulas CellRune cannot calculate do not
    /// block the save.
    ///
    /// # Errors
    ///
    /// Returns `xlsx.write.invalid_package_plan` for a session not opened from a package, or a
    /// typed write error when an edit cannot be represented safely or the output fails
    /// verification.
    pub fn save_edits_bytes(&self) -> Result<(Vec<u8>, EditWriteReportDto), InteropError> {
        let PreparedEditsSave { output, report, .. } =
            self.prepare_edits_save(EditWriteOptionsDto::default())?;
        Ok((output.into_bytes(), report))
    }

    /// Prepares and verifies the edited package without touching a destination path.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::save_edits_bytes`].
    pub fn prepare_edits_save(
        &self,
        options: EditWriteOptionsDto,
    ) -> Result<PreparedEditsSave, InteropError> {
        let write_options = WriteOptions::default().with_replace_existing(options.replace_existing);
        let output = write_xlsx_draft_edits_bytes(self.engine.draft(), write_options)?;
        let report = edit_write_report(self.engine.workbook(), output.report());
        Ok(PreparedEditsSave {
            output,
            report,
            write_options,
        })
    }

    /// Saves the edited package to a path.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Self::save_edits_bytes`] or a typed output-kind, destination, or
    /// atomic-write error.
    pub fn save_edits_path(
        &self,
        path: impl AsRef<Path>,
        options: EditWriteOptionsDto,
    ) -> Result<EditWriteReportDto, InteropError> {
        self.prepare_edits_save(options)?.commit_path(path)
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

/// A verified edited package prepared for an atomic path commit.
#[derive(Debug)]
pub struct PreparedEditsSave {
    output: EditedWorkbook,
    report: EditWriteReportDto,
    write_options: WriteOptions,
}

impl PreparedEditsSave {
    /// Returns the exact edit report that a successful path commit will return.
    pub const fn report(&self) -> &EditWriteReportDto {
        &self.report
    }

    /// Atomically installs the prepared package at a destination path.
    ///
    /// # Errors
    ///
    /// Returns a typed output-kind, destination, or atomic-write error.
    pub fn commit_path(self, path: impl AsRef<Path>) -> Result<EditWriteReportDto, InteropError> {
        self.output.save_path(path, self.write_options)?;
        Ok(self.report)
    }

    /// Atomically installs the prepared package beneath an open directory capability.
    ///
    /// # Errors
    ///
    /// Returns a typed output-kind, destination, or atomic-write error.
    #[cfg(feature = "capability-fs")]
    pub fn commit_in_directory(
        self,
        directory: &cap_std::fs::Dir,
        destination: impl AsRef<Path>,
    ) -> Result<EditWriteReportDto, InteropError> {
        self.output
            .save_in_directory(directory, destination, self.write_options)?;
        Ok(self.report)
    }
}
