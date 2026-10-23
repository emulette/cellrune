use std::error::Error;
use std::fmt;

use cellrune::{
    ApplyChangesError, DefinedNameAnalysisError, DefinedNameAnalysisErrorKind, SessionError,
    ValidationError, WorkbookSnapshot, XlsxReadError, XlsxWriteError,
};
use schemars::JsonSchema;

use crate::CellReferenceDto;
use crate::convert::cell_reference;
use serde::{Deserialize, Serialize};

const MESSAGE_SHEET_NOT_FOUND: &str = "workbook does not contain the requested sheet";
const MESSAGE_PAGE_LIMIT: &str = "page size exceeds the interop limit";
const MESSAGE_PAGE_OFFSET: &str = "page offset exceeds the requested range";
const MESSAGE_PAGE_OFFSET_INVALID: &str = "page offset must be a non-negative integer";
const MESSAGE_PAGE_LIMIT_INVALID: &str =
    "page limit must be a non-negative integer within the unsigned 32-bit range";
const MESSAGE_CALCULATION_REQUIRED: &str = "calculate the current workbook revision before saving";
const MESSAGE_SESSION_CLOSED: &str = "workbook session is closed";
const MESSAGE_SESSION_BUSY: &str = "workbook session state is unavailable";
const MESSAGE_SESSION_POISONED: &str = "workbook session state is unusable after a native panic";
const MESSAGE_EXCEL_ERROR: &str = "Excel error value is not recognized";
const MESSAGE_INVALID_CHANGE: &str = "workbook change failed transport validation";
const MESSAGE_SHEET_CREATION_FAILED: &str = "sheet identifier allocation failed";
const MESSAGE_REQUEST_ID_EXHAUSTED: &str = "calculation request identifier is exhausted";
const MESSAGE_CALCULATION_CANCELLED: &str = "calculation request was cancelled or superseded";
const MESSAGE_EDIT_CANCELLED: &str = "workbook edit was cancelled before installation";
const MESSAGE_CHANGE_PAYLOAD_INVALID: &str = "edit batch payload is invalid";
const MESSAGE_REVISION_OR_CURSOR_INVALID: &str =
    "revision or cursor must be an unsigned 64-bit integer";
const MESSAGE_RECALCULATION_MODE_INVALID: &str =
    "recalculation mode must be auto, incremental, or full";
const MESSAGE_ARITHMETIC_SEMANTICS_INVALID: &str =
    "arithmetic semantics must be excel_near_zero or ieee_754";
const MESSAGE_FINANCIAL_SOLVER_SEMANTICS_INVALID: &str =
    "financial solver semantics must be excel_iteration_budget or extended_search";
const MESSAGE_LOCALE_PROFILE_INVALID: &str =
    "locale profile must be an object whose locale and display_language are en-US or ko-KR";
const MESSAGE_DISPLAY_FORMAT_REQUEST_INVALID: &str = "display format request is invalid";
const MESSAGE_NUMBER_INVALID: &str =
    "number must be an integer or floating-point value; booleans are not accepted";
const MESSAGE_ARCHIVE_LIMIT_INVALID: &str = "archive byte limit must be greater than zero";
const MESSAGE_LIMIT_INVALID: &str = "resource limit must be greater than zero";
const MESSAGE_LIMIT_PAYLOAD_INVALID: &str =
    "resource limits must map known limit names to positive integers";
const MESSAGE_DEFINED_NAME_SHEET_IDENTITY: &str =
    "defined-name analysis returned an unknown sheet identity";
const MESSAGE_PREVIEW_NOT_FOUND: &str = "workbook preview is not published in this session";
const MESSAGE_PREVIEW_CURSOR_INVALID: &str = "preview cursor does not belong to this preview";
const MESSAGE_PREVIEW_RESPONSE_LIMIT: &str =
    "preview response cannot contain one complete detail item within the byte limit";
