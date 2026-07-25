use std::sync::Arc;

use quick_xml::events::Event;

use super::super::package::PartPath;
use super::super::xml::{
    XmlBudget, decode_cdata, decode_reference, decode_text, is_spreadsheet_element,
    read_attributes, reader, require_spreadsheet_element,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use super::PresentationCapture;
use super::phonetic::{PhoneticItemBuilder, PhoneticReadBudget};
use crate::PhoneticAnnotation;

const SHARED_STRING_TABLE: &[u8] = b"sst";
const SHARED_STRING_ITEM: &[u8] = b"si";
const TEXT: &[u8] = b"t";
const PHONETIC_RUN: &[u8] = b"rPh";
const PHONETIC_PROPERTIES: &[u8] = b"phoneticPr";

#[derive(Debug)]
struct SharedString {
    text: String,
    annotation: Option<Arc<PhoneticAnnotation>>,
    overlaps_or_reorders: bool,
}

#[derive(Debug, Default)]
pub(super) struct SharedStrings(Vec<SharedString>);

impl SharedStrings {
    pub(super) fn get(&self, index: usize) -> Option<&str> {
        self.0.get(index).map(|item| item.text.as_str())
    }

    pub(super) fn annotation(&self, index: usize) -> Option<(Arc<PhoneticAnnotation>, bool)> {
        let item = self.0.get(index)?;
        item.annotation
            .as_ref()
            .map(|annotation| (Arc::clone(annotation), item.overlaps_or_reorders))
    }
}

pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
    capture: PresentationCapture,
    font_count: u32,
    phonetic_budget: &mut PhoneticReadBudget,
) -> Result<SharedStrings, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(
        limits,
        source.source_id(),
        XlsxErrorCode::InvalidSharedStrings,
    );
    let mut saw_root = false;
    let mut strings = Vec::new();
    let mut current = None::<String>;
    let mut current_phonetics = None::<PhoneticItemBuilder>;
    let mut item_depth = None::<u64>;
    let mut text_depth = None::<u64>;
    let mut phonetic_depth = None::<u64>;
    let mut total_bytes = 0_u64;

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidSharedStrings)
                .with_cause(error)
        })?;
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                if depth == 1 {
                    require_spreadsheet_element(
                        is_spreadsheet,
                        &local_name,
                        SHARED_STRING_TABLE,
                        &budget,
                    )?;
                    if saw_root {
                        return Err(budget.error(XlsxErrorCode::InvalidSharedStrings));
                    }
                    saw_root = true;
                    if let Some(unique_count) = attributes.unqualified("uniqueCount") {
                        let declared = unique_count.parse::<u64>().map_err(|error| {
                            budget
                                .error(XlsxErrorCode::InvalidSharedStrings)
                                .with_cause(error)
                        })?;
                        if declared > limits.max_shared_strings() {
                            return Err(budget.error(XlsxErrorCode::TooManySharedStrings));
                        }
                    }
                } else if is_spreadsheet {
                    if depth == 2 && local_name == SHARED_STRING_ITEM {
                        begin_item(
                            &mut current,
                            &mut current_phonetics,
                            &mut item_depth,
                            depth,
                            capture,
                            &budget,
                        )?;
                    } else if current.is_some()
                        && local_name == PHONETIC_RUN
                        && phonetic_depth.is_none()
                    {
                        phonetic_depth = Some(depth);
                        if capture == PresentationCapture::Document {
                            current_phonetics
                                .as_mut()
                                .ok_or_else(|| {
                                    budget.error(XlsxErrorCode::InvalidPhoneticMetadata)
                                })?
                                .begin_run(&attributes, limits, &budget)?;
                        }
                    } else if current.is_some()
                        && local_name == PHONETIC_PROPERTIES
                        && phonetic_depth.is_none()
                        && capture == PresentationCapture::Document
                    {
                        current_phonetics
                            .as_mut()
                            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                            .set_properties(&attributes, font_count, &budget)?;
                    } else if current.is_some()
                        && local_name == TEXT
                        && text_depth.replace(depth).is_some()
                    {
                        return Err(budget.error(XlsxErrorCode::InvalidSharedStrings));
                    }
                }
            }
            Event::Empty(element) => {
                let depth = budget.empty()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                if depth == 1 {
                    require_spreadsheet_element(
                        is_spreadsheet,
                        &local_name,
                        SHARED_STRING_TABLE,
                        &budget,
                    )?;
                    if saw_root {
                        return Err(budget.error(XlsxErrorCode::InvalidSharedStrings));
                    }
                    saw_root = true;
                } else if is_spreadsheet && depth == 2 && local_name == SHARED_STRING_ITEM {
                    finish_item(
                        String::new(),
                        None,
                        &mut strings,
                        &mut total_bytes,
                        limits,
                        phonetic_budget,
                        &budget,
                    )?;
                } else if current.is_some()
                    && local_name == PHONETIC_PROPERTIES
                    && capture == PresentationCapture::Document
                {
                    current_phonetics
                        .as_mut()
                        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                        .set_properties(&attributes, font_count, &budget)?;
                } else if current.is_some()
                    && local_name == PHONETIC_RUN
                    && capture == PresentationCapture::Document
                {
                    let builder = current_phonetics
                        .as_mut()
                        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?;
                    builder.begin_run(&attributes, limits, &budget)?;
                    builder.finish_run(&budget)?;
                }
            }
            Event::Text(text) if text_depth.is_some() => {
                append_text(
                    &mut current,
                    &mut current_phonetics,
                    phonetic_depth.is_some(),
                    decode_text(&text, &budget)?,
                    capture,
                    limits,
                    &budget,
                )?;
            }
            Event::CData(text) if text_depth.is_some() => {
                append_text(
                    &mut current,
                    &mut current_phonetics,
                    phonetic_depth.is_some(),
                    decode_cdata(&text, &budget)?,
                    capture,
                    limits,
                    &budget,
                )?;
            }
            Event::GeneralRef(reference) if text_depth.is_some() => {
                append_text(
                    &mut current,
                    &mut current_phonetics,
                    phonetic_depth.is_some(),
                    decode_reference(&reference, &budget)?,
                    capture,
                    limits,
                    &budget,
                )?;
            }
            Event::End(element) => {
                let depth = budget.end()?;
                let local_name = element.local_name().as_ref().to_vec();
                if text_depth == Some(depth) && local_name == TEXT {
                    text_depth = None;
                }
                if phonetic_depth == Some(depth) && local_name == PHONETIC_RUN {
                    if capture == PresentationCapture::Document {
                        current_phonetics
                            .as_mut()
                            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                            .finish_run(&budget)?;
                    }
                    phonetic_depth = None;
                }
                if item_depth == Some(depth) && local_name == SHARED_STRING_ITEM {
                    let value = current
                        .take()
                        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidSharedStrings))?;
                    let phonetics = current_phonetics.take();
                    finish_item(
                        value,
                        phonetics,
                        &mut strings,
                        &mut total_bytes,
                        limits,
                        phonetic_budget,
                        &budget,
                    )?;
                    item_depth = None;
                }
            }
            Event::DocType(_) => {
                return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    budget.finish(saw_root)?;
    if current.is_some()
        || current_phonetics.is_some()
        || item_depth.is_some()
        || text_depth.is_some()
        || phonetic_depth.is_some()
    {
        return Err(budget.error(XlsxErrorCode::InvalidSharedStrings));
    }
    Ok(SharedStrings(strings))
}

