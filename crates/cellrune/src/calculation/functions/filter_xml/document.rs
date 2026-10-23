//! The XPath 1.0 data model of one XML document, read with quick-xml.
//!
//! Nodes are stored in document order: an element, then its attributes, then its content, so
//! comparing node indexes compares document order.

use quick_xml::XmlVersion;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

pub(super) type NodeId = usize;

pub(super) const ROOT: NodeId = 0;

/// Why a document cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentError {
    /// The text is not well-formed namespace-aware XML.
    Invalid,
    /// The document declares a DTD, whose effect on entity expansion and defaults is not
    /// modeled.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Name {
    pub(super) prefix: Option<String>,
    pub(super) local: String,
    pub(super) namespace: Option<String>,
}

impl Name {
    /// The name as written, with its prefix.
    pub(super) fn qualified(&self) -> String {
        match &self.prefix {
            Some(prefix) => format!("{prefix}:{}", self.local),
            None => self.local.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NodeKind {
    Root,
    Element(Name),
    Attribute(Name, String),
    Text(String),
    Comment(String),
    ProcessingInstruction { target: String, data: String },
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) kind: NodeKind,
    pub(super) parent: Option<NodeId>,
    pub(super) children: Vec<NodeId>,
    pub(super) attributes: Vec<NodeId>,
    /// The last node of this node's subtree, attributes included, in document order.
    pub(super) subtree_end: NodeId,
}

#[derive(Debug)]
pub(super) struct Document {
    nodes: Vec<Node>,
}

impl Document {
    pub(super) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Reads `xml` as a well-formed XML 1.0 document with namespaces. Adjacent text, CDATA
    /// sections, and references form one text node, as in the XPath data model.
    pub(super) fn parse(xml: &str) -> Result<Self, DocumentError> {
        let mut reader = NsReader::from_str(xml);
        let config = reader.config_mut();
        config.check_end_names = true;
        config.allow_unmatched_ends = false;
        config.check_comments = true;
        config.expand_empty_elements = false;
        config.trim_text(false);

        let mut builder = Builder {
            nodes: vec![Node {
                kind: NodeKind::Root,
                parent: None,
                children: Vec::new(),
                attributes: Vec::new(),
                subtree_end: ROOT,
            }],
            open: vec![ROOT],
            text: None,
            has_root_element: false,
        };
        loop {
            let event = reader.read_event().map_err(|_| DocumentError::Invalid)?;
            match event {
                Event::Start(element) => builder.element(&reader, &element, true)?,
                Event::Empty(element) => builder.element(&reader, &element, false)?,
                Event::End(_) => {
                    builder.flush_text();
                    builder.close();
                }
                Event::Text(text) => builder.text(&text.xml_content(XmlVersion::Implicit1_0))?,
                Event::CData(text) => builder.text(&text.xml_content(XmlVersion::Implicit1_0))?,
                Event::GeneralRef(reference) => builder.text(&reference_text(&reference)?)?,
                Event::Comment(text) => {
                    let comment = text.xml_content(XmlVersion::Implicit1_0).into_owned();
                    builder.leaf(NodeKind::Comment(comment))?;
                }
                Event::PI(instruction) => {
                    let target = instruction.target();
                    if !is_name(target)
                        || target.contains(':')
                        || target.eq_ignore_ascii_case("xml")
                    {
                        return Err(DocumentError::Invalid);
                    }
                    let data = instruction.content().trim_start().to_owned();
                    builder.leaf(NodeKind::ProcessingInstruction {
                        target: target.to_owned(),
                        data,
                    })?;
                }
                Event::Decl(_) => {}
                Event::DocType(_) => return Err(DocumentError::Unsupported),
                Event::Eof => break,
            }
        }
        if builder.open.len() != 1 || !builder.has_root_element {
            return Err(DocumentError::Invalid);
        }
        builder.nodes[ROOT].subtree_end = builder.nodes.len() - 1;
        Ok(Self {
            nodes: builder.nodes,
        })
    }
}

struct Builder {
    nodes: Vec<Node>,
    open: Vec<NodeId>,
    text: Option<String>,
    has_root_element: bool,
}

impl Builder {
    fn current(&self) -> NodeId {
        *self.open.last().expect("the root stays open")
    }

    fn push(&mut self, kind: NodeKind, parent: NodeId) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
            attributes: Vec::new(),
            subtree_end: id,
        });
        id
    }

    fn flush_text(&mut self) {
        if let Some(text) = self.text.take() {
            let parent = self.current();
            let id = self.push(NodeKind::Text(text), parent);
            self.nodes[parent].children.push(id);
        }
    }

    fn text(&mut self, text: &str) -> Result<(), DocumentError> {
        if !text.chars().all(is_xml_character) {
            return Err(DocumentError::Invalid);
        }
        if self.open.len() == 1 {
            // Outside the document element only white space may appear.
            return if text.chars().all(is_xml_space) {
                Ok(())
            } else {
                Err(DocumentError::Invalid)
            };
        }
        self.text.get_or_insert_with(String::new).push_str(text);
        Ok(())
    }

    fn leaf(&mut self, kind: NodeKind) -> Result<(), DocumentError> {
        self.flush_text();
        let parent = self.current();
        let id = self.push(kind, parent);
        self.nodes[parent].children.push(id);
        Ok(())
    }

    fn element(
        &mut self,
        reader: &NsReader<&[u8]>,
        element: &BytesStart<'_>,
        opens: bool,
    ) -> Result<(), DocumentError> {
        self.flush_text();
        if self.open.len() == 1 {
            if self.has_root_element {
                return Err(DocumentError::Invalid);
            }
            self.has_root_element = true;
        }
        let name = resolve(reader, element.name(), true)?;
        let parent = self.current();
        let id = self.push(NodeKind::Element(name), parent);
        self.nodes[parent].children.push(id);
        for attribute in element.attributes().with_checks(true) {
            let attribute = attribute.map_err(|_| DocumentError::Invalid)?;
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|_| DocumentError::Invalid)?;
            if !value.chars().all(is_xml_character) {
                return Err(DocumentError::Invalid);
            }
            if attribute.key.as_namespace_binding().is_some() {
                continue;
            }
            let name = resolve(reader, attribute.key, false)?;
            let duplicate = self.nodes[id].attributes.iter().any(|existing| {
                matches!(&self.nodes[*existing].kind, NodeKind::Attribute(other, _)
                    if other.local == name.local && other.namespace == name.namespace)
            });
            if duplicate {
                return Err(DocumentError::Invalid);
            }
            let attribute = self.push(NodeKind::Attribute(name, value.into_owned()), id);
            self.nodes[id].attributes.push(attribute);
        }
        if opens {
            self.open.push(id);
        } else {
            self.nodes[id].subtree_end = self.nodes.len() - 1;
        }
        Ok(())
    }

    fn close(&mut self) {
        if let Some(id) = self.open.pop() {
            self.nodes[id].subtree_end = self.nodes.len() - 1;
        }
    }
}

