use super::super::error::{compatibility, detail};
use super::super::xml::XmlBudget;
use super::super::xstring::decode_xstring;
use super::super::{XlsxErrorCode, XlsxReadError};
use super::shared_strings::SharedStrings;
use crate::calculation::{calendar_date_exists, serial_from_calendar_date};
use crate::{
    CellAddress, CellRange, CellValue, DateSystem, DiagnosticCode, EXCEL_MAX_COLUMNS,
    EXCEL_MAX_ROWS, ExcelError, SavedResult, SavedResultIssue,
};

const SECONDS_PER_DAY: f64 = 86_400.0;

pub(super) fn parse_literal_value(
    cell_type: &str,
    raw_value: Option<&str>,
    inline_text: Option<&str>,
    shared_strings: Option<&SharedStrings>,
    date_system: DateSystem,
    budget: &XmlBudget,
) -> Result<Option<CellValue>, XlsxReadError> {
    match cell_type {
        "n" => raw_value
            .map(|value| parse_number(value, budget))
            .transpose(),
        "d" => raw_value
            .map(|value| parse_iso_date(value, date_system, budget))
            .transpose(),
        "s" => parse_shared_string(raw_value, shared_strings, budget),
        "inlineStr" => {
            if raw_value.is_some() {
                return Err(budget.error(XlsxErrorCode::InvalidCellValue));
            }
            Ok(inline_text.map(|value| CellValue::Text(value.to_owned())))
        }
        "str" => Ok(raw_value.map(|value| CellValue::Text(decode_xstring(value).into_owned()))),
        "b" => match raw_value.map(str::trim) {
            None => Ok(None),
            Some("0") => Ok(Some(CellValue::Logical(false))),
            Some("1") => Ok(Some(CellValue::Logical(true))),
            Some(value) => Err(budget
                .error(XlsxErrorCode::InvalidCellValue)
                .with_detail(value.to_owned())),
        },
        "e" => raw_value
            .map(|value| parse_error(value.trim(), budget).map(CellValue::Error))
            .transpose(),
        value => Err(budget
            .error(XlsxErrorCode::UnsupportedCellType)
            .with_detail(value.to_owned())),
    }
}

pub(super) fn parse_cell_reference(
    value: &str,
    budget: &XmlBudget,
) -> Result<CellAddress, XlsxReadError> {
    let split = value
        .bytes()
        .position(|byte| byte.is_ascii_digit())
        .ok_or_else(|| invalid_reference(value, budget))?;
    let (column, row) = value.split_at(split);
    if column.is_empty()
        || row.is_empty()
        || !column.bytes().all(|byte| byte.is_ascii_alphabetic())
        || !row.bytes().all(|byte| byte.is_ascii_digit())
        || row.starts_with('0')
    {
        return Err(invalid_reference(value, budget));
    }
    let mut column_index = 0_u32;
    for byte in column.bytes() {
        let digit = u32::from(byte.to_ascii_uppercase() - b'A' + 1);
        column_index = column_index
            .checked_mul(26)
            .and_then(|index| index.checked_add(digit))
            .ok_or_else(|| invalid_reference(value, budget))?;
    }
    let row_index = row
        .parse::<u32>()
        .map_err(|error| invalid_reference(value, budget).with_cause(error))?;
    if column_index > EXCEL_MAX_COLUMNS || row_index > EXCEL_MAX_ROWS {
        return Err(invalid_reference(value, budget));
    }
    CellAddress::from_indices(row_index, column_index)
        .map_err(|error| invalid_reference(value, budget).with_cause(error))
}

pub(super) fn parse_cell_range(
    value: &str,
    budget: &XmlBudget,
) -> Result<CellRange, XlsxReadError> {
    let (start, end) = value.split_once(':').unwrap_or((value, value));
    if end.contains(':') {
        return Err(budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_detail(value.to_owned()));
    }
    let start = parse_cell_reference(start, budget).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_cause(error)
    })?;
    let end = parse_cell_reference(end, budget).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_cause(error)
    })?;
    CellRange::new(start, end).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_cause(error)
    })
}

pub(super) fn parse_saved_result(
    cell_type: &str,
    raw_value: Option<&str>,
    inline_text: Option<&str>,
    shared_strings: Option<&SharedStrings>,
    date_system: DateSystem,
    budget: &XmlBudget,
) -> Result<SavedResult, XlsxReadError> {
    if raw_value.is_none() && inline_text.is_none() {
        return Ok(SavedResult::Missing);
    }
    if cell_type == "n" && raw_value == Some("") && inline_text.is_none() {
        return Ok(SavedResult::Missing);
    }
    let supported_type = matches!(cell_type, "n" | "s" | "str" | "b" | "e" | "d");
    if inline_text.is_none()
        && supported_type
        && let Ok(Some(value)) = parse_literal_value(
            cell_type,
            raw_value,
            None,
            shared_strings,
            date_system,
            budget,
        )
    {
        return Ok(SavedResult::Present(value));
    }
    let issue_code = if supported_type && inline_text.is_none() {
        compatibility::INVALID_SAVED_RESULT_CODE
    } else {
        compatibility::UNSUPPORTED_SAVED_RESULT_CODE
    };
    let code = DiagnosticCode::new(issue_code).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_cause(error)
    })?;
    let raw = raw_value.or(inline_text).map(str::to_owned);
    Ok(SavedResult::Invalid(SavedResultIssue::new(code, raw)))
}