fn begin_item(
    current: &mut Option<String>,
    phonetics: &mut Option<PhoneticItemBuilder>,
    item_depth: &mut Option<u64>,
    depth: u64,
    capture: PresentationCapture,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if current.replace(String::new()).is_some() || item_depth.replace(depth).is_some() {
        return Err(budget.error(XlsxErrorCode::InvalidSharedStrings));
    }
    if capture == PresentationCapture::Document {
        *phonetics = Some(PhoneticItemBuilder::default());
    }
    Ok(())
}

fn append_text(
    current: &mut Option<String>,
    phonetics: &mut Option<PhoneticItemBuilder>,
    inside_phonetic: bool,
    text: String,
    capture: PresentationCapture,
    limits: ReadLimits,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if inside_phonetic {
        if capture == PresentationCapture::Document {
            phonetics
                .as_mut()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                .append_run_text(text, limits, budget)?;
        }
        return Ok(());
    }
    let current = current
        .as_mut()
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidSharedStrings))?;
    let next_len = current.len().saturating_add(text.len()) as u64;
    if next_len > limits.max_shared_string_bytes() {
        return Err(budget.error(XlsxErrorCode::SharedStringTooLarge));
    }
    current.push_str(&text);
    Ok(())
}

fn finish_item(
    value: String,
    phonetics: Option<PhoneticItemBuilder>,
    strings: &mut Vec<SharedString>,
    total_bytes: &mut u64,
    limits: ReadLimits,
    phonetic_budget: &mut PhoneticReadBudget,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if strings.len() as u64 >= limits.max_shared_strings() {
        return Err(budget.error(XlsxErrorCode::TooManySharedStrings));
    }
    if value.len() as u64 > limits.max_shared_string_bytes() {
        return Err(budget.error(XlsxErrorCode::SharedStringTooLarge));
    }
    *total_bytes = total_bytes.saturating_add(value.len() as u64);
    if *total_bytes > limits.max_total_shared_string_bytes() {
        return Err(budget.error(XlsxErrorCode::TotalSharedStringsTooLarge));
    }
    let completed = phonetics
        .map(|builder| builder.finish(&value, phonetic_budget, limits, budget))
        .transpose()?
        .flatten();
    strings.push(SharedString {
        text: value,
        annotation: completed
            .as_ref()
            .map(|completed| Arc::clone(&completed.annotation)),
        overlaps_or_reorders: completed.is_some_and(|completed| completed.overlaps_or_reorders),
    });
    Ok(())
}
