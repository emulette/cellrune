//! An owned XML tree of one part, for the structural edit passes that reorder, drop, and rewrite
//! whole elements. Everything the passes do not change is written back as it was read.

use std::borrow::Cow;
use std::io::Cursor;

use quick_xml::escape::unescape;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Writer, XmlVersion};

use super::serialization::escape_text;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{namespace_reader, read_event};

const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";
const DETAIL_NO_ROOT: &str = "part has no root element";

/// One node of a part: an element, or any other event written back verbatim.
#[derive(Debug, Clone)]
pub(super) enum XmlNode {
    Element(XmlElement),
    Other(Event<'static>),
}

/// An element with its start tag and children.
#[derive(Debug, Clone)]
pub(super) struct XmlElement {
    start: BytesStart<'static>,
    pub(super) children: Vec<XmlNode>,
}

/// A whole part: the declaration and other top-level events around its root element.
#[derive(Debug, Clone)]
pub(super) struct XmlDocument {
    nodes: Vec<XmlNode>,
}

impl XmlDocument {
    pub(super) fn parse(
        bytes: &[u8],
        source: &PartPath,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        enforce_bytes(bytes.len(), limits, source)?;
        let mut xml = namespace_reader(bytes, limits.max_xml_depth());
        xml.config_mut().check_end_names = true;
        xml.config_mut().allow_unmatched_ends = false;
        xml.config_mut().expand_empty_elements = false;
        xml.config_mut().trim_text(false);
        let mut buffer = Vec::new();
        let mut stack = vec![Vec::<XmlNode>::new()];
        let mut starts = Vec::<BytesStart<'static>>::new();
        loop {
            let event =
                read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
            match event {
                Event::Start(start) => {
                    if starts.len() as u64 + 1 > limits.max_xml_depth() {
                        return Err(resource_error(
                            source,
                            DETAIL_XML_DEPTH,
                            starts.len() as u64 + 1,
                            limits.max_xml_depth(),
                        ));
                    }
                    starts.push(start.into_owned());
                    stack.push(Vec::new());
                }
                Event::Empty(start) => {
                    let element = XmlElement {
                        start: start.into_owned(),
                        children: Vec::new(),
                    };
                    stack
                        .last_mut()
                        .expect("the document level is never closed")
                        .push(XmlNode::Element(element));
                }
                Event::End(_) => {
                    let children = stack.pop().expect("an end tag closes an open element");
                    let start = starts.pop().expect("an end tag closes an open element");
                    let mut element = XmlElement { start, children };
                    // `<x></x>` keeps its form so untouched elements write back as they read.
                    if element.children.is_empty() {
                        element
                            .children
                            .push(XmlNode::Other(Event::Text(BytesText::new(""))));
                    }
                    stack
                        .last_mut()
                        .expect("the document level is never closed")
                        .push(XmlNode::Element(element));
                }
                Event::Eof => break,
                other => stack
                    .last_mut()
                    .expect("the document level is never closed")
                    .push(XmlNode::Other(other.into_owned())),
            }
            buffer.clear();
        }
        let nodes = stack.pop().expect("the document level remains");
        if !nodes.iter().any(|node| matches!(node, XmlNode::Element(_))) {
            return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
                .with_detail(DETAIL_NO_ROOT)
                .at_source(source.source_id()));
        }
        Ok(Self { nodes })
    }

    pub(super) fn root_mut(&mut self) -> &mut XmlElement {
        self.nodes
            .iter_mut()
            .find_map(|node| match node {
                XmlNode::Element(element) => Some(element),
                XmlNode::Other(_) => None,
            })
            .expect("a parsed document has a root element")
    }

    pub(super) fn write(
        &self,
        source: &PartPath,
        limits: WriteLimits,
    ) -> Result<Vec<u8>, XlsxWriteError> {
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        for node in &self.nodes {
            write_node(&mut writer, node, source)?;
        }
        let output = writer.into_inner().into_inner();
        enforce_bytes(output.len(), limits, source)?;
        Ok(output)
    }
}

impl XmlElement {
    /// Creates an empty element named like `sibling`'s namespace prefix and `local_name`.
    pub(super) fn sibling(sibling: &Self, local_name: &str) -> Self {
        let qualified = format!("{}{local_name}", sibling.prefix());
        Self {
            start: BytesStart::new(qualified),
            children: Vec::new(),
        }
    }

    /// Returns the element's namespace prefix with its colon, or nothing.
    pub(super) fn prefix(&self) -> String {
        let name = self.start.name().as_ref().to_owned();
        name.rfind(':')
            .map_or_else(String::new, |index| name[..=index].to_owned())
    }

    pub(super) fn is(&self, local_name: &str) -> bool {
        self.start.local_name().as_ref() == local_name
    }

    /// Returns the decoded value of the attribute spelled `key`, prefix included.
    pub(super) fn attribute(
        &self,
        key: &str,
        source: &PartPath,
    ) -> Result<Option<String>, XlsxWriteError> {
        for attribute in self.start.attributes().with_checks(true) {
            let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
            if attribute.key.as_ref() == key {
                return attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map(|value| Some(value.into_owned()))
                    .map_err(|error| invalid_xml(source, error));
            }
        }
        Ok(None)
    }