fn parse_shared_string(
    raw_index: Option<&str>,
    shared_strings: Option<&SharedStrings>,
    budget: &XmlBudget,
) -> Result<Option<CellValue>, XlsxReadError> {
    let Some(raw_index) = raw_index else {
        return Ok(None);
    };
    let shared_strings = shared_strings.ok_or_else(|| {
        budget
            .error(XlsxErrorCode::InvalidSharedStrings)
            .with_detail(detail::SHARED_STRING_PART_REQUIRED)
    })?;
    let index = raw_index.trim().parse::<usize>().map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_cause(error)
    })?;
    let value = shared_strings.get(index).ok_or_else(|| {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_detail(index.to_string())
    })?;
    Ok(Some(CellValue::Text(value.to_owned())))
}

fn parse_number(value: &str, budget: &XmlBudget) -> Result<CellValue, XlsxReadError> {
    let number = value.trim().parse::<f64>().map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_cause(error)
    })?;
    CellValue::number(number).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_cause(error)
    })
}

/// Reads an ISO 8601 date cell (`t="d"`) as a serial in the workbook's date system.
///
/// Accepts `YYYY-MM-DD`, `YYYY-MM-DDThh:mm[:ss[.fff]]`, and `hh:mm[:ss[.fff]]`, each optionally
/// followed by the UTC designator `Z`, which spreadsheet serials have no zone to apply to, and
/// surrounded by ASCII whitespace. As in Excel, `24:00[:00]` is the end of the day, the end of
/// 9999-12-31 reads as 0, 1900-02-29 exists, and a date before the first day of the date system
/// reads as `#VALUE!`.
fn parse_iso_date(
    value: &str,
    date_system: DateSystem,
    budget: &XmlBudget,
) -> Result<CellValue, XlsxReadError> {
    let invalid = || {
        budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_detail(value.to_owned())
    };
    let trimmed = value.trim_ascii();
    let unzoned = trimmed.strip_suffix('Z').unwrap_or(trimmed);
    let (date, time) = match unzoned.split_once('T') {
        Some((date, time)) => (Some(date), Some(time)),
        None if unzoned.contains(':') => (None, Some(unzoned)),
        None => (Some(unzoned), None),
    };
    let date = date
        .map(|date| parse_iso_calendar_date(date).ok_or_else(invalid))
        .transpose()?;
    let (fraction, end_of_day) = match time {
        Some(time) => parse_iso_time_fraction(time).ok_or_else(invalid)?,
        None => (0.0, false),
    };
    let serial = match date {
        // The end of 9999-12-31 lies past the last serial; Excel reads it as 0 in either system.
        Some((9_999, 12, 31)) if end_of_day => 0.0,
        Some((year, month, day)) => {
            match serial_from_calendar_date(year, month, day, date_system) {
                Some(days) => days + fraction,
                None => return Ok(CellValue::Error(ExcelError::Value)),
            }
        }
        None => fraction,
    };
    CellValue::number(serial).map_err(|error| invalid().with_cause(error))
}

fn parse_iso_calendar_date(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.split('-');
    let year = i32::try_from(fixed_digits(parts.next()?, 4)?).ok()?;
    let month = fixed_digits(parts.next()?, 2)?;
    let day = fixed_digits(parts.next()?, 2)?;
    (parts.next().is_none() && calendar_date_exists(year, month, day)).then_some((year, month, day))
}

/// Returns the day fraction of `hh:mm[:ss[.fff]]` and whether the value is the `24:00[:00]` end of
/// the day.
fn parse_iso_time_fraction(value: &str) -> Option<(f64, bool)> {
    let (clock, fraction) = match value.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (value, None),
    };
    let mut parts = clock.split(':');
    let hours = fixed_digits(parts.next()?, 2)?;
    let minutes = fixed_digits(parts.next()?, 2)?;
    let seconds = parts.next().map_or(Some(0), |part| fixed_digits(part, 2))?;
    let end_of_day = hours == 24 && minutes == 0 && seconds == 0 && fraction.is_none();
    if parts.next().is_some() || (hours > 23 && !end_of_day) || minutes > 59 || seconds > 59 {
        return None;
    }
    if fraction.is_some() && clock.len() != "hh:mm:ss".len() {
        return None;
    }
    let fraction = match fraction {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => {
            format!("0.{digits}").parse::<f64>().ok()?
        }
        Some(_) => return None,
        None => 0.0,
    };
    let seconds = f64::from(hours * 3_600 + minutes * 60 + seconds) + fraction;
    Some((seconds / SECONDS_PER_DAY, end_of_day))
}

fn fixed_digits(value: &str, width: usize) -> Option<u32> {
    (value.len() == width && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn parse_error(value: &str, budget: &XmlBudget) -> Result<ExcelError, XlsxReadError> {
    match value {
        "#NULL!" => Ok(ExcelError::Null),
        "#DIV/0!" => Ok(ExcelError::DivisionByZero),
        "#VALUE!" => Ok(ExcelError::Value),
        "#REF!" => Ok(ExcelError::Reference),
        "#NAME?" => Ok(ExcelError::Name),
        "#NUM!" => Ok(ExcelError::Number),
        "#N/A" => Ok(ExcelError::NotAvailable),
        "#GETTING_DATA" => Ok(ExcelError::GettingData),
        "#SPILL!" => Ok(ExcelError::Spill),
        "#CALC!" => Ok(ExcelError::Calculation),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidCellValue)
            .with_detail(value.to_owned())),
    }
}

fn invalid_reference(value: &str, budget: &XmlBudget) -> XlsxReadError {
    budget
        .error(XlsxErrorCode::InvalidCellReference)
        .with_detail(value.to_owned())
}
