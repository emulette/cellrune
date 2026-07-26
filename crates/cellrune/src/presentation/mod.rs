//! Format-neutral workbook presentation metadata.

mod phonetic;
mod state;

pub use phonetic::{
    PhoneticAlignment, PhoneticProperties, PhoneticRun, PhoneticTextRange, PhoneticType,
    PhoneticWriteOptions, ResolvedPhoneticRun,
};
pub use state::{CellPhonetics, ColumnPhoneticVisibility, DocumentPresentation, FrozenPane};

pub(crate) use phonetic::{PhoneticAnnotation, validate_authoring_runs, validate_source_runs};
pub(crate) use state::CellPresentation;
