use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::super::error::detail;
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use super::path::PartPath;

const ELEMENT_TYPES: &[u8] = b"Types";
const ELEMENT_DEFAULT: &[u8] = b"Default";
const ELEMENT_OVERRIDE: &[u8] = b"Override";
const ELEMENT_RELATIONSHIPS: &[u8] = b"Relationships";
const ELEMENT_RELATIONSHIP: &[u8] = b"Relationship";
const CONTENT_TYPES_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/content-types";
const RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Debug)]
pub(super) struct ContentTypes {
    defaults: BTreeMap<Box<str>, Box<str>>,
    overrides: BTreeMap<PartPath, Box<str>>,
}

impl ContentTypes {
    pub(super) fn content_type<'a>(&'a self, part: &PartPath) -> Option<&'a str> {
        self.overrides.get(part).map_or_else(
            || {
                part.extension().and_then(|extension| {
                    self.defaults
                        .get(extension.to_ascii_lowercase().as_str())
                        .map(AsRef::as_ref)
                })
            },
            |value| Some(value.as_ref()),
        )
    }
}

#[derive(Debug)]
pub(super) enum RelationshipTarget {
    Internal(PartPath),
    External(Box<str>),
}

#[derive(Debug)]
pub(super) struct Relationship {
    pub(super) id: Box<str>,
    pub(super) kind: Box<str>,
    pub(super) target: RelationshipTarget,
}