/// Resolves an element or attribute name; a prefix bound to no namespace makes the document
/// invalid, as Microsoft documents for FILTERXML.
fn resolve(
    reader: &NsReader<&[u8]>,
    name: quick_xml::name::QName<'_>,
    element: bool,
) -> Result<Name, DocumentError> {
    let written = name.as_ref();
    let mut parts = written.split(':');
    let valid = match (parts.next(), parts.next(), parts.next()) {
        (Some(local), None, _) => is_name(local),
        (Some(prefix), Some(local), None) => is_name(prefix) && is_name(local),
        _ => false,
    };
    if !valid {
        return Err(DocumentError::Invalid);
    }
    let resolver = reader.resolver();
    let (namespace, local) = if element {
        resolver.resolve_element(name)
    } else {
        resolver.resolve_attribute(name)
    };
    let namespace = match namespace {
        ResolveResult::Unbound => None,
        ResolveResult::Bound(namespace) => Some(namespace.as_ref().to_owned()),
        ResolveResult::Unknown(_) => return Err(DocumentError::Invalid),
    };
    Ok(Name {
        prefix: name.prefix().map(|prefix| prefix.as_ref().to_owned()),
        local: local.as_ref().to_owned(),
        namespace,
    })
}

fn reference_text(reference: &BytesRef<'_>) -> Result<String, DocumentError> {
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|_| DocumentError::Invalid)?
    {
        return if is_xml_character(character) {
            Ok(character.to_string())
        } else {
            Err(DocumentError::Invalid)
        };
    }
    match reference.as_ref() {
        "lt" => Ok("<".to_owned()),
        "gt" => Ok(">".to_owned()),
        "amp" => Ok("&".to_owned()),
        "apos" => Ok("'".to_owned()),
        "quot" => Ok("\"".to_owned()),
        _ => Err(DocumentError::Invalid),
    }
}

/// XML 1.0 white space.
pub(super) const fn is_xml_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r')
}

const fn is_xml_character(character: char) -> bool {
    matches!(
        character as u32,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

/// An XML 1.0 `NCName`: a name without a colon.
pub(super) fn is_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(is_name_start_character) && characters.all(is_name_character)
}

fn is_name_start_character(character: char) -> bool {
    matches!(
        character,
        'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{00C0}'..='\u{00D6}'
            | '\u{00D8}'..='\u{00F6}'
            | '\u{00F8}'..='\u{02FF}'
            | '\u{0370}'..='\u{037D}'
            | '\u{037F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

pub(super) fn is_name_character(character: char) -> bool {
    is_name_start_character(character)
        || matches!(
            character,
            '-' | '.' | '0'..='9' | '\u{00B7}' | '\u{0300}'..='\u{036F}' | '\u{203F}'..='\u{2040}'
        )
}
