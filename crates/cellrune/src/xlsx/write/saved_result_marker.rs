//! The mark a minimal write leaves in `workbook.xml` so that a later read can tell its saved
//! results are as current as those of the file it was written from.
//!
//! A minimal write removes the saved results its edits can make out of date and asks the host to
//! recalculate on open, which xlsxwriter and other writers also ask while saving placeholder
//! results. The mark is an `ext` of the workbook's `extLst` holding a SHA-256 of the package with
//! the mark taken out: every part, in name order, with its name and content. A file another tool
//! changed afterwards no longer matches it.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use quick_xml::events::Event;

use super::grid_xml::{XmlDocument, XmlElement, XmlNode};
use super::package::PackageReader;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::XlsxDocument;
use crate::xlsx::package::PartPath;

/// The `ext` URI of the mark, which spreadsheet apps that do not know it ignore.
const MARK_URI: &str = "{6F1B9E52-2C7A-4E0D-9A43-5D8C11E2B7A1}";
const MARK_NAMESPACE: &str = "urn:cellrune:saved-results";
const PLACEHOLDER: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const DETAIL_NOT_UTF8: &str = "workbook part is not UTF-8";
const MAIN_NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// Returns `workbook` without the mark, and the digest the mark holds, when it has one.
pub(crate) fn split_mark(workbook: &[u8]) -> (Vec<u8>, Option<String>) {
    let Some((start, end)) = mark_span(workbook) else {
        return (workbook.to_vec(), None);
    };
    let digest = std::str::from_utf8(&workbook[start..end])
        .ok()
        .and_then(|mark| mark.split("sha256=\"").nth(1))
        .and_then(|rest| rest.get(..64))
        .map(str::to_owned);
    let mut without = workbook[..start].to_vec();
    without.extend_from_slice(&workbook[end..]);
    (without, digest)
}

/// Returns `workbook` with a mark over the package whose other parts `parts` returns by name, in
/// place of any mark it held. The mark joins the workbook's `extLst`, the root's last child, or a
/// new one.
pub(crate) fn marked(
    workbook: &[u8],
    workbook_part: &PartPath,
    parts: &BTreeMap<PartPath, Vec<u8>>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let (unmarked, _) = split_mark(workbook);
    let mut document = XmlDocument::parse(&unmarked, workbook_part, limits)?;
    let root = document.root_mut();
    let prefix = root.prefix();
    let mark = element(
        &format!(
            "<{prefix}ext uri=\"{MARK_URI}\" xmlns:cr=\"{MARK_NAMESPACE}\"><cr:savedResults sha256=\"{PLACEHOLDER}\"/></{prefix}ext>"
        ),
        &prefix,
        workbook_part,
        limits,
    )?;
    let list = root
        .children
        .iter()
        .rposition(|node| matches!(node, XmlNode::Element(element) if element.is("extLst")));
    match list {
        Some(index) => {
            let XmlNode::Element(list) = &mut root.children[index] else {
                unreachable!("the index names an element child");
            };
            list.children.retain(
                |node| !matches!(node, XmlNode::Other(Event::Text(text)) if text.is_empty()),
            );
            list.children.push(XmlNode::Element(mark));
        }
        None => {
            let mut list = element(
                &format!("<{prefix}extLst></{prefix}extLst>"),
                &prefix,
                workbook_part,
                limits,
            )?;
            list.children = vec![XmlNode::Element(mark)];
            root.children.push(XmlNode::Element(list));
        }
    }
    let written = String::from_utf8(document.write(workbook_part, limits)?)
        .map_err(|_| no_workbook_end(workbook_part))?;
    let digest = package_digest(&split_mark(written.as_bytes()).0, workbook_part, parts);
    let (start, end) =
        mark_span(written.as_bytes()).ok_or_else(|| no_workbook_end(workbook_part))?;
    let mut marked = written[..start].to_owned();
    marked.push_str(&written[start..end].replacen(PLACEHOLDER, &digest, 1));
    marked.push_str(&written[end..]);
    Ok(marked.into_bytes())
}

/// Parses one element whose names `prefix` qualifies in the main namespace.
fn element(
    xml: &str,
    prefix: &str,
    part: &PartPath,
    limits: WriteLimits,
) -> Result<XmlElement, XlsxWriteError> {
    let declared = prefix
        .strip_suffix(':')
        .map_or_else(String::new, |name| format!(":{name}"));
    let wrapped = format!("<wrapper xmlns{declared}=\"{MAIN_NAMESPACE}\">{xml}</wrapper>");
    let wrapper = XmlDocument::parse(wrapped.as_bytes(), part, limits)?.into_root();
    Ok(wrapper
        .children
        .into_iter()
        .find_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            XmlNode::Other(_) => None,
        })
        .expect("the wrapper holds the element"))
}

/// Returns whether `document`'s saved results are as current as when the app that calculated them
/// saved the file: the file does not ask its host to recalculate on open, or a minimal write that
/// asked it to left a mark the package still matches. A package that cannot be read is not.
pub(crate) fn saved_results_current(document: &XlsxDocument) -> bool {
    if document
        .workbook()
        .calculation_hints()
        .full_calculation_on_load()
        != Some(true)
    {
        return true;
    }
    document
        .preserved_package()
        .reader()
        .and_then(|mut reader| holds_current_mark(&mut reader, document.workbook_part_path()))
        .unwrap_or(false)
}

/// Returns whether the source package holds a mark that matches it.
fn holds_current_mark(
    reader: &mut PackageReader<'_>,
    workbook_part: &PartPath,
) -> Result<bool, XlsxWriteError> {
    let workbook = reader.read_part(workbook_part)?;
    let (unmarked, Some(digest)) = split_mark(&workbook) else {
        return Ok(false);
    };
    let mut parts = BTreeMap::new();
    for part in reader.parts().cloned().collect::<Vec<_>>() {
        if &part != workbook_part {
            let bytes = reader.read_part(&part)?;
            parts.insert(part, bytes);
        }
    }
    Ok(package_digest(&unmarked, workbook_part, &parts) == digest)
}

/// The hex SHA-256 of every part in name order, `workbook` standing for the workbook part.
fn package_digest(
    workbook: &[u8],
    workbook_part: &PartPath,
    parts: &BTreeMap<PartPath, Vec<u8>>,
) -> String {
    let mut all = parts
        .iter()
        .filter(|(part, _)| *part != workbook_part)
        .map(|(part, bytes)| (part, bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    all.insert(workbook_part, workbook);
    let mut hasher = Sha256::new();
    for (part, bytes) in all {
        hasher.update(part.as_str().as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The byte span of the mark's `ext` element, when the workbook holds one.
fn mark_span(workbook: &[u8]) -> Option<(usize, usize)> {
    let text = std::str::from_utf8(workbook).ok()?;
    let uri = text.rfind(&format!("uri=\"{MARK_URI}\""))?;
    let start = text[..uri].rfind('<')?;
    let prefix = text[start + 1..uri].trim_end().strip_suffix("ext")?;
    let closing = format!("</{prefix}ext>");
    let end = uri + text[uri..].find(&closing)? + closing.len();
    Some((start, end))
}

fn no_workbook_end(part: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(DETAIL_NOT_UTF8)
        .at_source(part.source_id())
}

#[cfg(test)]
#[path = "saved_result_marker_tests.rs"]
mod tests;