pub(super) fn parse_content_types(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<ContentTypes, XlsxReadError> {
    let mut reader = Reader::from_reader(bytes);
    configure(&mut reader);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut saw_root = false;
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| xml_error(source, XlsxErrorCode::InvalidXml).with_cause(error))?;
        match event {
            Event::Start(element) => {
                depth = checked_depth(depth, limits, source)?;
                let attributes = read_attributes(&element, &reader, limits, source)?;
                if depth == 1 {
                    require_root(
                        &element,
                        ELEMENT_TYPES,
                        CONTENT_TYPES_NAMESPACE,
                        &attributes,
                        source,
                    )?;
                    saw_root = true;
                } else if depth == 2 {
                    parse_content_type_element(
                        &element,
                        attributes,
                        source,
                        &mut defaults,
                        &mut overrides,
                    )?;
                }
            }
            Event::Empty(element) => {
                let element_depth = checked_depth(depth, limits, source)?;
                let attributes = read_attributes(&element, &reader, limits, source)?;
                if element_depth == 1 {
                    require_root(
                        &element,
                        ELEMENT_TYPES,
                        CONTENT_TYPES_NAMESPACE,
                        &attributes,
                        source,
                    )?;
                    saw_root = true;
                } else if element_depth == 2 {
                    parse_content_type_element(
                        &element,
                        attributes,
                        source,
                        &mut defaults,
                        &mut overrides,
                    )?;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    xml_error(source, XlsxErrorCode::InvalidXml)
                        .with_detail(detail::UNEXPECTED_CLOSING_ELEMENT)
                })?;
            }
            Event::DocType(_) => {
                return Err(xml_error(source, XlsxErrorCode::ForbiddenXmlConstruct));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if !saw_root || depth != 0 {
        return Err(xml_error(source, XlsxErrorCode::InvalidContentTypes));
    }
    Ok(ContentTypes {
        defaults,
        overrides,
    })
}

pub(super) fn parse_relationships(
    bytes: &[u8],
    relationship_part: &PartPath,
    source_part: Option<&PartPath>,
    limits: ReadLimits,
) -> Result<Vec<Relationship>, XlsxReadError> {
    let mut reader = Reader::from_reader(bytes);
    configure(&mut reader);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut saw_root = false;
    let mut relationships = BTreeMap::<Box<str>, Relationship>::new();

    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|error| {
            xml_error(relationship_part, XlsxErrorCode::InvalidXml).with_cause(error)
        })?;
        match event {
            Event::Start(element) => {
                depth = checked_depth(depth, limits, relationship_part)?;
                let attributes = read_attributes(&element, &reader, limits, relationship_part)?;
                if depth == 1 {
                    require_root(
                        &element,
                        ELEMENT_RELATIONSHIPS,
                        RELATIONSHIPS_NAMESPACE,
                        &attributes,
                        relationship_part,
                    )?;
                    saw_root = true;
                } else if depth == 2 && element.local_name().as_ref() == ELEMENT_RELATIONSHIP {
                    insert_relationship(
                        attributes,
                        relationship_part,
                        source_part,
                        &mut relationships,
                    )?;
                }
            }
            Event::Empty(element) => {
                let element_depth = checked_depth(depth, limits, relationship_part)?;
                let attributes = read_attributes(&element, &reader, limits, relationship_part)?;
                if element_depth == 1 {
                    require_root(
                        &element,
                        ELEMENT_RELATIONSHIPS,
                        RELATIONSHIPS_NAMESPACE,
                        &attributes,
                        relationship_part,
                    )?;
                    saw_root = true;
                } else if element_depth == 2
                    && element.local_name().as_ref() == ELEMENT_RELATIONSHIP
                {
                    insert_relationship(
                        attributes,
                        relationship_part,
                        source_part,
                        &mut relationships,
                    )?;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    xml_error(relationship_part, XlsxErrorCode::InvalidXml)
                        .with_detail(detail::UNEXPECTED_CLOSING_ELEMENT)
                })?;
            }
            Event::DocType(_) => {
                return Err(xml_error(
                    relationship_part,
                    XlsxErrorCode::ForbiddenXmlConstruct,
                ));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if !saw_root || depth != 0 {
        return Err(xml_error(
            relationship_part,
            XlsxErrorCode::InvalidRelationships,
        ));
    }
    Ok(relationships.into_values().collect())
}

fn configure(reader: &mut Reader<&[u8]>) {
    let config = reader.config_mut();
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.expand_empty_elements = false;
    config.trim_text(true);
}

fn checked_depth(
    current: u64,
    limits: ReadLimits,
    source: &PartPath,
) -> Result<u64, XlsxReadError> {
    let next = current.saturating_add(1);
    if next > limits.max_xml_depth() {
        return Err(xml_error(source, XlsxErrorCode::XmlDepthExceeded));
    }
    Ok(next)
}

fn require_root(
    element: &BytesStart<'_>,
    expected: &[u8],
    expected_namespace: &str,
    attributes: &BTreeMap<Box<str>, Box<str>>,
    source: &PartPath,
) -> Result<(), XlsxReadError> {
    if element.local_name().as_ref() != expected
        || attributes.get("xmlns").map(AsRef::as_ref) != Some(expected_namespace)
    {
        return Err(xml_error(source, XlsxErrorCode::InvalidXml)
            .with_detail(detail::UNEXPECTED_ROOT_ELEMENT));
    }
    Ok(())
}

fn read_attributes(
    element: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    limits: ReadLimits,
    source: &PartPath,
) -> Result<BTreeMap<Box<str>, Box<str>>, XlsxReadError> {
    let mut values = BTreeMap::new();
    for (index, attribute) in element.attributes().with_checks(true).enumerate() {
        if index as u64 >= limits.max_xml_attributes() {
            return Err(xml_error(source, XlsxErrorCode::XmlAttributesExceeded));
        }
        let attribute = attribute
            .map_err(|error| xml_error(source, XlsxErrorCode::InvalidXml).with_cause(error))?;
        let name = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|error| xml_error(source, XlsxErrorCode::InvalidXml).with_cause(error))?;
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| xml_error(source, XlsxErrorCode::InvalidXml).with_cause(error))?;
        values.insert(Box::<str>::from(name), value.into_owned().into_boxed_str());
    }
    Ok(values)
}

