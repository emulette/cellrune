use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::NsReader;

use super::canonical::StyleRegistry;
use super::serialization::escape_attribute;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::{CalculationCellId, NumberFormat};

const DETAIL_MISSING_CELL_XFS: &str = "styles part does not contain cellXfs";
const DETAIL_BASE_STYLE: &str = "cell references an unavailable base style";
const DETAIL_CUSTOM_CONFLICT: &str =
    "one custom number-format ID is associated with multiple format codes";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

pub(crate) struct StyleRequest {
    pub(crate) base_index: usize,
    pub(crate) format: NumberFormat,
}

pub(crate) struct DocumentStylePlan {
    pub(crate) bytes: Vec<u8>,
    pub(crate) indexes: BTreeMap<CalculationCellId, usize>,
}

#[derive(Clone)]
struct XfTemplate {
    events: Vec<Event<'static>>,
}

struct StyleAnalysis {
    custom_formats: BTreeMap<u32, String>,
    xfs: Vec<XfTemplate>,
    has_num_formats: bool,
}

pub(crate) fn plan_document_styles(
    bytes: Option<&[u8]>,
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    limits: WriteLimits,
) -> Result<Option<DocumentStylePlan>, XlsxWriteError> {
    if requests.is_empty() {
        return Ok(None);
    }
    match bytes {
        Some(bytes) => patch_existing_styles(bytes, source, requests, limits).map(Some),
        None => generate_styles(source, requests, limits).map(Some),
    }
}

fn generate_styles(
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    limits: WriteLimits,
) -> Result<DocumentStylePlan, XlsxWriteError> {
    if requests.values().any(|request| request.base_index != 0) {
        return Err(invalid_generated(source, DETAIL_BASE_STYLE));
    }
    let registry = StyleRegistry::for_formats(requests.values().map(|request| &request.format))?;
    let indexes = requests
        .iter()
        .map(|(id, request)| (*id, registry.index(&request.format)))
        .collect();
    let bytes = registry.to_xml()?.into_bytes();
    enforce_bytes(bytes.len(), limits, source)?;
    Ok(DocumentStylePlan { bytes, indexes })
}

fn patch_existing_styles(
    bytes: &[u8],
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    limits: WriteLimits,
) -> Result<DocumentStylePlan, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let analysis = analyze_styles(bytes, source, limits)?;
    let mut additions = Vec::<XfTemplate>::new();
    let mut indexes = BTreeMap::new();
    let mut requested_custom = BTreeMap::<u32, String>::new();
    let mut pair_indexes = BTreeMap::<(usize, u32), usize>::new();
    for (id, request) in requests {
        let base = analysis
            .xfs
            .get(request.base_index)
            .ok_or_else(|| invalid_generated(source, DETAIL_BASE_STYLE))?;
        if request.format.id() >= 164 {
            let code = request
                .format
                .code()
                .ok_or_else(|| invalid_generated(source, DETAIL_CUSTOM_CONFLICT))?;
            if analysis
                .custom_formats
                .get(&request.format.id())
                .is_some_and(|existing| existing != code)
                || requested_custom
                    .get(&request.format.id())
                    .is_some_and(|existing| existing != code)
            {
                return Err(invalid_generated(source, DETAIL_CUSTOM_CONFLICT));
            }
            if !analysis.custom_formats.contains_key(&request.format.id()) {
                requested_custom.insert(request.format.id(), code.to_owned());
            }
        }
        if template_number_format(base, source)? == request.format.id() {
            indexes.insert(*id, request.base_index);
            continue;
        }
        let key = (request.base_index, request.format.id());
        let style_index = if let Some(index) = pair_indexes.get(&key) {
            *index
        } else {
            let index = analysis.xfs.len().saturating_add(additions.len());
            additions.push(patch_template(base, request.format.id(), source)?);
            pair_indexes.insert(key, index);
            index
        };
        indexes.insert(*id, style_index);
    }
    let output = rewrite_styles(
        bytes,
        source,
        &analysis,
        &requested_custom,
        &additions,
        limits,
    )?;
    Ok(DocumentStylePlan {
        bytes: output,
        indexes,
    })
}