    /// Returns every attribute as decoded `(key, value)` pairs in source order.
    pub(super) fn attributes(
        &self,
        source: &PartPath,
    ) -> Result<Vec<(String, String)>, XlsxWriteError> {
        self.start
            .attributes()
            .with_checks(true)
            .map(|attribute| {
                let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
                let value = attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|error| invalid_xml(source, error))?;
                Ok((attribute.key.as_ref().to_owned(), value.into_owned()))
            })
            .collect()
    }

    /// Replaces the attributes with `attributes`, in their order.
    pub(super) fn set_attributes(&mut self, attributes: &[(String, String)]) {
        let mut start = BytesStart::new(self.start.name().as_ref().to_owned());
        for (key, value) in attributes {
            start.push_attribute((key.as_str(), value.as_str()));
        }
        self.start = start;
    }

    /// Sets, or with `None` removes, the attribute spelled `key`, keeping the others in place
    /// with their source spelling.
    pub(super) fn set_attribute(
        &mut self,
        key: &str,
        value: Option<&str>,
        source: &PartPath,
    ) -> Result<(), XlsxWriteError> {
        let mut start = BytesStart::new(self.start.name().as_ref().to_owned());
        let mut replaced = false;
        for attribute in self.start.attributes().with_checks(true) {
            let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
            if attribute.key.as_ref() == key {
                replaced = true;
                if let Some(value) = value {
                    start.push_attribute((key, value));
                }
            } else {
                start.push_attribute(attribute);
            }
        }
        if !replaced && let Some(value) = value {
            start.push_attribute((key, value));
        }
        self.start = start.into_owned();
        Ok(())
    }

    /// Returns the element's text, or `None` when it holds child elements, whose markup a
    /// rewritten text could not keep.
    pub(super) fn text(&self, source: &PartPath) -> Result<Option<String>, XlsxWriteError> {
        let mut text = String::new();
        for child in &self.children {
            match child {
                XmlNode::Element(_) => return Ok(None),
                XmlNode::Other(Event::Text(content)) => {
                    text.push_str(&content.xml_content(XmlVersion::Implicit1_0));
                }
                XmlNode::Other(Event::CData(content)) => {
                    text.push_str(&content.xml_content(XmlVersion::Implicit1_0));
                }
                XmlNode::Other(Event::GeneralRef(reference)) => {
                    let decoded = unescape(&format!("&{};", reference.as_ref()))
                        .map(Cow::into_owned)
                        .map_err(|error| invalid_xml(source, error))?;
                    text.push_str(&decoded);
                }
                XmlNode::Other(_) => return Ok(None),
            }
        }
        Ok(Some(text))
    }

    pub(super) fn set_text(&mut self, text: &str) -> Result<(), XlsxWriteError> {
        self.children = vec![XmlNode::Other(Event::Text(
            BytesText::from_escaped(escape_text(text)?).into_owned(),
        ))];
        Ok(())
    }

    pub(super) fn elements(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().filter_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            XmlNode::Other(_) => None,
        })
    }

    pub(super) fn elements_mut(&mut self) -> impl Iterator<Item = &mut Self> {
        self.children.iter_mut().filter_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            XmlNode::Other(_) => None,
        })
    }

    /// Returns the first child element with `local_name`.
    pub(super) fn child(&self, local_name: &str) -> Option<&Self> {
        self.elements().find(|element| element.is(local_name))
    }

    pub(super) fn child_mut(&mut self, local_name: &str) -> Option<&mut Self> {
        self.elements_mut().find(|element| element.is(local_name))
    }

    /// Keeps the child elements `keep` accepts, with the events between them.
    pub(super) fn retain_elements(&mut self, mut keep: impl FnMut(&Self) -> bool) {
        self.children.retain(|node| match node {
            XmlNode::Element(element) => keep(element),
            XmlNode::Other(_) => true,
        });
    }

    /// Returns every descendant element with `local_name`, in document order.
    pub(super) fn descendants_mut(&mut self, local_name: &str) -> Vec<&mut Self> {
        let mut found = Vec::new();
        for child in self.elements_mut() {
            if child.is(local_name) {
                found.push(child);
            } else {
                found.extend(child.descendants_mut(local_name));
            }
        }
        found
    }
}

fn write_node(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    node: &XmlNode,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    match node {
        XmlNode::Other(event) => write_event(writer, event.clone(), source),
        XmlNode::Element(element) => {
            if element.children.is_empty() {
                return write_event(writer, Event::Empty(element.start.clone()), source);
            }
            write_event(writer, Event::Start(element.start.clone()), source)?;
            for child in &element.children {
                write_node(writer, child, source)?;
            }
            write_event(
                writer,
                Event::End(BytesEnd::new(element.start.name().as_ref().to_owned())),
                source,
            )
        }
    }
}

fn write_event(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    event: Event<'_>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    writer
        .write_event(event)
        .map_err(|error| invalid_xml(source, error))
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

pub(super) fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
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
