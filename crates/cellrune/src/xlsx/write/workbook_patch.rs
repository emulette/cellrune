use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{SPREADSHEETML_STRICT, SPREADSHEETML_TRANSITIONAL};
use crate::{CalculationHints, CalculationMode};

const DETAIL_DUPLICATE_CALCULATION_PROPERTIES: &str =
    "workbook contains duplicate calculation properties";
const DETAIL_MISSING_WORKBOOK_ROOT: &str = "workbook root was not found";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

pub(crate) fn patch_calculation_properties(
    bytes: &[u8],
    source: &PartPath,
    request_host_recalculation: bool,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    patch_calculation_properties_with_hints(bytes, source, request_host_recalculation, None, limits)
}

pub(crate) fn patch_calculation_properties_with_hints(
    bytes: &[u8],
    source: &PartPath,
    request_host_recalculation: bool,
    hints: Option<CalculationHints>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = NsReader::from_reader(bytes);
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut workbook_depth = None;
    let mut workbook_name = None;
    let mut saw_calc_properties = false;

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                let local = element.local_name();
                if depth == 1 && spreadsheet && local.as_ref() == b"workbook" {
                    workbook_depth = Some(depth);
                    workbook_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && spreadsheet
                    && local.as_ref() == b"extLst"
                    && !saw_calc_properties
                {
                    write_generated_calc_properties(
                        &mut writer,
                        workbook_name.as_deref().unwrap_or(b"workbook"),
                        request_host_recalculation,
                        hints,
                        source,
                    )?;
                    saw_calc_properties = true;
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && spreadsheet
                    && local.as_ref() == b"calcPr"
                {
                    if saw_calc_properties {
                        return Err(invalid_generated(
                            source,
                            DETAIL_DUPLICATE_CALCULATION_PROPERTIES,
                        ));
                    }
                    saw_calc_properties = true;
                    let patched = patch_calc_properties_start(
                        &element,
                        request_host_recalculation,
                        hints,
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && spreadsheet
                    && element.local_name().as_ref() == b"extLst"
                    && !saw_calc_properties
                {
                    write_generated_calc_properties(
                        &mut writer,
                        workbook_name.as_deref().unwrap_or(b"workbook"),
                        request_host_recalculation,
                        hints,
                        source,
                    )?;
                    saw_calc_properties = true;
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && spreadsheet
                    && element.local_name().as_ref() == b"calcPr"
                {
                    if saw_calc_properties {
                        return Err(invalid_generated(
                            source,
                            DETAIL_DUPLICATE_CALCULATION_PROPERTIES,
                        ));
                    }
                    saw_calc_properties = true;
                    let patched = patch_calc_properties_start(
                        &element,
                        request_host_recalculation,
                        hints,
                        source,
                    )?;
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else {
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if workbook_depth == Some(depth)
                    && element.local_name().as_ref() == b"workbook"
                    && !saw_calc_properties
                {
                    write_generated_calc_properties(
                        &mut writer,
                        workbook_name.as_deref().unwrap_or(b"workbook"),
                        request_host_recalculation,
                        hints,
                        source,
                    )?;
                    saw_calc_properties = true;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if workbook_depth.is_none() || !saw_calc_properties {
        return Err(invalid_generated(source, DETAIL_MISSING_WORKBOOK_ROOT));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

fn write_generated_calc_properties(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_name: &[u8],
    request_host_recalculation: bool,
    hints: Option<CalculationHints>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let qualified = qualified_sibling_name(workbook_name, b"calcPr");
    let mut calc = BytesStart::new(decode_name(&qualified, source)?.to_owned());
    push_calculation_attributes(&mut calc, request_host_recalculation, hints);
    write_event(writer, Event::Empty(calc), source)
}

fn patch_calc_properties_start(
    element: &BytesStart<'_>,
    request_host_recalculation: bool,
    hints: Option<CalculationHints>,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let replace = matches!(attribute.key.as_ref(), b"fullCalcOnLoad" | b"forceFullCalc")
            || (hints.is_some()
                && matches!(attribute.key.as_ref(), b"calcMode" | b"calcId" | b"iterate"));
        if !replace {
            patched.push_attribute(attribute);
        }
    }
    push_calculation_attributes(&mut patched, request_host_recalculation, hints);
    Ok(patched.into_owned())
}

fn push_calculation_attributes(
    element: &mut BytesStart<'_>,
    request_host_recalculation: bool,
    hints: Option<CalculationHints>,
) {
    if let Some(hints) = hints {
        if let Some(mode) = hints.mode() {
            element.push_attribute((
                "calcMode",
                match mode {
                    CalculationMode::Automatic => "auto",
                    CalculationMode::AutomaticExceptDataTables => "autoNoTable",
                    CalculationMode::Manual => "manual",
                },
            ));
        }
        if let Some(id) = hints.calculation_id() {
            element.push_attribute(("calcId", id.to_string().as_str()));
        }
        if let Some(iterative) = hints.iterative_calculation() {
            element.push_attribute(("iterate", if iterative { "1" } else { "0" }));
        }
        let full = request_host_recalculation || hints.full_calculation_on_load().unwrap_or(false);
        let force = request_host_recalculation || hints.force_full_calculation().unwrap_or(false);
        element.push_attribute(("fullCalcOnLoad", if full { "1" } else { "0" }));
        element.push_attribute(("forceFullCalc", if force { "1" } else { "0" }));
    } else {
        let value = if request_host_recalculation { "1" } else { "0" };
        element.push_attribute(("fullCalcOnLoad", value));
        element.push_attribute(("forceFullCalc", value));
    }
}

fn qualified_sibling_name(template: &[u8], local_name: &[u8]) -> Vec<u8> {
    let mut output = template
        .iter()
        .rposition(|byte| *byte == b':')
        .map_or_else(Vec::new, |index| template[..=index].to_vec());
    output.extend_from_slice(local_name);
    output
}

fn decode_name<'a>(name: &'a [u8], source: &PartPath) -> Result<&'a str, XlsxWriteError> {
    std::str::from_utf8(name).map_err(|error| invalid_xml(source, error))
}

fn is_spreadsheet_element(xml: &NsReader<&[u8]>, name: QName<'_>) -> bool {
    matches!(
        xml.resolver().resolve_element(name).0,
        ResolveResult::Bound(namespace)
            if namespace.as_ref() == SPREADSHEETML_TRANSITIONAL
                || namespace.as_ref() == SPREADSHEETML_STRICT
    )
}

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(resource_error(
            source,
            DETAIL_XML_DEPTH,
            depth,
            limits.max_xml_depth(),
        ));
    }
    Ok(())
}

fn enforce_bytes(
    actual: usize,
    limits: WriteLimits,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            DETAIL_XML_BYTES,
            actual as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    Ok(())
}

fn write_event<'a>(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    event: Event<'a>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    writer
        .write_event(event)
        .map_err(|error| invalid_xml(source, error))
}

fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn invalid_generated(source: &PartPath, detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(detail)
        .at_source(source.source_id())
}

fn resource_error(
    source: &PartPath,
    name: &'static str,
    actual: u64,
    maximum: u64,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
        .at_source(source.source_id())
}

#[cfg(test)]
#[path = "workbook_patch_tests.rs"]
mod tests;