const MESSAGE_PREVIEW_ID_EXHAUSTED: &str = "workbook preview identifier is exhausted";
const MESSAGE_PREVIEW_SECTION_INVALID: &str = "preview detail section is not recognized";
const MESSAGE_SERIALIZATION: &str = "interop DTO serialization failed";
pub(crate) const DETAIL_FONT_UNDERLINE: &str = "font underline is not recognized";
pub(crate) const DETAIL_FILL_PATTERN: &str = "fill pattern is not recognized";
pub(crate) const DETAIL_BORDER_STYLE: &str = "border style is not recognized";
pub(crate) const DETAIL_HORIZONTAL_ALIGNMENT: &str = "horizontal alignment is not recognized";
pub(crate) const DETAIL_VERTICAL_ALIGNMENT: &str = "vertical alignment is not recognized";
pub(crate) const DETAIL_COLOR_ARGB: &str = "argb color must be eight hexadecimal digits";
pub(crate) const DETAIL_PHONETIC_TYPE: &str = "phonetic type is not recognized";
pub(crate) const DETAIL_PHONETIC_ALIGNMENT: &str = "phonetic alignment is not recognized";

/// Broad error boundary used by all language bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteropErrorKind {
    /// Transport-facing input is invalid.
    Input,
    /// A format-neutral workbook invariant was rejected.
    Validation,
    /// An XLSX or XLSM package could not be read.
    Read,
    /// A verified package could not be written.
    Write,
    /// The requested session operation is not valid in its current state.
    State,
}

/// Optional structured context carried with a stable interop error.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorDetails {
    /// Stable lower-layer code, when distinct from the interop code.
    pub source_code: Option<String>,
    /// Package source identifier, when available.
    pub source_id: Option<String>,
    /// Source-specific detail that does not change the code.
    pub detail: Option<String>,
    /// Formula cells the failure concerns: for `xlsx.write.incomplete_calculation`, the first 64
    /// formulas whose results were unavailable. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<CellReferenceDto>,
    /// Figures of a failure that exceeded a resource limit. Omitted when the failure reports none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<LimitDiagnosticsDto>,
}

/// Figures a resource-limit failure reports; each is omitted when the failure does not measure it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LimitDiagnosticsDto {
    /// Read limit a `xlsx.too_many_phonetic_runs` failure exceeded: `max_phonetic_runs_per_item`
    /// or `max_total_phonetic_runs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Measured value that exceeded the read limit the error code names, in that limit's unit: bytes,
    /// a count, a nesting depth, or a compression ratio rounded up. A declared size or count reports
    /// the declared value, and a running total the value counted when reading stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<u64>,
    /// Requested cells counted when targeted calculation passed its result-cell limit: a lower
    /// bound on the distinct cells the request names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_cell_count: Option<u64>,
    /// Evaluator invocations performed before targeted calculation stopped at its work limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluated_count: Option<u64>,
}

impl LimitDiagnosticsDto {
    /// Returns the figures, or `None` when there are none to report.
    fn reported(self) -> Option<Self> {
        (self != Self::default()).then_some(self)
    }
}

/// Stable error object shared by Rust, Python, Node.js, and MCP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InteropError {
    kind: InteropErrorKind,
    code: String,
    message: String,
    details: Box<ErrorDetails>,
}

impl InteropError {
    pub(crate) fn target_limit() -> Self {
        let code = cellrune::TargetCalculationErrorCode::TargetLimitExceeded;
        Self::input(code.as_str(), code.message())
    }

    /// Creates the stable invalid partial-calculation payload error used by native bindings.
    pub fn invalid_target_payload(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.calculation.target.invalid_payload",
            "targeted calculation payload is invalid",
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }
    /// Returns the broad error boundary.
    pub const fn kind(&self) -> InteropErrorKind {
        self.kind
    }

    /// Returns the stable dotted error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the stable human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns structured error context.
    pub const fn details(&self) -> &ErrorDetails {
        &self.details
    }

    pub(crate) fn input(code: &'static str, message: &'static str) -> Self {
        Self::new(
            InteropErrorKind::Input,
            code,
            message,
            ErrorDetails::default(),
        )
    }

    pub(crate) fn state(code: &'static str, message: &'static str) -> Self {
        Self::new(
            InteropErrorKind::State,
            code,
            message,
            ErrorDetails::default(),
        )
    }

