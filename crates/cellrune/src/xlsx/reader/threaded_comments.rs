//! Reads Excel 365 threaded comments, whose legacy comments part holds only a placeholder note.

use std::collections::BTreeMap;

use quick_xml::events::Event;

use super::super::package::PartPath;
use super::super::xml::{
    XmlBudget, decode_cdata, decode_reference, decode_text, read_attributes, read_event, reader,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::{CellAddress, Comment, CommentReply};

/// Parses a persons part into display names by person ID.
pub(super) fn parse_persons(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<BTreeMap<Box<str>, Box<str>>, XlsxReadError> {
    let mut persons = BTreeMap::new();
    walk(bytes, source, limits, |event| {
        if let Element::Start { name, attributes } = event
            && name == b"person"
            && let (Some(id), Some(display_name)) = (attributes.id, attributes.display_name)
        {
            persons.insert(id, display_name);
        }
    })?;
    Ok(persons)
}

/// Parses a threaded comments part into one comment per thread, in part order, whose replies are
/// the thread's other comments. A comment whose `ref` does not parse, or a reply whose thread is
/// missing, is skipped.
pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
    persons: &BTreeMap<Box<str>, Box<str>>,
) -> Result<Vec<Comment>, XlsxReadError> {
    struct Thread {
        address: CellAddress,
        author: Option<Box<str>>,
        text: String,
        replies: Vec<CommentReply>,
    }
    struct Pending {
        address: Option<CellAddress>,
        id: Option<Box<str>>,
        parent: Option<Box<str>>,
        author: Option<Box<str>>,
        text: String,
    }
    let mut threads = Vec::<Thread>::new();
    let mut thread_index = BTreeMap::<Box<str>, usize>::new();
    let mut pending = None::<Pending>;
    let mut in_text = false;
    walk(bytes, source, limits, |event| match event {
        Element::Start { name, attributes } if name == b"threadedComment" => {
            pending = Some(Pending {
                address: attributes
                    .reference
                    .and_then(|value| CellAddress::from_a1(&value).ok()),
                id: attributes.id,
                parent: attributes.parent_id,
                author: attributes
                    .person_id
                    .and_then(|person| persons.get(&person).cloned()),
                text: String::new(),
            });
        }
        Element::Start { name, .. } if name == b"text" => in_text = true,
        Element::Text(text) if in_text => {
            if let Some(pending) = &mut pending {
                pending.text.push_str(&text);
            }
        }
        Element::End(name) if name == b"text" => in_text = false,
        Element::End(name) if name == b"threadedComment" => {
            let Some(comment) = pending.take() else {
                return;
            };
            match comment.parent {
                Some(parent) => {
                    if let Some(&index) = thread_index.get(&parent) {
                        threads[index].replies.push(CommentReply::from_xlsx(
                            comment.author,
                            comment.text.into_boxed_str(),
                        ));
                    }
                }
                None => {
                    if let Some(address) = comment.address {
                        if let Some(id) = comment.id {
                            thread_index.insert(id, threads.len());
                        }
                        threads.push(Thread {
                            address,
                            author: comment.author,
                            text: comment.text,
                            replies: Vec::new(),
                        });
                    }
                }
            }
        }
        _ => {}
    })?;
    Ok(threads
        .into_iter()
        .map(|thread| {
            Comment::from_xlsx(thread.address, thread.author, thread.text.into_boxed_str())
                .with_replies(thread.replies)
        })
        .collect())
}

#[derive(Default)]
struct Attributes {
    reference: Option<String>,
    id: Option<Box<str>>,
    parent_id: Option<Box<str>>,
    person_id: Option<Box<str>>,
    display_name: Option<Box<str>>,
}

enum Element<'a> {
    Start {
        name: &'a [u8],
        attributes: Attributes,
    },
    Text(String),
    End(&'a [u8]),
}

/// Reads every element and text of a part, starting and then ending an empty element.
fn walk(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
    mut visit: impl FnMut(Element<'_>),
) -> Result<(), XlsxReadError> {
    let mut xml = reader(bytes, limits);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidWorksheet);
    let mut stack = Vec::<Vec<u8>>::new();
    loop {
        let event = read_event(&mut xml, &mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidWorksheet)
                .with_cause(error)
        })?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                if empty {
                    budget.empty()?;
                } else {
                    budget.start()?;
                }
                let name = element.local_name().as_ref().as_bytes().to_vec();
                let read = read_attributes(&element, &xml, &budget)?;
                let attribute = |key: &str| read.unqualified(key).map(Box::<str>::from);
                visit(Element::Start {
                    name: &name,
                    attributes: Attributes {
                        reference: read.unqualified("ref").map(str::to_owned),
                        id: attribute("id"),
                        parent_id: attribute("parentId"),
                        person_id: attribute("personId"),
                        display_name: attribute("displayName"),
                    },
                });
                if empty {
                    visit(Element::End(&name));
                } else {
                    stack.push(name);
                }
            }
            Event::Text(text) => visit(Element::Text(decode_text(&text))),
            Event::CData(text) => visit(Element::Text(decode_cdata(&text))),
            Event::GeneralRef(reference) => {
                visit(Element::Text(decode_reference(&reference, &budget)?));
            }
            Event::End(_) => {
                budget.end()?;
                let name = stack.pop().unwrap_or_default();
                visit(Element::End(&name));
            }
            Event::DocType(_) => return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct)),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}
