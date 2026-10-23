//! The text a cell shows for a value and a number-format code.

use std::error::Error;
use std::fmt;

use super::error::MESSAGE_DISPLAY_FORMAT_UNSUPPORTED;
use super::textfmt::{FormatContext, format_number, format_text};
use crate::{CellValue, DateSystem, LocaleProfile};

/// Why [`format_display_text`] could not produce a cell's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DisplayFormatError {
    /// The format code uses a construct whose Excel display has not been established, such as a
    /// `*` fill, or the number needs more digits than the format engine renders.
    Unsupported,
}

impl DisplayFormatError {
    /// Returns the stable machine-readable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "display_format.unsupported",
        }
    }
}

impl fmt::Display for DisplayFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str(MESSAGE_DISPLAY_FORMAT_UNSUPPORTED),
        }
    }
}

impl Error for DisplayFormatError {}

/// Returns the text a cell holding `value` shows with the number-format code `format_code`.
///
/// Numbers follow the same engine as `TEXT`, in the given date system and with the names of the
/// profile's regional format. Text goes through the code's text section: the fourth section, or a
/// lone section holding `@`; a code without one shows the text unchanged. Logical and error values
/// show `TRUE`, `FALSE`, or the error, and a blank shows nothing. `General` shows a number of up to
/// 11 characters as `TEXT` does; a longer one, which Excel rounds or writes in scientific notation,
/// is unsupported, and so is a `*` fill, because the column width is not an input.
///
/// # Errors
///
/// Returns [`DisplayFormatError::Unsupported`] when the code uses a construct whose Excel display
/// has not been established or the number needs more digits than the engine renders.
pub fn format_display_text(
    value: &CellValue,
    format_code: &str,
    date_system: DateSystem,
    locale_profile: LocaleProfile,
) -> Result<String, DisplayFormatError> {
    let context = FormatContext {
        date_system,
        locale: locale_profile.locale(),
        drops_fill: false,
    };
    let formatted = match value {
        CellValue::Blank => return Ok(String::new()),
        CellValue::Number(number) => format_number(number.get(), format_code, context),
        CellValue::Text(text) => format_text(text, format_code),
        CellValue::Logical(true) => return Ok("TRUE".to_owned()),
        CellValue::Logical(false) => return Ok("FALSE".to_owned()),
        CellValue::Error(error) => return Ok(error.as_str().to_owned()),
    };
    formatted.map_err(|_| DisplayFormatError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExcelError, Locale};

    fn display(value: CellValue, format: &str) -> Result<String, DisplayFormatError> {
        format_display_text(
            &value,
            format,
            DateSystem::Excel1900,
            LocaleProfile::default(),
        )
    }

    fn number(value: f64) -> CellValue {
        CellValue::number(value).expect("finite test number")
    }

    #[test]
    fn numbers_use_the_number_format_engine() {
        assert_eq!(
            display(number(1234.567), "#,##0.00").as_deref(),
            Ok("1,234.57")
        );
        assert_eq!(
            display(number(-5.0), "0.00;[Red](0.00)").as_deref(),
            Ok("(5.00)")
        );
        assert_eq!(
            display(number(46_230.0), "d-mmm-yy").as_deref(),
            Ok("27-Jul-26")
        );
        assert_eq!(
            format_display_text(
                &number(0.0),
                "yyyy-mm-dd",
                DateSystem::Excel1904,
                LocaleProfile::default()
            )
            .as_deref(),
            Ok("1904-01-01")
        );
        assert_eq!(
            display(number(5.0), "_($* #,##0_)"),
            Err(DisplayFormatError::Unsupported)
        );
        assert_eq!(
            display(number(1e16), "0"),
            Err(DisplayFormatError::Unsupported)
        );
    }

    #[test]
    fn general_numbers_longer_than_eleven_characters_are_unsupported() {
        assert_eq!(display(number(1234.5), "General").as_deref(), Ok("1234.5"));
        // Excel shows "1E+20" and "3.141592654".
        assert_eq!(
            display(number(1e20), "General"),
            Err(DisplayFormatError::Unsupported)
        );
        assert_eq!(
            display(number(std::f64::consts::PI), "General"),
            Err(DisplayFormatError::Unsupported)
        );
    }

    #[test]
    fn text_uses_the_text_section_or_stays_unchanged() {
        let text = || CellValue::Text("abc".to_owned());
        assert_eq!(display(text(), "0.00").as_deref(), Ok("abc"));
        assert_eq!(display(text(), "@").as_deref(), Ok("abc"));
        assert_eq!(
            display(text(), "\"gross receipts for \"@").as_deref(),
            Ok("gross receipts for abc")
        );
        assert_eq!(display(text(), "0;-0;0;[Blue]@!").as_deref(), Ok("abc!"));
        assert_eq!(display(text(), ";;;").as_deref(), Ok(""));
        assert_eq!(display(text(), "0;@"), Err(DisplayFormatError::Unsupported));
    }

    #[test]
    fn other_values_ignore_the_format() {
        assert_eq!(display(CellValue::Blank, "0.00").as_deref(), Ok(""));
        assert_eq!(
            display(CellValue::Logical(true), "0.00").as_deref(),
            Ok("TRUE")
        );
        assert_eq!(
            display(CellValue::Error(ExcelError::DivisionByZero), "0.00").as_deref(),
            Ok("#DIV/0!")
        );
    }

    #[test]
    fn the_locale_profile_selects_the_names() {
        let korean = LocaleProfile::new(Locale::KoKr);
        assert_eq!(
            format_display_text(&number(46_230.0), "mmmm", DateSystem::Excel1900, korean),
            Err(DisplayFormatError::Unsupported)
        );
        assert_eq!(
            format_display_text(
                &number(46_230.0),
                "yyyy-mm-dd",
                DateSystem::Excel1900,
                korean
            )
            .as_deref(),
            Ok("2026-07-27")
        );
        assert_eq!(
            DisplayFormatError::Unsupported.code(),
            "display_format.unsupported"
        );
    }
}
