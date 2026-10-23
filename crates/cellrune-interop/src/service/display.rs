//! Stateless display text for a value and a number-format code.

use cellrune::{DateSystem, LocaleProfile};

use crate::convert::{locale_profile, value_from_dto};
use crate::{DateSystemDto, DisplayFormatRequestDto, DisplayFormatResultDto, InteropError};

/// Returns the text a cell holding the request's value shows with the request's number-format
/// code, as `cellrune::format_display_text` renders it.
///
/// # Errors
///
/// Returns `display_format.unsupported` for a code whose Excel display has not been established,
/// and the errors a value edit reports for a non-finite number or an unknown error value.
pub fn format_display_text(
    request: DisplayFormatRequestDto,
) -> Result<DisplayFormatResultDto, InteropError> {
    let value = value_from_dto(request.value)?;
    let date_system = match request.date_system {
        DateSystemDto::Excel1900 => DateSystem::Excel1900,
        DateSystemDto::Excel1904 => DateSystem::Excel1904,
    };
    let profile = request
        .locale_profile
        .map_or_else(LocaleProfile::default, locale_profile);
    let text = cellrune::format_display_text(&value, &request.format_code, date_system, profile)?;
    Ok(DisplayFormatResultDto { text })
}