    /// Creates the stable closed-session error used by binding lifecycle guards.
    pub fn session_closed() -> Self {
        Self::state("interop.session.closed", MESSAGE_SESSION_CLOSED)
    }

    /// Creates the stable lock-state error used when concurrent work holds a shared session.
    pub fn session_busy() -> Self {
        Self::state("interop.session.unavailable", MESSAGE_SESSION_BUSY)
    }

    /// Creates the stable lock-state error used when a panic poisoned a shared session.
    pub fn session_poisoned() -> Self {
        Self::state("interop.session.poisoned", MESSAGE_SESSION_POISONED)
    }

    pub(crate) fn sheet_not_found() -> Self {
        Self::input("interop.sheet.not_found", MESSAGE_SHEET_NOT_FOUND)
    }

    pub(crate) fn page_limit() -> Self {
        Self::input("interop.page.limit_exceeded", MESSAGE_PAGE_LIMIT)
    }

    pub(crate) fn page_offset() -> Self {
        Self::input("interop.page.offset_out_of_range", MESSAGE_PAGE_OFFSET)
    }

    /// Creates the stable invalid page-offset error used by typed bindings.
    pub fn invalid_page_offset() -> Self {
        Self::input("interop.page.offset_invalid", MESSAGE_PAGE_OFFSET_INVALID)
    }

    /// Creates the stable invalid page-limit error used by typed bindings.
    pub fn invalid_page_limit() -> Self {
        Self::input("interop.page.limit_invalid", MESSAGE_PAGE_LIMIT_INVALID)
    }

    pub(crate) fn calculation_required() -> Self {
        Self::state("interop.calculation.required", MESSAGE_CALCULATION_REQUIRED)
    }

    pub(crate) fn excel_error(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.value.excel_error_invalid",
            MESSAGE_EXCEL_ERROR,
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }

    pub(crate) fn invalid_change(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.change.invalid",
            MESSAGE_INVALID_CHANGE,
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }

    pub(crate) fn sheet_creation_failed() -> Self {
        Self::state("interop.sheet.id_exhausted", MESSAGE_SHEET_CREATION_FAILED)
    }

    pub(crate) fn session_request_id_exhausted() -> Self {
        Self::state(
            "interop.calculation.request_id_exhausted",
            MESSAGE_REQUEST_ID_EXHAUSTED,
        )
    }

    pub(crate) fn calculation_cancelled() -> Self {
        Self::state("session.cancelled", MESSAGE_CALCULATION_CANCELLED)
    }

    pub(crate) fn edit_cancelled() -> Self {
        Self::state("session.cancelled", MESSAGE_EDIT_CANCELLED)
    }

    /// Creates the stable malformed typed-change payload error used by bindings.
    pub fn invalid_change_payload(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.change.payload_invalid",
            MESSAGE_CHANGE_PAYLOAD_INVALID,
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }

    /// Creates the stable invalid revision or delta-cursor error used by bindings.
    pub fn invalid_revision_or_cursor() -> Self {
        Self::input(
            "interop.session.revision_or_cursor_invalid",
            MESSAGE_REVISION_OR_CURSOR_INVALID,
        )
    }

    /// Creates the stable invalid recalculation-mode error used by bindings.
    pub fn invalid_recalculation_mode() -> Self {
        Self::input(
            "interop.calculation.mode_invalid",
            MESSAGE_RECALCULATION_MODE_INVALID,
        )
    }

    /// Creates the stable invalid arithmetic-semantics error used by typed bindings.
    pub fn invalid_arithmetic_semantics() -> Self {
        Self::input(
            "interop.calculation.arithmetic_semantics_invalid",
            MESSAGE_ARITHMETIC_SEMANTICS_INVALID,
        )
    }

    /// Creates the stable invalid financial-solver-semantics error used by typed bindings.
    pub fn invalid_financial_solver_semantics() -> Self {
        Self::input(
            "interop.calculation.financial_solver_semantics_invalid",
            MESSAGE_FINANCIAL_SOLVER_SEMANTICS_INVALID,
        )
    }

