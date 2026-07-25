use std::io::Write;
use std::path::Path;

use super::canonical::write_canonical_draft;
use super::document_authoring::write_document_draft;
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteReport, XlsxWriteError,
    XlsxWriteErrorCode,
};
use crate::{CalculationSnapshot, WorkbookDraft};

/// Calculates materialization output for a draft and returns a verified XLSX or XLSM archive.
///
/// New drafts use the canonical XLSX producer. Drafts created from an [`crate::XlsxDocument`]
/// preserve that package and its kind.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when the calculation is stale, incomplete under the selected
/// policy, cannot be represented safely, or the completed package fails verification.
pub fn write_xlsx_draft_bytes(
    draft: &WorkbookDraft,
    calculation: &CalculationSnapshot,
    options: RecalculationWriteOptions,
) -> Result<RecalculatedWorkbook, XlsxWriteError> {
    if draft.is_document_backed() {
        return write_document_draft(draft, calculation, options);
    }
    write_canonical_draft(draft, calculation, options)
}

/// Writes a fully prepared draft archive to an output sink.
///
/// The complete package is prepared and verified before the supplied writer is touched.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] for preparation failures or when the output cannot be written
/// and flushed.
pub fn write_xlsx_draft<W: Write>(
    draft: &WorkbookDraft,
    calculation: &CalculationSnapshot,
    writer: &mut W,
    options: RecalculationWriteOptions,
) -> Result<WriteReport, XlsxWriteError> {
    let output = write_xlsx_draft_bytes(draft, calculation, options)?;
    writer.write_all(output.bytes()).map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    let (_, report) = output.into_parts();
    Ok(report)
}

/// Saves a verified draft package to a new path or explicitly replaces the destination.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when package preparation fails, the destination kind is wrong,
/// the destination exists without replacement permission, or the atomic Save As operation fails.
pub fn write_xlsx_draft_path(
    draft: &WorkbookDraft,
    calculation: &CalculationSnapshot,
    path: impl AsRef<Path>,
    options: RecalculationWriteOptions,
) -> Result<WriteReport, XlsxWriteError> {
    let output = write_xlsx_draft_bytes(draft, calculation, options)?;
    output.save_path(path, options.write_options())?;
    let (_, report) = output.into_parts();
    Ok(report)
}

fn io_error(error: std::io::Error) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}