fn analyze_styles(
    bytes: &[u8],
    source: &PartPath,
    limits: WriteLimits,
) -> Result<StyleAnalysis, XlsxWriteError> {
    let mut xml = configured_reader(bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut num_formats_depth = None;
    let mut cell_xfs_depth = None;
    let mut custom_formats = BTreeMap::new();
    let mut xfs = Vec::new();
    let mut capture = None::<(u64, Vec<Event<'static>>)>;
    let mut has_num_formats = false;
    let mut saw_cell_xfs = false;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match &event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if let Some((_, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                } else if element.local_name().as_ref() == b"numFmts" && depth == 2 {
                    has_num_formats = true;
                    num_formats_depth = Some(depth);
                } else if element.local_name().as_ref() == b"cellXfs" && depth == 2 {
                    saw_cell_xfs = true;
                    cell_xfs_depth = Some(depth);
                } else if num_formats_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"numFmt"
                {
                    insert_custom_format(element, &mut custom_formats, source)?;
                } else if cell_xfs_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"xf"
                {
                    capture = Some((depth, vec![event.clone().into_owned()]));
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if let Some((_, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                } else if element.local_name().as_ref() == b"numFmts" && depth + 1 == 2 {
                    has_num_formats = true;
                } else if element.local_name().as_ref() == b"cellXfs" && depth + 1 == 2 {
                    saw_cell_xfs = true;
                    cell_xfs_depth = Some(depth + 1);
                } else if num_formats_depth.is_some_and(|parent| depth + 1 == parent + 1)
                    && element.local_name().as_ref() == b"numFmt"
                {
                    insert_custom_format(element, &mut custom_formats, source)?;
                } else if cell_xfs_depth.is_some_and(|parent| depth + 1 == parent + 1)
                    && element.local_name().as_ref() == b"xf"
                {
                    xfs.push(XfTemplate {
                        events: vec![event.clone().into_owned()],
                    });
                }
            }
            Event::End(_) => {
                if let Some((capture_depth, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                    if depth == *capture_depth {
                        let (_, events) = capture
                            .take()
                            .ok_or_else(|| invalid_generated(source, DETAIL_MISSING_CELL_XFS))?;
                        xfs.push(XfTemplate { events });
                    }
                }
                if num_formats_depth == Some(depth) {
                    num_formats_depth = None;
                }
                if cell_xfs_depth == Some(depth) {
                    cell_xfs_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {
                if let Some((_, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                }
            }
        }
        buffer.clear();
    }
    if xfs.is_empty() || !saw_cell_xfs {
        return Err(invalid_generated(source, DETAIL_MISSING_CELL_XFS));
    }
    Ok(StyleAnalysis {
        custom_formats,
        xfs,
        has_num_formats,
    })
}

fn rewrite_styles(
    bytes: &[u8],
    source: &PartPath,
    analysis: &StyleAnalysis,
    custom_additions: &BTreeMap<u32, String>,
    xf_additions: &[XfTemplate],
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut num_formats_depth = None;
    let mut cell_xfs_depth = None;
    let mut inserted_num_formats = custom_additions.is_empty();
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 2 && !analysis.has_num_formats && !inserted_num_formats {
                    write_num_formats(&mut writer, custom_additions, source)?;
                    inserted_num_formats = true;
                }
                if depth == 2 && element.local_name().as_ref() == b"numFmts" {
                    num_formats_depth = Some(depth);
                    let patched = patch_count(
                        &element,
                        analysis
                            .custom_formats
                            .len()
                            .saturating_add(custom_additions.len()),
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if depth == 2 && element.local_name().as_ref() == b"cellXfs" {
                    cell_xfs_depth = Some(depth);
                    let patched = patch_count(
                        &element,
                        analysis.xfs.len().saturating_add(xf_additions.len()),
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if depth + 1 == 2 && !analysis.has_num_formats && !inserted_num_formats {
                    write_num_formats(&mut writer, custom_additions, source)?;
                    inserted_num_formats = true;
                }
                if depth + 1 == 2 && element.local_name().as_ref() == b"numFmts" {
                    let patched = patch_count(
                        &element,
                        analysis
                            .custom_formats
                            .len()
                            .saturating_add(custom_additions.len()),
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                    write_custom_formats(&mut writer, custom_additions, source)?;
                    write_event(&mut writer, Event::End(BytesEnd::new("numFmts")), source)?;
                    inserted_num_formats = true;
                } else if depth + 1 == 2 && element.local_name().as_ref() == b"cellXfs" {
                    let patched = patch_count(
                        &element,
                        analysis.xfs.len().saturating_add(xf_additions.len()),
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                    write_templates(&mut writer, xf_additions, source)?;
                    write_event(&mut writer, Event::End(BytesEnd::new("cellXfs")), source)?;
                    cell_xfs_depth = Some(depth + 1);
                } else {
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if num_formats_depth == Some(depth) {
                    write_custom_formats(&mut writer, custom_additions, source)?;
                    inserted_num_formats = true;
                    num_formats_depth = None;
                }
                if cell_xfs_depth == Some(depth) {
                    write_templates(&mut writer, xf_additions, source)?;
                    cell_xfs_depth = None;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if !inserted_num_formats || cell_xfs_depth.is_some() {
        return Err(invalid_generated(source, DETAIL_MISSING_CELL_XFS));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

fn patch_template(
    template: &XfTemplate,
    number_format_id: u32,
    source: &PartPath,
) -> Result<XfTemplate, XlsxWriteError> {
    let mut events = template.events.clone();
    let first = events
        .first_mut()
        .ok_or_else(|| invalid_generated(source, DETAIL_BASE_STYLE))?;
    match first {
        Event::Start(element) | Event::Empty(element) => {
            *element = patch_xf(element, number_format_id, source)?;
        }
        _ => return Err(invalid_generated(source, DETAIL_BASE_STYLE)),
    }
    Ok(XfTemplate { events })
}

fn patch_xf(
    element: &BytesStart<'_>,
    number_format_id: u32,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if !matches!(attribute.key.as_ref(), b"numFmtId" | b"applyNumberFormat") {
            patched.push_attribute(attribute);
        }
    }
    let id = number_format_id.to_string();
    patched.push_attribute(("numFmtId", id.as_str()));
    patched.push_attribute(("applyNumberFormat", "1"));
    Ok(patched.into_owned())
}

fn template_number_format(template: &XfTemplate, source: &PartPath) -> Result<u32, XlsxWriteError> {
    let element = match template.events.first() {
        Some(Event::Start(element) | Event::Empty(element)) => element,
        _ => return Err(invalid_generated(source, DETAIL_BASE_STYLE)),
    };
    required_u32_attribute(element, b"numFmtId", source)
}

fn insert_custom_format(
    element: &BytesStart<'_>,
    formats: &mut BTreeMap<u32, String>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let id = required_u32_attribute(element, b"numFmtId", source)?;
    let code = required_attribute(element, b"formatCode", source)?;
    if formats.insert(id, code).is_some() {
        return Err(invalid_generated(source, DETAIL_CUSTOM_CONFLICT));
    }
    Ok(())
}

fn write_num_formats(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    formats: &BTreeMap<u32, String>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let mut start = BytesStart::new("numFmts");
    let count = formats.len().to_string();
    start.push_attribute(("count", count.as_str()));
    write_event(writer, Event::Start(start), source)?;
    write_custom_formats(writer, formats, source)?;
    write_event(writer, Event::End(BytesEnd::new("numFmts")), source)
}

fn write_custom_formats(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    formats: &BTreeMap<u32, String>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    for (id, code) in formats {
        let mut format = BytesStart::new("numFmt");
        let id = id.to_string();
        format.push_attribute(("numFmtId", id.as_str()));
        let escaped_code = escape_attribute(code)?;
        format.push_attribute((b"formatCode".as_slice(), escaped_code.as_bytes()));
        write_event(writer, Event::Empty(format), source)?;
    }
    Ok(())
}

fn write_templates(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    templates: &[XfTemplate],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    for template in templates {
        for event in &template.events {
            write_event(writer, event.clone(), source)?;
        }
    }
    Ok(())
}

fn patch_count(
    element: &BytesStart<'_>,
    count: usize,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != b"count" {
            patched.push_attribute(attribute);
        }
    }
    let count = count.to_string();
    patched.push_attribute(("count", count.as_str()));
    Ok(patched.into_owned())
}

fn required_u32_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<u32, XlsxWriteError> {
    required_attribute(element, name, source)?
        .parse::<u32>()
        .map_err(|error| invalid_xml(source, error))
}

fn required_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<String, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == name {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map(|value| value.into_owned())
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Err(invalid_generated(source, DETAIL_BASE_STYLE))
}

fn configured_reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
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

fn decode_name<'a>(name: &'a [u8], source: &PartPath) -> Result<&'a str, XlsxWriteError> {
    std::str::from_utf8(name).map_err(|error| invalid_xml(source, error))
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
