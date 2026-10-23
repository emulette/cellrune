//! The parts and worksheet entries a table converted to a range leaves in a preserved package.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

use super::package::PackageReader;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::{PartPath, scan_internal_relationships};
use crate::xlsx::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, namespace_reader,
    read_event,
};
use crate::{WorkbookSnapshot, XlsxDocument};

const DETAIL_MISSING_PART: &str = "source table or worksheet part mapping was not found";
const DETAIL_TABLE_RELATIONSHIPS: &str =
    "a table converted to a range has relationships of its own, such as a query table";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

/// What converting tables to ranges removes from the preserved package.
#[derive(Debug, Default)]
pub(super) struct TableRemoval {
    /// The removed table parts.
    pub(super) parts: BTreeSet<PartPath>,
    /// For each worksheet part, its relationship part and the relationships to removed tables.
    pub(super) worksheets: BTreeMap<PartPath, (PartPath, BTreeSet<String>)>,
}

/// Plans the removal of the source tables the draft converted to ranges on sheets it kept. A
/// deleted sheet's tables leave with the sheet instead.
///
/// # Errors
///
/// Returns [`XlsxWriteErrorCode::UnsupportedPreservation`] for a table part with relationships of
/// its own, whose targets the conversion would leave unreachable.
pub(super) fn plan_table_removal(
    document: &XlsxDocument,
    draft: &WorkbookSnapshot,
    reader: &mut PackageReader<'_>,
) -> Result<TableRemoval, XlsxWriteError> {
    let mut removal = TableRemoval::default();
    let limits = document.read_options().limits();
    for sheet in document.workbook().sheets() {
        if draft.sheet_by_id(sheet.id()).is_none() {
            continue;
        }
        let converted = sheet
            .tables()
            .iter()
            .filter(|table| draft.table_by_id(table.id()).is_none())
            .map(|table| document.table_part_path(table.id()).cloned())
            .collect::<Option<BTreeSet<_>>>()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_PART))?;
        if converted.is_empty() {
            continue;
        }
        for part in &converted {
            let relationships = part
                .relationship_part()
                .map_err(|error| invalid_plan_with_cause(part, error))?;
            if reader.contains(&relationships) {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                        .with_detail(format!("{DETAIL_TABLE_RELATIONSHIPS}: {}", part.as_str()))
                        .at_source(part.source_id()),
                );
            }
        }
        let worksheet = document
            .worksheet_part_path(sheet.id())
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_PART))?;
        let relationship_part = worksheet
            .relationship_part()
            .map_err(|error| invalid_plan_with_cause(&worksheet, error))?;
        let bytes = reader.read_part(&relationship_part)?;
        let ids = scan_internal_relationships(&bytes, &relationship_part, Some(&worksheet), limits)
            .map_err(|error| invalid_plan_with_cause(&relationship_part, error))?
            .into_iter()
            .filter(|relationship| converted.contains(&relationship.target))
            .map(|relationship| relationship.id.into_string())
            .collect();
        removal
            .worksheets
            .insert(worksheet, (relationship_part, ids));
        removal.parts.extend(converted);
    }
    Ok(removal)
}

