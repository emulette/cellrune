use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use super::package::{PartPath, WorkbookPackageKind};
use super::reader::{PresentationCapture, read_xlsx_with_identity};
use super::write::PreservedPackage;
use super::{PackageSummary, ReadOptions, XlsxErrorCode, XlsxReadError};
use crate::{
    DocumentPresentation, InputHash, SheetId, SourceId, TableId, WorkbookSnapshot,
    WorkbookSourceKind,
};

/// Open XML spreadsheet package kind retained by a writable document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum XlsxDocumentKind {
    /// Ordinary macro-free XLSX package.
    Xlsx,
    /// Macro-enabled XLSM package whose VBA content is preserved but never executed.
    Xlsm,
}

/// Options for opening a package-backed writable workbook document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenOptions {
    read_options: ReadOptions,
}

impl OpenOptions {
    /// Constructs document options from bounded XLSX read options.
    pub const fn new(read_options: ReadOptions) -> Self {
        Self { read_options }
    }

    /// Returns the bounded read options used while opening the document.
    pub const fn read_options(self) -> ReadOptions {
        self.read_options
    }
}

/// An immutable workbook snapshot paired with its exact preserved XLSX or XLSM package.
#[derive(Clone)]
pub struct XlsxDocument {
    workbook: WorkbookSnapshot,
    presentation: DocumentPresentation,
    read_options: ReadOptions,
    package_summary: PackageSummary,
    workbook_part: PartPath,
    worksheet_parts: BTreeMap<SheetId, PartPath>,
    table_parts: BTreeMap<TableId, PartPath>,
    package: PreservedPackage,
}

impl XlsxDocument {
    /// Returns the immutable format-neutral workbook model.
    pub const fn workbook(&self) -> &WorkbookSnapshot {
        &self.workbook
    }

    /// Returns immutable presentation metadata kept outside calculation semantics.
    pub const fn presentation(&self) -> &DocumentPresentation {
        &self.presentation
    }

    /// Returns the exact SHA-256 identity of the opened archive bytes.
    pub const fn input_hash(&self) -> InputHash {
        self.package.input_hash()
    }

    /// Returns whether the preserved package is XLSX or macro-enabled XLSM.
    pub const fn kind(&self) -> XlsxDocumentKind {
        self.package.kind()
    }

    /// Returns the current semantic workbook revision.
    pub const fn semantic_revision(&self) -> u64 {
        self.workbook.semantic_revision()
    }

    /// Returns the current presentation-only revision.
    pub const fn presentation_revision(&self) -> u64 {
        self.presentation.revision()
    }

    pub(in crate::xlsx) const fn read_options(&self) -> ReadOptions {
        self.read_options
    }

    /// Returns the bounded package summary discovered while opening.
    pub const fn package_summary(&self) -> &PackageSummary {
        &self.package_summary
    }

    /// Returns the relationship-selected workbook package part.
    pub fn workbook_part(&self) -> SourceId {
        self.workbook_part.source_id()
    }

    /// Returns the worksheet package part associated with a stable sheet ID.
    pub fn worksheet_part(&self, sheet_id: SheetId) -> Option<SourceId> {
        self.worksheet_parts.get(&sheet_id).map(PartPath::source_id)
    }

    /// Returns the table-definition package part associated with a stable table ID.
    pub fn table_part(&self, table_id: TableId) -> Option<SourceId> {
        self.table_parts.get(&table_id).map(PartPath::source_id)
    }

    pub(crate) const fn preserved_package(&self) -> &PreservedPackage {
        &self.package
    }

    pub(in crate::xlsx) const fn workbook_part_path(&self) -> &PartPath {
        &self.workbook_part
    }

    pub(in crate::xlsx) fn worksheet_part_path(&self, sheet_id: SheetId) -> Option<&PartPath> {
        self.worksheet_parts.get(&sheet_id)
    }
}

impl fmt::Debug for XlsxDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("XlsxDocument")
            .field("input_hash", &self.input_hash())
            .field("kind", &self.kind())
            .field("semantic_revision", &self.semantic_revision())
            .field("presentation_revision", &self.presentation_revision())
            .field("package_summary", &self.package_summary)
            .field("workbook", &self.workbook)
            .finish_non_exhaustive()
    }
}

/// Opens a bounded package-backed workbook from a seekable reader.
///
/// The exact input archive is retained privately for later round-trip writing. No host path is
/// retained and no macro or external relationship is executed.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the input cannot be buffered within the configured archive
/// limit or cannot be parsed into a trustworthy workbook.
pub fn open_xlsx_document<R: Read + Seek + Send + 'static>(
    reader: R,
    options: OpenOptions,
) -> Result<XlsxDocument, XlsxReadError> {
    open_with_source(reader, options, WorkbookSourceKind::Reader)
}

