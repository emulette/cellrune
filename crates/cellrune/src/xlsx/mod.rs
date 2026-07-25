#![forbid(unsafe_code)]

mod document;
mod error;
mod options;
mod package;
mod reader;
mod write;
mod xml;

pub use document::{
    OpenOptions, XlsxDocument, XlsxDocumentKind, open_xlsx_document, open_xlsx_document_bytes,
    open_xlsx_document_path,
};
pub use error::{ReadOptionsError, XlsxErrorCode, XlsxReadError};
pub use options::{ReadLimits, ReadOptions};
pub use package::{PackageSummary, inspect_package};
pub use reader::{read_xlsx, read_xlsx_bytes, read_xlsx_path};
pub use write::{
    RecalculatedWorkbook, RecalculationWriteOptions, RecalculationWritePolicy, WriteLimits,
    WriteOptions, WriteOptionsError, WriteProvenance, WriteReport, XlsxWriteError,
    XlsxWriteErrorCode, write_preserved_xlsx_bytes, write_recalculated_xlsx,
    write_recalculated_xlsx_bytes, write_recalculated_xlsx_path, write_xlsx_draft,
    write_xlsx_draft_bytes, write_xlsx_draft_path,
};