    /// Creates the stable error for a display-format request a binding could not read.
    pub fn invalid_display_format_request(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.display_format.request_invalid",
            MESSAGE_DISPLAY_FORMAT_REQUEST_INVALID,
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }

    /// Creates the stable invalid locale-profile error used by typed bindings.
    pub fn invalid_locale_profile() -> Self {
        Self::input(
            "interop.calculation.locale_profile_invalid",
            MESSAGE_LOCALE_PROFILE_INVALID,
        )
    }

    /// Creates the stable numeric type error used by dynamically typed bindings.
    pub fn invalid_number() -> Self {
        Self::input("interop.value.number_invalid", MESSAGE_NUMBER_INVALID)
    }

    /// Converts a rejected read or calculation limit, naming the limit in the detail.
    pub(crate) fn invalid_limit(error: impl fmt::Display) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.limit.invalid",
            MESSAGE_LIMIT_INVALID,
            ErrorDetails {
                detail: Some(error.to_string()),
                ..ErrorDetails::default()
            },
        )
    }

    /// Creates the stable error for a limit object a binding could not read.
    pub fn invalid_limit_payload(detail: String) -> Self {
        Self::new(
            InteropErrorKind::Input,
            "interop.limit.payload_invalid",
            MESSAGE_LIMIT_PAYLOAD_INVALID,
            ErrorDetails {
                detail: Some(detail),
                ..ErrorDetails::default()
            },
        )
    }

    pub(crate) fn invalid_archive_limit() -> Self {
        Self::input(
            "interop.workbook.archive_limit_invalid",
            MESSAGE_ARCHIVE_LIMIT_INVALID,
        )
    }

    pub(crate) fn defined_name_sheet_identity() -> Self {
        Self::state(
            "interop.defined_name.sheet_identity_invalid",
            MESSAGE_DEFINED_NAME_SHEET_IDENTITY,
        )
    }

    pub(crate) fn preview_not_found() -> Self {
        Self::state("interop.preview.not_found", MESSAGE_PREVIEW_NOT_FOUND)
    }

    /// Creates the stable malformed retained-preview cursor error used by bindings.
    pub fn preview_cursor_invalid() -> Self {
        Self::input(
            "interop.preview.cursor_invalid",
            MESSAGE_PREVIEW_CURSOR_INVALID,
        )
    }

    pub(crate) fn preview_response_limit() -> Self {
        Self::state(
            "interop.preview.response_limit_exceeded",
            MESSAGE_PREVIEW_RESPONSE_LIMIT,
        )
    }

    pub(crate) fn preview_id_exhausted() -> Self {
        Self::state("interop.preview.id_exhausted", MESSAGE_PREVIEW_ID_EXHAUSTED)
    }

    /// Creates the stable invalid-preview-section input error used by bindings and MCP.
    pub fn invalid_preview_section() -> Self {
        Self::input(
            "interop.preview.section_invalid",
            MESSAGE_PREVIEW_SECTION_INVALID,
        )
    }

    /// Creates the stable DTO serialization error used by native bindings.
    pub fn serialization() -> Self {
        Self::state("interop.dto.serialization", MESSAGE_SERIALIZATION)
    }

    fn new(
        kind: InteropErrorKind,
        code: impl Into<String>,
        message: impl Into<String>,
        details: ErrorDetails,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            details: Box::new(details),
        }
    }
}

impl From<cellrune::TargetCalculationError> for InteropError {
    fn from(error: cellrune::TargetCalculationError) -> Self {
        use cellrune::TargetCalculationErrorCode as Code;
        let kind = match error.code() {
            Code::Cancelled | Code::StaleResult => InteropErrorKind::State,
            _ => InteropErrorKind::Input,
        };
        let limit = LimitDiagnosticsDto {
            requested_cell_count: error.requested_cell_count(),
            evaluated_count: error.evaluated_count().map(crate::convert::count_u64),
            ..LimitDiagnosticsDto::default()
        };
        Self::new(
            kind,
            error.code().as_str(),
            error.code().message(),
            ErrorDetails {
                limit: limit.reported(),
                ..ErrorDetails::default()
            },
        )
    }
}

