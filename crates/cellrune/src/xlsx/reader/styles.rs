use std::collections::BTreeMap;

use quick_xml::events::Event;

use super::super::error::detail;
use super::super::package::PartPath;
use super::super::xml::{
    XmlBudget, is_spreadsheet_element, read_attributes, reader, require_spreadsheet_element,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::{NumberFormat, NumberFormatKind};

const STYLE_SHEET: &[u8] = b"styleSheet";
const NUMBER_FORMATS: &[u8] = b"numFmts";
const NUMBER_FORMAT: &[u8] = b"numFmt";
const FONTS: &[u8] = b"fonts";
const FONT: &[u8] = b"font";
const CELL_FORMATS: &[u8] = b"cellXfs";
const FORMAT_RECORD: &[u8] = b"xf";

#[derive(Debug)]
pub(super) struct Styles {
    formats: Vec<NumberFormat>,
    font_count: u32,
}

#[derive(Debug, Default)]
struct StyleParseState {
    stack: Vec<Box<[u8]>>,
    saw_root: bool,
    saw_number_formats: bool,
    saw_fonts: bool,
    saw_cell_formats: bool,
    custom_formats: BTreeMap<u32, Box<str>>,
    cell_format_ids: Vec<u32>,
    font_count: u32,
}

impl Styles {
    pub(super) fn format(&self, style_index: usize) -> Option<&NumberFormat> {
        self.formats.get(style_index)
    }

    pub(super) const fn font_count(&self) -> u32 {
        self.font_count
    }
}

impl Default for Styles {
    fn default() -> Self {
        Self {
            formats: vec![NumberFormat::default()],
            font_count: 1,
        }
    }
}

pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<Styles, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidStyles);
    let mut state = StyleParseState::default();

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| budget.error(XlsxErrorCode::InvalidStyles).with_cause(error))?;
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                process_element(
                    is_spreadsheet,
                    &local_name,
                    depth,
                    &attributes,
                    &budget,
                    &mut state,
                )?;
                state.stack.push(local_name.into_boxed_slice());
            }
            Event::Empty(element) => {
                let depth = budget.empty()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                process_element(
                    is_spreadsheet,
                    &local_name,
                    depth,
                    &attributes,
                    &budget,
                    &mut state,
                )?;
            }
            Event::End(_) => {
                budget.end()?;
                state
                    .stack
                    .pop()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidStyles))?;
            }
            Event::DocType(_) => {
                return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    budget.finish(state.saw_root)?;
    if state.cell_format_ids.is_empty() {
        return Err(budget.error(XlsxErrorCode::InvalidStyles));
    }
    let formats = state
        .cell_format_ids
        .into_iter()
        .map(|id| make_number_format(id, state.custom_formats.get(&id).map(AsRef::as_ref)))
        .collect();
    Ok(Styles {
        formats,
        font_count: state.font_count.max(1),
    })
}

fn process_element(
    is_spreadsheet: bool,
    local_name: &[u8],
    depth: u64,
    attributes: &super::super::xml::XmlAttributes,
    budget: &XmlBudget,
    state: &mut StyleParseState,
) -> Result<(), XlsxReadError> {
    if depth == 1 {
        require_spreadsheet_element(is_spreadsheet, local_name, STYLE_SHEET, budget)?;
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidStyles));
        }
        state.saw_root = true;
        return Ok(());
    }
    if !is_spreadsheet {
        return Ok(());
    }
    if depth == 2 && local_name == NUMBER_FORMATS {
        if std::mem::replace(&mut state.saw_number_formats, true) {
            return Err(budget.error(XlsxErrorCode::InvalidStyles));
        }
        return Ok(());
    }
    if depth == 2 && local_name == FONTS {
        if std::mem::replace(&mut state.saw_fonts, true) {
            return Err(budget.error(XlsxErrorCode::InvalidStyles));
        }
        return Ok(());
    }
    if depth == 2 && local_name == CELL_FORMATS {
        if std::mem::replace(&mut state.saw_cell_formats, true) {
            return Err(budget.error(XlsxErrorCode::InvalidStyles));
        }
        return Ok(());
    }
    if depth != 3 {
        return Ok(());
    }
    let parent = state.stack.get(1).map(|name| name.as_ref());
    if local_name == FONT && parent == Some(FONTS) {
        state.font_count = state
            .font_count
            .checked_add(1)
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidStyles))?;
    } else if local_name == NUMBER_FORMAT && parent == Some(NUMBER_FORMATS) {
        let id = required_u32(attributes.unqualified("numFmtId"), "numFmtId", budget)?;
        let code = required(attributes.unqualified("formatCode"), "formatCode", budget)?;
        if state
            .custom_formats
            .insert(id, code.to_owned().into_boxed_str())
            .is_some()
        {
            return Err(budget
                .error(XlsxErrorCode::InvalidStyles)
                .with_detail(detail::DUPLICATE_NUMBER_FORMAT));
        }
    } else if local_name == FORMAT_RECORD && parent == Some(CELL_FORMATS) {
        state.cell_format_ids.push(required_u32(
            attributes.unqualified("numFmtId"),
            "numFmtId",
            budget,
        )?);
    }
    Ok(())
}

