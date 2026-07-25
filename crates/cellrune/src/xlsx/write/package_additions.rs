use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::NsReader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;

const DETAIL_DUPLICATE_RELATIONSHIP: &str = "generated relationship ID already exists";
const DETAIL_DUPLICATE_CONTENT_TYPE: &str = "generated content type override already exists";
const DETAIL_RELATIONSHIP_ROOT: &str = "relationships root was not found";
const DETAIL_CONTENT_TYPES_ROOT: &str = "content-types root was not found";
const DETAIL_RELATIONSHIP_COUNT: &str = "max_relationships";
const DETAIL_CONTENT_TYPE_COUNT: &str = "max_content_types";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

pub(crate) struct NewRelationship {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
    pub(crate) target: String,
}

pub(crate) struct RelationshipIdAllocator {
    used: BTreeSet<String>,
}

impl RelationshipIdAllocator {
    pub(crate) fn from_xml(
        bytes: &[u8],
        source: &PartPath,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        enforce_bytes(bytes.len(), limits, source)?;
        let mut xml = configured_reader(bytes);
        let mut buffer = Vec::new();
        let mut depth = 0_u64;
        let mut root_depth = None;
        let mut used = BTreeSet::new();
        let mut count = 0_u64;
        loop {
            let event = xml
                .read_event_into(&mut buffer)
                .map_err(|error| invalid_xml(source, error))?;
            match event {
                Event::Start(element) => {
                    depth = depth.saturating_add(1);
                    enforce_depth(depth, limits, source)?;
                    if depth == 1 && element.local_name().as_ref() == b"Relationships" {
                        root_depth = Some(depth);
                    } else if root_depth.is_some_and(|root| depth == root + 1)
                        && element.local_name().as_ref() == b"Relationship"
                    {
                        used.insert(required_attribute(&element, b"Id", source)?);
                        count = count.saturating_add(1);
                        enforce_count(
                            DETAIL_RELATIONSHIP_COUNT,
                            count,
                            limits.max_relationships(),
                            source,
                        )?;
                    }
                }
                Event::Empty(element) => {
                    enforce_depth(depth.saturating_add(1), limits, source)?;
                    if root_depth.is_some_and(|root| depth + 1 == root + 1)
                        && element.local_name().as_ref() == b"Relationship"
                    {
                        used.insert(required_attribute(&element, b"Id", source)?);
                        count = count.saturating_add(1);
                        enforce_count(
                            DETAIL_RELATIONSHIP_COUNT,
                            count,
                            limits.max_relationships(),
                            source,
                        )?;
                    }
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                Event::Decl(_)
                | Event::Text(_)
                | Event::CData(_)
                | Event::Comment(_)
                | Event::PI(_)
                | Event::DocType(_)
                | Event::GeneralRef(_) => {}
            }
            buffer.clear();
        }
        if root_depth.is_none() {
            return Err(invalid_generated(source, DETAIL_RELATIONSHIP_ROOT));
        }
        Ok(Self { used })
    }

    pub(crate) fn allocate(&mut self, preferred: &str) -> String {
        if self.used.insert(preferred.to_owned()) {
            return preferred.to_owned();
        }
        let maximum_attempts = self.used.len().saturating_add(1);
        for suffix in 1..=maximum_attempts {
            let candidate = format!("{preferred}_{suffix}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
        }
        unreachable!("one more relationship ID candidate than used IDs must be available")
    }
}

pub(crate) fn append_relationships(
    bytes: &[u8],
    source: &PartPath,
    additions: &[NewRelationship],
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    if additions.is_empty() {
        return Ok(bytes.to_vec());
    }
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut root_depth = None;
    let mut root_name = None::<Vec<u8>>;
    let mut ids = BTreeSet::new();
    let mut count = 0_u64;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 1 && element.local_name().as_ref() == b"Relationships" {
                    root_depth = Some(depth);
                    root_name = Some(element.name().as_ref().to_vec());
                } else if root_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"Relationship"
                {
                    ids.insert(required_attribute(&element, b"Id", source)?);
                    count = count.saturating_add(1);
                }
                write_event(&mut writer, Event::Start(element.into_owned()), source)?;
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if root_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"Relationship"
                {
                    ids.insert(required_attribute(&element, b"Id", source)?);
                    count = count.saturating_add(1);
                }
                write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
            }
            Event::End(element) => {
                if root_depth == Some(depth) && element.local_name().as_ref() == b"Relationships" {
                    for relationship in additions {
                        if !ids.insert(relationship.id.clone()) {
                            return Err(invalid_generated(source, DETAIL_DUPLICATE_RELATIONSHIP));
                        }
                        count = count.saturating_add(1);
                        enforce_count(
                            DETAIL_RELATIONSHIP_COUNT,
                            count,
                            limits.max_relationships(),
                            source,
                        )?;
                        let name = qualified_sibling_name(
                            root_name.as_deref().unwrap_or(b"Relationships"),
                            b"Relationship",
                        );
                        let mut generated = BytesStart::new(decode_name(&name, source)?);
                        generated.push_attribute(("Id", relationship.id.as_str()));
                        generated.push_attribute(("Type", relationship.kind));
                        generated.push_attribute(("Target", relationship.target.as_str()));
                        write_event(&mut writer, Event::Empty(generated), source)?;
                    }
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if root_depth.is_none() {
        return Err(invalid_generated(source, DETAIL_RELATIONSHIP_ROOT));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

pub(crate) fn append_content_type_overrides(
    bytes: &[u8],
    source: &PartPath,
    additions: &BTreeMap<PartPath, &'static str>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    if additions.is_empty() {
        return Ok(bytes.to_vec());
    }
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut root_depth = None;
    let mut root_name = None::<Vec<u8>>;
    let mut parts = BTreeSet::new();
    let mut count = 0_u64;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 1 && element.local_name().as_ref() == b"Types" {
                    root_depth = Some(depth);
                    root_name = Some(element.name().as_ref().to_vec());
                } else if root_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"Override"
                {
                    parts.insert(required_attribute(&element, b"PartName", source)?);
                    count = count.saturating_add(1);
                }
                write_event(&mut writer, Event::Start(element.into_owned()), source)?;
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if root_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"Override"
                {
                    parts.insert(required_attribute(&element, b"PartName", source)?);
                    count = count.saturating_add(1);
                }
                write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
            }
            Event::End(element) => {
                if root_depth == Some(depth) && element.local_name().as_ref() == b"Types" {
                    for (part, content_type) in additions {
                        let part_name = format!("/{}", part.as_str());
                        if !parts.insert(part_name.clone()) {
                            return Err(invalid_generated(source, DETAIL_DUPLICATE_CONTENT_TYPE));
                        }
                        count = count.saturating_add(1);
                        enforce_count(
                            DETAIL_CONTENT_TYPE_COUNT,
                            count,
                            limits.max_content_types(),
                            source,
                        )?;
                        let name = qualified_sibling_name(
                            root_name.as_deref().unwrap_or(b"Types"),
                            b"Override",
                        );
                        let mut generated = BytesStart::new(decode_name(&name, source)?);
                        generated.push_attribute(("PartName", part_name.as_str()));
                        generated.push_attribute(("ContentType", *content_type));
                        write_event(&mut writer, Event::Empty(generated), source)?;
                    }
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if root_depth.is_none() {
        return Err(invalid_generated(source, DETAIL_CONTENT_TYPES_ROOT));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
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
    Err(invalid_generated(source, DETAIL_RELATIONSHIP_ROOT))
}

fn configured_reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn qualified_sibling_name(template: &[u8], local_name: &[u8]) -> Vec<u8> {
    let mut output = template
        .iter()
        .rposition(|byte| *byte == b':')
        .map_or_else(Vec::new, |index| template[..=index].to_vec());
    output.extend_from_slice(local_name);
    output
}

fn enforce_count(
    name: &'static str,
    actual: u64,
    maximum: u64,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual > maximum {
        Err(resource_error(source, name, actual, maximum))
    } else {
        Ok(())
    }
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