fn parse_content_type_element(
    element: &BytesStart<'_>,
    mut attributes: BTreeMap<Box<str>, Box<str>>,
    source: &PartPath,
    defaults: &mut BTreeMap<Box<str>, Box<str>>,
    overrides: &mut BTreeMap<PartPath, Box<str>>,
) -> Result<(), XlsxReadError> {
    match element.local_name().as_ref() {
        ELEMENT_DEFAULT => {
            let extension = required_attribute(&mut attributes, "Extension", source)?;
            let content_type = required_attribute(&mut attributes, "ContentType", source)?;
            let extension = extension.to_ascii_lowercase().into_boxed_str();
            if defaults.insert(extension, content_type).is_some() {
                return Err(xml_error(source, XlsxErrorCode::InvalidContentTypes)
                    .with_detail(detail::DUPLICATE_DEFAULT_EXTENSION));
            }
        }
        ELEMENT_OVERRIDE => {
            let part_name = required_attribute(&mut attributes, "PartName", source)?;
            let content_type = required_attribute(&mut attributes, "ContentType", source)?;
            let part = PartPath::from_content_type_override(&part_name)?;
            if overrides.insert(part, content_type).is_some() {
                return Err(xml_error(source, XlsxErrorCode::InvalidContentTypes)
                    .with_detail(detail::DUPLICATE_OVERRIDE_PART));
            }
        }
        _ => {}
    }
    Ok(())
}

fn insert_relationship(
    mut attributes: BTreeMap<Box<str>, Box<str>>,
    relationship_part: &PartPath,
    source_part: Option<&PartPath>,
    relationships: &mut BTreeMap<Box<str>, Relationship>,
) -> Result<(), XlsxReadError> {
    let id = required_attribute(&mut attributes, "Id", relationship_part)?;
    let kind = required_attribute(&mut attributes, "Type", relationship_part)?;
    let raw_target = required_attribute(&mut attributes, "Target", relationship_part)?;
    let target_mode = attributes.remove("TargetMode" as &str);
    let target = match target_mode.as_deref() {
        None | Some("Internal") => RelationshipTarget::Internal(
            PartPath::resolve_relationship(source_part, &raw_target).map_err(|error| {
                XlsxReadError::new(XlsxErrorCode::InvalidRelationshipTarget)
                    .with_detail(raw_target.to_string())
                    .at_source(relationship_part.source_id())
                    .with_cause(error)
            })?,
        ),
        Some(mode) if mode.eq_ignore_ascii_case("External") => {
            RelationshipTarget::External(raw_target)
        }
        Some(_) => {
            return Err(
                xml_error(relationship_part, XlsxErrorCode::InvalidRelationships)
                    .with_detail(detail::UNKNOWN_TARGET_MODE),
            );
        }
    };
    let relationship = Relationship {
        id: id.clone(),
        kind,
        target,
    };
    if relationships.insert(id, relationship).is_some() {
        return Err(
            xml_error(relationship_part, XlsxErrorCode::InvalidRelationships)
                .with_detail(detail::DUPLICATE_RELATIONSHIP_ID),
        );
    }
    Ok(())
}

fn required_attribute(
    attributes: &mut BTreeMap<Box<str>, Box<str>>,
    name: &str,
    source: &PartPath,
) -> Result<Box<str>, XlsxReadError> {
    let value = attributes.remove(name).ok_or_else(|| {
        xml_error(source, XlsxErrorCode::InvalidXml)
            .with_detail(format!("{} {name}", detail::MISSING_ATTRIBUTE))
    })?;
    if value.is_empty() {
        return Err(xml_error(source, XlsxErrorCode::InvalidXml)
            .with_detail(format!("{} {name}", detail::EMPTY_ATTRIBUTE)));
    }
    Ok(value)
}

fn xml_error(source: &PartPath, code: XlsxErrorCode) -> XlsxReadError {
    XlsxReadError::new(code).at_source(source.source_id())
}
