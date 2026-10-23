//! Format-neutral workbook presentation metadata.

mod format;
mod phonetic;
mod sheet_format;
mod state;

pub use format::{
    Alignment, Border, BorderEdge, BorderStyle, CellFormat, CellFormatEdit, Color, ColorKind, Fill,
    FillPattern, Font, FontUnderline, HorizontalAlignment, PatternFill, VerticalAlignment,
};
pub use sheet_format::ColumnWidth;

pub use phonetic::{
    PhoneticAlignment, PhoneticProperties, PhoneticRun, PhoneticTextRange, PhoneticType,
    PhoneticWriteOptions, ResolvedPhoneticRun,
};
pub use state::{CellPhonetics, ColumnPhoneticVisibility, DocumentPresentation, FrozenPane};

pub(crate) use format::VERTICAL_TEXT_ROTATION;
pub(crate) use phonetic::{PhoneticAnnotation, validate_authoring_runs, validate_source_runs};
pub(crate) use sheet_format::canonical_default_format;
pub(crate) use state::CellPresentation;