/// Opens a bounded package-backed workbook from in-memory XLSX or XLSM bytes.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when `bytes` exceed the configured archive limit or do not contain
/// a trustworthy workbook.
pub fn open_xlsx_document_bytes(
    bytes: &[u8],
    options: OpenOptions,
) -> Result<XlsxDocument, XlsxReadError> {
    open_with_source(
        Cursor::new(bytes.to_vec()),
        options,
        WorkbookSourceKind::Bytes,
    )
}

/// Opens a bounded package-backed workbook from a filesystem path without retaining the host path.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the path cannot be opened or the file cannot be parsed into a
/// trustworthy workbook.
pub fn open_xlsx_document_path(
    path: impl AsRef<Path>,
    options: OpenOptions,
) -> Result<XlsxDocument, XlsxReadError> {
    let reader = File::open(path)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::Io).with_cause(error))?;
    open_with_source(reader, options, WorkbookSourceKind::Path)
}

fn open_with_source<R: Read + Seek>(
    reader: R,
    options: OpenOptions,
    source_kind: WorkbookSourceKind,
) -> Result<XlsxDocument, XlsxReadError> {
    let bytes = read_bounded_archive(reader, options.read_options())?;
    let input_hash = InputHash::for_bytes(&bytes);
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let read = read_xlsx_with_identity(
        Cursor::new(Arc::clone(&bytes)),
        options.read_options(),
        source_kind,
        Some(input_hash),
        PresentationCapture::Document,
    )?;
    let kind = match read.package_kind {
        WorkbookPackageKind::Xlsx => XlsxDocumentKind::Xlsx,
        WorkbookPackageKind::Xlsm => XlsxDocumentKind::Xlsm,
    };
    Ok(XlsxDocument {
        workbook: read.workbook,
        presentation: read.presentation,
        read_options: options.read_options(),
        package_summary: read.package_summary,
        workbook_part: read.workbook_part,
        worksheet_parts: read.worksheet_parts,
        table_parts: read.table_parts,
        package: PreservedPackage::new(bytes, input_hash, kind),
    })
}

fn read_bounded_archive<R: Read + Seek>(
    mut reader: R,
    options: ReadOptions,
) -> Result<Vec<u8>, XlsxReadError> {
    let maximum = options.limits().max_archive_bytes();
    let byte_length = reader
        .seek(SeekFrom::End(0))
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::Io).with_cause(error))?;
    if byte_length > maximum {
        return Err(
            XlsxReadError::new(XlsxErrorCode::ArchiveTooLarge).with_detail(byte_length.to_string())
        );
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::Io).with_cause(error))?;
    let capacity = usize::try_from(byte_length)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::ArchiveTooLarge).with_cause(error))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::ArchiveTooLarge).with_cause(error))?;
    reader
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::Io).with_cause(error))?;
    if bytes.len() as u64 > maximum {
        return Err(
            XlsxReadError::new(XlsxErrorCode::ArchiveTooLarge).with_detail(bytes.len().to_string())
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Seek, SeekFrom};

    use super::read_bounded_archive;
    use crate::{ReadLimits, ReadOptions, XlsxErrorCode};

    struct ReportedLengthReader {
        contents: Cursor<Vec<u8>>,
        reported_length: u64,
    }

    impl ReportedLengthReader {
        fn new(contents: Vec<u8>, reported_length: u64) -> Self {
            Self {
                contents: Cursor::new(contents),
                reported_length,
            }
        }
    }

    impl Read for ReportedLengthReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.contents.read(buffer)
        }
    }

    impl Seek for ReportedLengthReader {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            if position == SeekFrom::End(0) {
                return Ok(self.reported_length);
            }
            self.contents.seek(position)
        }
    }

    #[test]
    fn archive_budget_accepts_the_exact_boundary() {
        let options = options_with_archive_limit(4);
        let bytes =
            read_bounded_archive(Cursor::new(vec![1, 2, 3, 4]), options).expect("exact limit");
        assert_eq!(bytes, [1, 2, 3, 4]);
    }

    #[test]
    fn archive_budget_rejects_a_reported_length_above_the_boundary() {
        let reader = ReportedLengthReader::new(vec![1, 2, 3, 4], 5);
        let error = read_bounded_archive(reader, options_with_archive_limit(4))
            .expect_err("reported length must be bounded before reading");
        assert_eq!(error.code(), XlsxErrorCode::ArchiveTooLarge);
        assert_eq!(error.detail(), Some("5"));
    }

    #[test]
    fn archive_budget_rechecks_the_bytes_read_from_a_changing_source() {
        let reader = ReportedLengthReader::new(vec![1, 2, 3, 4, 5], 4);
        let error = read_bounded_archive(reader, options_with_archive_limit(4))
            .expect_err("bytes read must be checked independently");
        assert_eq!(error.code(), XlsxErrorCode::ArchiveTooLarge);
        assert_eq!(error.detail(), Some("5"));
    }

    fn options_with_archive_limit(limit: u64) -> ReadOptions {
        let limits = ReadLimits::default()
            .with_max_archive_bytes(limit)
            .expect("positive archive limit");
        ReadOptions::new(limits)
    }
}
