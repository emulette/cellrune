//! Reads a legacy comments part into read-only cell comments.

use quick_xml::events::Event;

use super::super::package::PartPath;
use super::super::xml::{
    XmlBudget, decode_cdata, decode_reference, decode_text, is_spreadsheet_element,
    read_attributes, read_event, reader,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::{CellAddress, Comment};

#[derive(Debug)]
struct PendingComment {
    address: Option<CellAddress>,
    author: Option<usize>,
    text: String,
}

/// Parses `comments` in part order. Comments were not read before they were modeled, so the
/// caller drops them on any error instead of failing the read; a comment whose `ref` does not
/// parse is skipped.
pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<Vec<Comment>, XlsxReadError> {
    let mut xml = reader(bytes, limits);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidWorksheet);
    let mut stack = Vec::<Vec<u8>>::new();
    let mut authors = Vec::<Box<str>>::new();
    let mut author = None::<String>;
    let mut pending = None::<PendingComment>;
    let mut comments = Vec::new();
    loop {
        let event = read_event(&mut xml, &mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidWorksheet)
                .with_cause(error)
        })?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let depth = if empty {
                    budget.empty()?
                } else {
                    budget.start()?
                };
                let local_name = element.local_name().as_ref().as_bytes().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                if is_spreadsheet_element(&xml, element.name(), &budget)? {
                    match (depth, local_name.as_slice()) {
                        (3, b"author") if parent(&stack) == Some(b"authors") => {
                            author = Some(String::new());
                        }
                        (3, b"comment") if parent(&stack) == Some(b"commentList") => {
                            pending = Some(PendingComment {
                                address: attributes
                                    .unqualified("ref")
                                    .and_then(|value| CellAddress::from_a1(value).ok()),
                                author: attributes
                                    .unqualified("authorId")
                                    .and_then(|value| value.parse::<usize>().ok()),
                                text: String::new(),
                            });
                        }
                        _ => {}
                    }
                }
                if empty {
                    close(
                        &local_name,
                        &mut author,
                        &mut authors,
                        &mut pending,
                        &mut comments,
                    );
                } else {
                    stack.push(local_name);
                }
            }
            Event::Text(text) => append(&stack, &mut author, &mut pending, &decode_text(&text)),
            Event::CData(text) => append(&stack, &mut author, &mut pending, &decode_cdata(&text)),
            Event::GeneralRef(reference) => {
                let text = decode_reference(&reference, &budget)?;
                append(&stack, &mut author, &mut pending, &text);
            }
            Event::End(_) => {
                budget.end()?;
                let local_name = stack.pop().unwrap_or_default();
                close(
                    &local_name,
                    &mut author,
                    &mut authors,
                    &mut pending,
                    &mut comments,
                );
            }
            Event::DocType(_) => return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct)),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(comments
        .into_iter()
        .filter_map(|comment: PendingComment| {
            Some(Comment::from_xlsx(
                comment.address?,
                comment.author.and_then(|index| authors.get(index)).cloned(),
                comment.text.into_boxed_str(),
            ))
        })
        .collect())
}

fn parent(stack: &[Vec<u8>]) -> Option<&[u8]> {
    stack.last().map(Vec::as_slice)
}

/// Appends text inside an author, or inside a comment's `t` that is not a phonetic run.
fn append(
    stack: &[Vec<u8>],
    author: &mut Option<String>,
    pending: &mut Option<PendingComment>,
    text: &str,
) {
    if let Some(author) = author {
        author.push_str(text);
    } else if let Some(pending) = pending
        && parent(stack) == Some(b"t")
        && !stack.iter().any(|name| name.as_slice() == b"rPh")
    {
        pending.text.push_str(text);
    }
}

fn close(
    local_name: &[u8],
    author: &mut Option<String>,
    authors: &mut Vec<Box<str>>,
    pending: &mut Option<PendingComment>,
    comments: &mut Vec<PendingComment>,
) {
    match local_name {
        b"author" => {
            if let Some(author) = author.take() {
                authors.push(author.into_boxed_str());
            }
        }
        b"comment" => comments.extend(pending.take()),
        _ => {}
    }
}