impl fmt::Display for InteropError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)?;
        if let Some(detail) = &self.details.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for InteropError {}

impl From<cellrune::DisplayFormatError> for InteropError {
    fn from(error: cellrune::DisplayFormatError) -> Self {
        let code = error.code();
        Self::new(
            InteropErrorKind::Input,
            code,
            error.to_string(),
            ErrorDetails {
                source_code: Some(code.to_owned()),
                ..ErrorDetails::default()
            },
        )
    }
}

impl From<ValidationError> for InteropError {
    fn from(error: ValidationError) -> Self {
        let code = error.code().as_str();
        let message = error.to_string();
        Self::new(
            InteropErrorKind::Validation,
            code,
            message,
            ErrorDetails {
                source_code: Some(code.to_owned()),
                ..ErrorDetails::default()
            },
        )
    }
}

impl From<cellrune::CsvError> for InteropError {
    fn from(error: cellrune::CsvError) -> Self {
        let code = error.code();
        let kind = match code {
            cellrune::CsvErrorCode::InvalidDelimiter => InteropErrorKind::Input,
            cellrune::CsvErrorCode::UnknownSheet
            | cellrune::CsvErrorCode::StaleCalculation
            | cellrune::CsvErrorCode::UnavailableResult => InteropErrorKind::Write,
            _ => InteropErrorKind::Read,
        };
        Self::new(
            kind,
            code.as_str(),
            error.to_string(),
            ErrorDetails {
                source_code: Some(code.as_str().to_owned()),
                detail: error.detail().map(str::to_owned),
                ..ErrorDetails::default()
            },
        )
    }
}

impl From<XlsxReadError> for InteropError {
    fn from(error: XlsxReadError) -> Self {
        let code = error.code().as_str();
        let limit = LimitDiagnosticsDto {
            name: error
                .phonetic_run_limit()
                .map(|limit| limit.as_str().to_owned()),
            observed: error.observed(),
            ..LimitDiagnosticsDto::default()
        };
        Self::new(
            InteropErrorKind::Read,
            code,
            error.to_string(),
            ErrorDetails {
                source_code: Some(code.to_owned()),
                source_id: error.source_id().map(|value| value.as_str().to_owned()),
                detail: error.detail().map(str::to_owned),
                cells: Vec::new(),
                limit: limit.reported(),
            },
        )
    }
}

impl From<XlsxWriteError> for InteropError {
    fn from(error: XlsxWriteError) -> Self {
        let code = error.code().as_str();
        Self::new(
            InteropErrorKind::Write,
            code,
            error.to_string(),
            ErrorDetails {
                source_code: Some(code.to_owned()),
                source_id: error.source_id().map(|value| value.as_str().to_owned()),
                detail: error.detail().map(str::to_owned),
                ..ErrorDetails::default()
            },
        )
    }
}

impl InteropError {
    /// Converts a write failure, naming its unavailable formulas with `workbook`'s sheet names.
    pub(crate) fn from_write(error: XlsxWriteError, workbook: &WorkbookSnapshot) -> Self {
        let cells = error
            .unavailable_formulas()
            .iter()
            .map(|id| cell_reference(workbook, *id))
            .collect();
        let mut converted = Self::from(error);
        converted.details.cells = cells;
        converted
    }
}

impl From<SessionError> for InteropError {
    fn from(error: SessionError) -> Self {
        let code = error.code().as_str();
        Self::new(
            InteropErrorKind::State,
            code,
            error.message(),
            ErrorDetails {
                source_code: Some(code.to_owned()),
                detail: error.detail().map(str::to_owned),
                ..ErrorDetails::default()
            },
        )
    }
}

impl From<ApplyChangesError> for InteropError {
    fn from(error: ApplyChangesError) -> Self {
        match error {
            ApplyChangesError::Session(error) => error.into(),
            ApplyChangesError::Validation(error) => error.into(),
        }
    }
}

