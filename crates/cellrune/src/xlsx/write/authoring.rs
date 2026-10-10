use std::io::Write;
use std::path::Path;

use super::canonical::write_canonical_draft;
use super::document_authoring::write_document_draft;
use super::document_edits::write_document_edits;
use super::{
    EditWriteReport, EditedWorkbook, RecalculatedWorkbook, RecalculationWriteOptions, WriteOptions,
    WriteReport, XlsxWriteError, XlsxWriteErrorCode,
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

/// Writes the edits of a draft opened from a document into its preserved package without
/// calculating, and returns the verified archive.
///
/// Formulas the edits can affect have their saved results removed and the output asks the host to
/// recalculate on open; every other saved result and every untouched part keep their source bytes.
/// Formulas whose references are only known after evaluation, such as INDIRECT, OFFSET, or a
/// formula that cannot be parsed, count as affected whenever anything changed.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when the draft was not opened from a document, an edit cannot be
/// represented safely, a write limit is exceeded, or the completed package fails verification.
pub fn write_xlsx_draft_edits_bytes(
    draft: &WorkbookDraft,
    options: WriteOptions,
) -> Result<EditedWorkbook, XlsxWriteError> {
    write_document_edits(draft, options)
}

/// Writes a draft's edits, prepared and verified as by [`write_xlsx_draft_edits_bytes`], to an
/// output sink.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] for preparation failures or when the output cannot be written
/// and flushed.
pub fn write_xlsx_draft_edits<W: Write>(
    draft: &WorkbookDraft,
    writer: &mut W,
    options: WriteOptions,
) -> Result<EditWriteReport, XlsxWriteError> {
    let output = write_xlsx_draft_edits_bytes(draft, options)?;
    writer.write_all(output.bytes()).map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    let (_, report) = output.into_parts();
    Ok(report)
}

/// Saves a draft's edits, prepared and verified as by [`write_xlsx_draft_edits_bytes`], to a new
/// path or explicitly replaces the destination.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when package preparation fails, the destination kind is wrong,
/// the destination exists without replacement permission, or the atomic Save As operation fails.
pub fn write_xlsx_draft_edits_path(
    draft: &WorkbookDraft,
    path: impl AsRef<Path>,
    options: WriteOptions,
) -> Result<EditWriteReport, XlsxWriteError> {
    let output = write_xlsx_draft_edits_bytes(draft, options)?;
    output.save_path(path, options)?;
    let (_, report) = output.into_parts();
    Ok(report)
}

fn io_error(error: std::io::Error) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}
