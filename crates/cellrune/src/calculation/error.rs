use super::parser::ParseError;

pub(crate) const ERROR_LEX_UNEXPECTED_CHARACTER: &str = "unexpected character in formula";
pub(crate) const ERROR_LEX_UNTERMINATED_STRING: &str = "unterminated string literal";
pub(crate) const ERROR_LEX_UNTERMINATED_SHEET_NAME: &str = "unterminated quoted sheet name";
pub(crate) const ERROR_LEX_UNKNOWN_ERROR_LITERAL: &str = "unknown error literal";
pub(crate) const ERROR_LEX_UNTERMINATED_STRUCTURED_REF: &str =
    "unterminated structured reference brackets";
pub(crate) const ERROR_PARSE_UNEXPECTED_TOKEN: &str = "unexpected token";
pub(crate) const ERROR_PARSE_UNEXPECTED_END: &str = "unexpected end of formula";
pub(crate) const ERROR_PARSE_INVALID_REFERENCE: &str = "invalid cell reference";
pub(crate) const ERROR_PARSE_MISMATCHED_RANGE: &str = "mismatched range endpoints";

pub(super) const MESSAGE_MISSING_FORMULA_TEXT: &str = "formula text is unavailable";
pub(super) const MESSAGE_PARSE_ERROR: &str = "formula cannot be parsed";
pub(super) const MESSAGE_UNSUPPORTED_FUNCTION: &str = "formula uses an unsupported function";
pub(super) const MESSAGE_UNSUPPORTED_NAME: &str = "formula uses an unsupported defined name";
pub(super) const MESSAGE_UNSUPPORTED_EXPRESSION: &str = "formula uses an unsupported expression";
pub(super) const MESSAGE_UNSUPPORTED_SHEET_RANGE: &str =
    "formula uses an unsupported 3-D sheet-range reference";
pub(super) const MESSAGE_UNSUPPORTED_STRUCTURED_REFERENCE: &str =
    "formula uses an unsupported structured table reference";
pub(super) const MESSAGE_RESOURCE_LIMIT_EXCEEDED: &str =
    "formula calculation exceeds a configured resource limit";
pub(super) const MESSAGE_VOLATILE_INPUT_MISSING: &str =
    "formula requires a deterministic volatile input";
pub(super) const MESSAGE_CIRCULAR_REFERENCE: &str = "formula participates in a circular reference";
pub(super) const MESSAGE_BLOCKED_BY_UPSTREAM: &str =
    "formula depends on a cell that could not be calculated";

pub(crate) const ERROR_PARSE_INVALID_STRUCTURED_REFERENCE: &str = "invalid structured reference";
pub(crate) const ERROR_PARSE_INVALID_EXTERNAL_REFERENCE: &str =
    "invalid external workbook reference";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorCode {
    UnexpectedCharacter,
    UnterminatedString,
    UnterminatedSheetName,
    UnknownErrorLiteral,
    UnterminatedStructuredReference,
    UnexpectedToken,
    UnexpectedEnd,
    InvalidReference,
    MismatchedRange,
    InvalidStructuredReference,
    InvalidExternalReference,
}

impl ParseErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "formula.lex.unexpected_character",
            Self::UnterminatedString => "formula.lex.unterminated_string",
            Self::UnterminatedSheetName => "formula.lex.unterminated_sheet_name",
            Self::UnknownErrorLiteral => "formula.lex.unknown_error_literal",
            Self::UnterminatedStructuredReference => {
                "formula.lex.unterminated_structured_reference"
            }
            Self::UnexpectedToken => "formula.parse.unexpected_token",
            Self::UnexpectedEnd => "formula.parse.unexpected_end",
            Self::InvalidReference => "formula.parse.invalid_reference",
            Self::MismatchedRange => "formula.parse.mismatched_range",
            Self::InvalidStructuredReference => "formula.parse.invalid_structured_reference",
            Self::InvalidExternalReference => "formula.parse.invalid_external_reference",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => ERROR_LEX_UNEXPECTED_CHARACTER,
            Self::UnterminatedString => ERROR_LEX_UNTERMINATED_STRING,
            Self::UnterminatedSheetName => ERROR_LEX_UNTERMINATED_SHEET_NAME,
            Self::UnknownErrorLiteral => ERROR_LEX_UNKNOWN_ERROR_LITERAL,
            Self::UnterminatedStructuredReference => ERROR_LEX_UNTERMINATED_STRUCTURED_REF,
            Self::UnexpectedToken => ERROR_PARSE_UNEXPECTED_TOKEN,
            Self::UnexpectedEnd => ERROR_PARSE_UNEXPECTED_END,
            Self::InvalidReference => ERROR_PARSE_INVALID_REFERENCE,
            Self::MismatchedRange => ERROR_PARSE_MISMATCHED_RANGE,
            Self::InvalidStructuredReference => ERROR_PARSE_INVALID_STRUCTURED_REFERENCE,
            Self::InvalidExternalReference => ERROR_PARSE_INVALID_EXTERNAL_REFERENCE,
        }
    }
}

pub(super) fn parse_error_detail(error: &ParseError) -> String {
    format!(
        "bytes {}..{} [{}]: {}",
        error.span.start,
        error.span.end,
        error.code.as_str(),
        error.code.message()
    )
}