impl From<DefinedNameAnalysisError> for InteropError {
    fn from(error: DefinedNameAnalysisError) -> Self {
        let code = error.kind().as_str();
        let kind = match error.kind() {
            DefinedNameAnalysisErrorKind::UnknownCurrentSheet => InteropErrorKind::Input,
            DefinedNameAnalysisErrorKind::ResourceLimit
            | DefinedNameAnalysisErrorKind::Cancelled => InteropErrorKind::State,
            _ => InteropErrorKind::State,
        };
        Self::new(
            kind,
            code,
            error.message(),
            ErrorDetails {
                source_code: Some(code.to_owned()),
                detail: error.detail().map(str::to_owned),
                ..ErrorDetails::default()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use cellrune::ValidationError;

    use super::{
        InteropError, InteropErrorKind, MESSAGE_ARCHIVE_LIMIT_INVALID, MESSAGE_NUMBER_INVALID,
        MESSAGE_SERIALIZATION,
    };

    #[test]
    fn invalid_number_has_stable_binding_contract() {
        let error = InteropError::invalid_number();

        assert_eq!(error.kind(), InteropErrorKind::Input);
        assert_eq!(error.code(), "interop.value.number_invalid");
        assert_eq!(error.message(), MESSAGE_NUMBER_INVALID);
        assert_eq!(
            error.to_string(),
            "interop.value.number_invalid: number must be an integer or floating-point value; booleans are not accepted"
        );
    }

    #[test]
    fn invalid_archive_limit_has_stable_transport_contract() {
        let error = InteropError::invalid_archive_limit();

        assert_eq!(error.kind(), InteropErrorKind::Input);
        assert_eq!(error.code(), "interop.workbook.archive_limit_invalid");
        assert_eq!(error.message(), MESSAGE_ARCHIVE_LIMIT_INVALID);
    }

    #[test]
    fn dto_serialization_has_a_stable_binding_contract() {
        let error = InteropError::serialization();

        assert_eq!(error.kind(), InteropErrorKind::State);
        assert_eq!(error.code(), "interop.dto.serialization");
        assert_eq!(error.message(), MESSAGE_SERIALIZATION);
    }

    #[test]
    fn formerly_unmapped_validation_errors_keep_their_core_codes() {
        let cases = [
            (
                ValidationError::PresentationRevisionExhausted,
                "validation.presentation_revision_exhausted",
            ),
            (
                ValidationError::PhoneticRangeEmpty { start: 1, end: 1 },
                "validation.phonetic_range_empty",
            ),
            (
                ValidationError::PhoneticRangeOutOfBounds {
                    end: 2,
                    base_utf16_len: 1,
                },
                "validation.phonetic_range_out_of_bounds",
            ),
            (
                ValidationError::PhoneticRangeSplitsSurrogate { offset: 1 },
                "validation.phonetic_range_splits_surrogate",
            ),
            (
                ValidationError::PhoneticRunsOutOfOrder,
                "validation.phonetic_runs_out_of_order",
            ),
            (
                ValidationError::PhoneticTextEmpty,
                "validation.phonetic_text_empty",
            ),
            (
                ValidationError::PhoneticTextInvalidCharacter { character: '\0' },
                "validation.phonetic_text_invalid_character",
            ),
            (
                ValidationError::PhoneticsRequireTextCell {
                    sheet_id: 1,
                    row: 1,
                    column: 1,
                },
                "validation.phonetics_require_text_cell",
            ),
            (
                ValidationError::PhoneticFontIdUnsupported { value: 1 },
                "validation.phonetic_font_id_unsupported",
            ),
            (
                ValidationError::AnnotatedTextReplacementRequired {
                    sheet_id: 1,
                    row: 1,
                    column: 1,
                },
                "validation.annotated_text_replacement_required",
            ),
            (
                ValidationError::FrozenRowsOutOfRange { value: 1_048_576 },
                "validation.frozen_rows_out_of_range",
            ),
            (
                ValidationError::FrozenColumnsOutOfRange { value: 16_384 },
                "validation.frozen_columns_out_of_range",
            ),
        ];

        for (source, expected_code) in cases {
            let error = InteropError::from(source);

            assert_eq!(error.kind(), InteropErrorKind::Validation);
            assert_eq!(error.code(), expected_code);
            assert_eq!(error.details().source_code.as_deref(), Some(expected_code));
        }
    }
}
