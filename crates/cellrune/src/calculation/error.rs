use super::parser::ErrorPosition;

pub(crate) const ERROR_LEX_UNEXPECTED_CHARACTER: &str = "unexpected character in formula";
pub(crate) const ERROR_LEX_UNTERMINATED_STRING: &str = "unterminated string literal";
pub(crate) const ERROR_LEX_UNTERMINATED_SHEET_NAME: &str = "unterminated quoted sheet name";
pub(crate) const ERROR_LEX_UNKNOWN_ERROR_LITERAL: &str = "unknown error literal";
pub(crate) const ERROR_LEX_UNTERMINATED_STRUCTURED_REF: &str =
    "unterminated structured reference brackets";
pub(crate) const ERROR_LEX_EXTERNAL_REFERENCE: &str = "external workbook reference";
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

pub(super) const DETAIL_POSITION_CHARACTER: &str = "character";
pub(super) const DETAIL_POSITION_TOKEN: &str = "token";

pub(super) fn parse_error_detail(position: ErrorPosition, message: &str) -> String {
    let (label, index) = match position {
        ErrorPosition::Character(index) => (DETAIL_POSITION_CHARACTER, index),
        ErrorPosition::Token(index) => (DETAIL_POSITION_TOKEN, index),
    };
    format!("{label} {index}: {message}")
}