/// Removes the `<tablePart>` entries whose relationship is in `relationship_ids`, renumbering the
/// `<tableParts>` count or dropping the container once it would be empty.
pub(super) fn remove_table_parts(
    bytes: &[u8],
    source: &PartPath,
    relationship_ids: &BTreeSet<String>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut xml = namespace_reader(bytes, limits.max_xml_depth());
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    // The container's start tag and the entries it keeps, held until its end tag.
    let mut container = None::<(BytesStart<'static>, Vec<Event<'static>>, usize)>;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                if depth == 2 && element.local_name().as_ref() == "tableParts" {
                    container = Some((element.into_owned(), Vec::new(), 0));
                } else {
                    let event = Event::Start(element.into_owned());
                    match &mut container {
                        Some((_, held, _)) => held.push(event),
                        None => write_event(&mut writer, event, source)?,
                    }
                }
            }
            Event::Empty(element) => {
                let entry = container.is_some()
                    && depth == 2
                    && element.local_name().as_ref() == "tablePart";
                if entry && removed(&xml, &element, relationship_ids) {
                    buffer.clear();
                    continue;
                }
                let event = Event::Empty(element.into_owned());
                match &mut container {
                    Some((_, held, kept)) => {
                        *kept += usize::from(entry);
                        held.push(event);
                    }
                    None => write_event(&mut writer, event, source)?,
                }
            }
            Event::End(element) => {
                if depth == 2
                    && let Some((start, held, kept)) = container.take()
                {
                    if kept > 0 {
                        write_event(&mut writer, Event::Start(with_count(&start, kept)), source)?;
                        for event in held {
                            write_event(&mut writer, event, source)?;
                        }
                        write_event(&mut writer, Event::End(element.into_owned()), source)?;
                    }
                } else {
                    let event = Event::End(element.into_owned());
                    match &mut container {
                        Some((_, held, _)) => held.push(event),
                        None => write_event(&mut writer, event, source)?,
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => {
                let event = other.into_owned();
                match &mut container {
                    Some((_, held, _)) => held.push(event),
                    None => write_event(&mut writer, event, source)?,
                }
            }
        }
        buffer.clear();
    }
    let output = writer.into_inner().into_inner();
    if output.len() as u64 > limits.max_rewritten_xml_bytes() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .with_detail(format!(
                    "{DETAIL_XML_BYTES}: {} > {}",
                    output.len(),
                    limits.max_rewritten_xml_bytes()
                ))
                .at_source(source.source_id()),
        );
    }
    Ok(output)
}

fn with_count(element: &BytesStart<'_>, count: usize) -> BytesStart<'static> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    let count = count.to_string();
    for attribute in element.attributes().flatten() {
        if attribute.key.as_ref() == "count" {
            patched.push_attribute(("count", count.as_str()));
        } else {
            patched.push_attribute(attribute);
        }
    }
    patched.into_owned()
}

/// Reports whether a `<tablePart>` entry's relationship is one being removed.
fn removed(
    xml: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
    relationship_ids: &BTreeSet<String>,
) -> bool {
    element.attributes().flatten().any(|attribute| {
        let (namespace, local_name) = xml.resolver().resolve_attribute(attribute.key);
        local_name.as_ref() == "id"
            && matches!(
                namespace,
                ResolveResult::Bound(namespace)
                    if matches!(
                        namespace.as_ref(),
                        DOCUMENT_RELATIONSHIPS_TRANSITIONAL | DOCUMENT_RELATIONSHIPS_STRICT
                    )
            )
            && attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .is_ok_and(|value| relationship_ids.contains(value.as_ref()))
    })
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

fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn invalid_plan(detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_detail(detail)
}

fn invalid_plan_with_cause(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
        .at_source(source.source_id())
        .with_cause(cause)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::remove_table_parts;
    use crate::WriteLimits;
    use crate::xlsx::package::PartPath;

    #[test]
    fn removed_table_entries_leave_and_an_empty_container_goes_too() {
        let worksheet = |parts: &str| {
            format!(
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData/>{parts}<extLst/></worksheet>"#
            )
        };
        let remove = |xml: &str, ids: &[&str]| {
            let source = PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part");
            let ids = ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<BTreeSet<_>>();
            String::from_utf8(
                remove_table_parts(xml.as_bytes(), &source, &ids, WriteLimits::default())
                    .expect("removed"),
            )
            .expect("UTF-8")
        };
        let two = worksheet(
            r#"<tableParts count="2"><tablePart r:id="rId1"/><tablePart r:id="rId2"/></tableParts>"#,
        );
        assert_eq!(
            remove(&two, &["rId1"]),
            worksheet(r#"<tableParts count="1"><tablePart r:id="rId2"/></tableParts>"#)
        );
        assert_eq!(remove(&two, &["rId1", "rId2"]), worksheet(""));
        assert_eq!(remove(&two, &["rId9"]), two);
    }
}
