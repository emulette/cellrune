use std::collections::BTreeSet;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;

const DETAIL_DUPLICATE_CALC_CHAIN_RELATIONSHIP: &str =
    "workbook contains duplicate calculation-chain relationships";
const DETAIL_CALC_CHAIN_CONTENT_TYPE_MISSING: &str =
    "calculation-chain content type override was not found";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

pub(crate) struct CalculationChainPatch {
    pub(crate) relationship_bytes: Option<Vec<u8>>,
    pub(crate) removed_parts: BTreeSet<PartPath>,
}

pub(crate) fn remove_calculation_chain_relationship(
    bytes: &[u8],
    relationship_part: &PartPath,
    workbook_part: &PartPath,
    limits: WriteLimits,
) -> Result<CalculationChainPatch, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, relationship_part)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut root_depth = None;
    let mut skip_depth = None;
    let mut removed_parts = BTreeSet::new();
    let mut relationship_count = 0_u64;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(relationship_part, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, relationship_part)?;
                if skip_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if depth == 1 && element.local_name().as_ref() == b"Relationships" {
                    root_depth = Some(depth);
                    write_event(
                        &mut writer,
                        Event::Start(element.into_owned()),
                        relationship_part,
                    )?;
                } else if root_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"Relationship"
                {
                    relationship_count = relationship_count.saturating_add(1);
                    enforce_relationship_count(relationship_count, limits, relationship_part)?;
                    if let Some(target) = calc_chain_target(&element, relationship_part)? {
                        let part = PartPath::resolve_relationship(Some(workbook_part), &target)
                            .map_err(|error| invalid_xml(relationship_part, error))?;
                        if !removed_parts.insert(part) {
                            return Err(invalid_generated(
                                relationship_part,
                                DETAIL_DUPLICATE_CALC_CHAIN_RELATIONSHIP,
                            ));
                        }
                        skip_depth = Some(depth);
                    } else {
                        write_event(
                            &mut writer,
                            Event::Start(element.into_owned()),
                            relationship_part,
                        )?;
                    }
                } else {
                    write_event(
                        &mut writer,
                        Event::Start(element.into_owned()),
                        relationship_part,
                    )?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, relationship_part)?;
                if skip_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if root_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"Relationship"
                {
                    relationship_count = relationship_count.saturating_add(1);
                    enforce_relationship_count(relationship_count, limits, relationship_part)?;
                    if let Some(target) = calc_chain_target(&element, relationship_part)? {
                        let part = PartPath::resolve_relationship(Some(workbook_part), &target)
                            .map_err(|error| invalid_xml(relationship_part, error))?;
                        if !removed_parts.insert(part) {
                            return Err(invalid_generated(
                                relationship_part,
                                DETAIL_DUPLICATE_CALC_CHAIN_RELATIONSHIP,
                            ));
                        }
                    } else {
                        write_event(
                            &mut writer,
                            Event::Empty(element.into_owned()),
                            relationship_part,
                        )?;
                    }
                } else {
                    write_event(
                        &mut writer,
                        Event::Empty(element.into_owned()),
                        relationship_part,
                    )?;
                }
            }
            Event::End(element) => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                } else if skip_depth.is_none() {
                    write_event(
                        &mut writer,
                        Event::End(element.into_owned()),
                        relationship_part,
                    )?;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => {
                if skip_depth.is_none() {
                    write_event(&mut writer, other.into_owned(), relationship_part)?;
                }
            }
        }
        buffer.clear();
    }
    if removed_parts.is_empty() {
        return Ok(CalculationChainPatch {
            relationship_bytes: None,
            removed_parts,
        });
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, relationship_part)?;
    Ok(CalculationChainPatch {
        relationship_bytes: Some(output),
        removed_parts,
    })
}

pub(crate) fn remove_content_type_overrides(
    bytes: &[u8],
    content_types_part: &PartPath,
    removals: &BTreeSet<PartPath>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, content_types_part)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut found = BTreeSet::new();
    let mut declaration_count = 0_u64;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(content_types_part, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, content_types_part)?;
                write_event(
                    &mut writer,
                    Event::Start(element.into_owned()),
                    content_types_part,
                )?;
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, content_types_part)?;
                if matches!(element.local_name().as_ref(), b"Default" | b"Override") {
                    declaration_count = declaration_count.saturating_add(1);
                    if declaration_count > limits.max_content_types() {
                        return Err(resource_error(
                            content_types_part,
                            "max_content_types",
                            declaration_count,
                            limits.max_content_types(),
                        ));
                    }
                }
                let removed = if element.local_name().as_ref() == b"Override" {
                    let part_name = required_attribute(&element, b"PartName", content_types_part)?;
                    let part = PartPath::from_content_type_override(&part_name)
                        .map_err(|error| invalid_xml(content_types_part, error))?;
                    if removals.contains(&part) {
                        found.insert(part);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !removed {
                    write_event(
                        &mut writer,
                        Event::Empty(element.into_owned()),
                        content_types_part,
                    )?;
                }
            }
            Event::End(element) => {
                write_event(
                    &mut writer,
                    Event::End(element.into_owned()),
                    content_types_part,
                )?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), content_types_part)?,
        }
        buffer.clear();
    }
    if let Some(missing) = removals.iter().find(|part| !found.contains(*part)) {
        return Err(
            invalid_generated(content_types_part, DETAIL_CALC_CHAIN_CONTENT_TYPE_MISSING)
                .at_source(missing.source_id()),
        );
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, content_types_part)?;
    Ok(output)
}

fn calc_chain_target(
    element: &quick_xml::events::BytesStart<'_>,
    source: &PartPath,
) -> Result<Option<String>, XlsxWriteError> {
    let relationship_type = required_attribute(element, b"Type", source)?;
    if !crate::xlsx::package::relationship_type::is_calc_chain(&relationship_type)
        || optional_attribute(element, b"TargetMode", source)?.as_deref() == Some("External")
    {
        return Ok(None);
    }
    Ok(Some(required_attribute(element, b"Target", source)?))
}

fn optional_attribute(
    element: &quick_xml::events::BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<Option<String>, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == name {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map(|value| Some(value.into_owned()))
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Ok(None)
}

fn required_attribute(
    element: &quick_xml::events::BytesStart<'_>,
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
    Err(invalid_generated(
        source,
        DETAIL_DUPLICATE_CALC_CHAIN_RELATIONSHIP,
    ))
}

fn enforce_relationship_count(
    actual: u64,
    limits: WriteLimits,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual > limits.max_relationships() {
        return Err(resource_error(
            source,
            "max_relationships",
            actual,
            limits.max_relationships(),
        ));
    }
    Ok(())
}

fn enforce_depth(
    actual: u64,
    limits: WriteLimits,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual > limits.max_xml_depth() {
        return Err(resource_error(
            source,
            DETAIL_XML_DEPTH,
            actual,
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
#[path = "package_metadata_patch_tests.rs"]
mod tests;