fn make_number_format(id: u32, custom: Option<&str>) -> NumberFormat {
    if let Some(code) = custom {
        return NumberFormat::new(
            id,
            Some(Box::<str>::from(code)),
            classify_custom_format(code),
        );
    }
    let (code, kind) = built_in_format(id);
    NumberFormat::new(id, code.map(Box::<str>::from), kind)
}

fn built_in_format(id: u32) -> (Option<&'static str>, NumberFormatKind) {
    match id {
        0 => (Some("General"), NumberFormatKind::General),
        14 => (Some("mm-dd-yy"), NumberFormatKind::Date),
        15 => (Some("d-mmm-yy"), NumberFormatKind::Date),
        16 => (Some("d-mmm"), NumberFormatKind::Date),
        17 => (Some("mmm-yy"), NumberFormatKind::Date),
        18 => (Some("h:mm AM/PM"), NumberFormatKind::Time),
        19 => (Some("h:mm:ss AM/PM"), NumberFormatKind::Time),
        20 => (Some("h:mm"), NumberFormatKind::Time),
        21 => (Some("h:mm:ss"), NumberFormatKind::Time),
        22 => (Some("m/d/yy h:mm"), NumberFormatKind::DateTime),
        45 => (Some("mm:ss"), NumberFormatKind::Time),
        46 => (Some("[h]:mm:ss"), NumberFormatKind::Duration),
        47 => (Some("mm:ss.0"), NumberFormatKind::Time),
        _ => (None, NumberFormatKind::Number),
    }
}

fn classify_custom_format(code: &str) -> NumberFormatKind {
    let mut escaped = false;
    let mut quoted = false;
    let mut bracket = None::<String>;
    let mut has_date = false;
    let mut has_time = false;
    let mut has_month = false;
    for character in code.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if matches!(character, '\\' | '_' | '*') && !quoted && bracket.is_none() {
            escaped = true;
            continue;
        }
        if character == '"' && bracket.is_none() {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if let Some(content) = &mut bracket {
            if character == ']' {
                if matches!(
                    content.to_ascii_lowercase().as_str(),
                    "h" | "hh" | "m" | "mm" | "s" | "ss"
                ) {
                    return NumberFormatKind::Duration;
                }
                bracket = None;
            } else {
                content.push(character);
            }
            continue;
        }
        if character == '[' {
            bracket = Some(String::new());
            continue;
        }
        match character.to_ascii_lowercase() {
            'y' | 'd' | 'e' => has_date = true,
            'h' | 's' => has_time = true,
            'm' => has_month = true,
            _ => {}
        }
    }
    if has_date && has_time {
        NumberFormatKind::DateTime
    } else if has_date {
        NumberFormatKind::Date
    } else if has_time {
        NumberFormatKind::Time
    } else if has_month {
        NumberFormatKind::Date
    } else {
        NumberFormatKind::Number
    }
}

fn required<'a>(
    value: Option<&'a str>,
    name: &str,
    budget: &XmlBudget,
) -> Result<&'a str, XlsxReadError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidStyles)
            .with_detail(format!("{} {name}", detail::MISSING_ATTRIBUTE))),
    }
}

fn required_u32(value: Option<&str>, name: &str, budget: &XmlBudget) -> Result<u32, XlsxReadError> {
    required(value, name, budget)?
        .parse::<u32>()
        .map_err(|error| budget.error(XlsxErrorCode::InvalidStyles).with_cause(error))
}

#[cfg(test)]
mod tests {
    use super::classify_custom_format;
    use crate::NumberFormatKind;

    #[test]
    fn custom_number_formats_are_classified_without_reading_literals() {
        assert_eq!(classify_custom_format("yyyy-mm-dd"), NumberFormatKind::Date);
        assert_eq!(classify_custom_format("h:mm:ss"), NumberFormatKind::Time);
        assert_eq!(
            classify_custom_format("yyyy-mm-dd h:mm"),
            NumberFormatKind::DateTime
        );
        assert_eq!(
            classify_custom_format("[h]:mm:ss"),
            NumberFormatKind::Duration
        );
        assert_eq!(classify_custom_format("0.00\\m"), NumberFormatKind::Number);
        assert_eq!(
            classify_custom_format("0.00\" days\""),
            NumberFormatKind::Number
        );
    }
}
