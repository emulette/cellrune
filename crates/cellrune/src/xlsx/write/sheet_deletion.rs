//! The package parts and workbook entries a sheet deletion removes from a preserved package.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

use super::package::PackageReader;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::{PartPath, relationship_type, scan_internal_relationships};
use crate::xlsx::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, namespace_reader,
    read_event,
};
use crate::{WorkbookSnapshot, XlsxDocument};

const ROOT_RELATIONSHIPS_PART: &[u8] = b"_rels/.rels";
const DETAIL_MISSING_SHEET_PART: &str = "source worksheet part mapping was not found";
const DETAIL_SLICER_CACHE: &str =
    "a preserved slicer or timeline cache may select a pivot table or table of a deleted sheet";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

/// What deleting worksheets removes from the preserved package.
#[derive(Debug, Default)]
pub(super) struct SheetDeletionPlan {
    /// Parts that only deleted worksheets reached, and the relationship parts of those parts.
    pub(super) removed_parts: BTreeSet<PartPath>,
    /// Workbook relationships to the deleted worksheets and to the pivot caches only their pivot
    /// tables used.
    pub(super) workbook_relationship_ids: BTreeSet<String>,
    /// The removed pivot caches' workbook relationships, whose `<pivotCache>` entries leave the
    /// workbook part.
    pub(super) pivot_cache_relationship_ids: BTreeSet<String>,
}

/// One relationship to a part, from its source part or, for `None`, from the package root.
struct Referrer {
    source: Option<PartPath>,
    id: Box<str>,
    kind: Box<str>,
}

/// Plans the removal of the source worksheets the draft deleted and of every part that only they
/// reached.
///
/// A part is removed once every part referring to it is removed, until nothing changes. A pivot
/// cache is referred to by the workbook and by its pivot tables; once all of those pivot tables
/// are removed, its workbook relationship and `<pivotCache>` entry go too and it is removed.
///
/// # Errors
///
/// Returns [`XlsxWriteErrorCode::UnsupportedPreservation`] when a slicer or timeline cache exists
/// and a deleted worksheet holds a pivot table or table, which the cache may select by name.
pub(super) fn plan_sheet_deletion(
    document: &XlsxDocument,
    draft: &WorkbookSnapshot,
    reader: &mut PackageReader<'_>,
) -> Result<SheetDeletionPlan, XlsxWriteError> {
    let deleted = document
        .workbook()
        .sheets()
        .iter()
        .filter(|sheet| draft.sheet_by_id(sheet.id()).is_none())
        .map(|sheet| {
            document
                .worksheet_part_path(sheet.id())
                .cloned()
                .ok_or_else(|| {
                    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                        .with_detail(DETAIL_MISSING_SHEET_PART)
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if deleted.is_empty() {
        return Ok(SheetDeletionPlan::default());
    }
    let referrers = package_referrers(document, reader)?;
    let workbook = document.workbook_part_path();
    let from_workbook = |referrer: &Referrer| referrer.source.as_ref() == Some(workbook);

    let caches = referrers
        .iter()
        .filter(|(_, referrers)| {
            referrers.iter().any(|referrer| {
                from_workbook(referrer)
                    && relationship_type::is_slicer_or_timeline_cache(&referrer.kind)
            })
        })
        .map(|(part, _)| part)
        .collect::<Vec<_>>();
    let deleted_selectable = referrers.values().flatten().any(|referrer| {
        referrer
            .source
            .as_ref()
            .is_some_and(|source| deleted.contains(source))
            && (relationship_type::is_pivot_table(&referrer.kind)
                || relationship_type::is_table(&referrer.kind))
    });
    if let Some(first) = caches.first()
        && deleted_selectable
    {
        let parts = caches
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(format!("{DETAIL_SLICER_CACHE}: {parts}"))
                .at_source(first.source_id()),
        );
    }

    let mut removed = deleted;
    loop {
        let before = removed.len();
        for (part, part_referrers) in &referrers {
            if removed.contains(part) || part == workbook {
                continue;
            }
            // The workbook lists every pivot cache, so a cache is used only by its pivot tables.
            let pivot_cache = part_referrers.iter().any(|referrer| {
                from_workbook(referrer)
                    && relationship_type::is_pivot_cache_definition(&referrer.kind)
            });
            let mut users = part_referrers
                .iter()
                .filter(|referrer| !(pivot_cache && from_workbook(referrer)))
                .peekable();
            if users.peek().is_some()
                && users.all(|referrer| {
                    referrer
                        .source
                        .as_ref()
                        .is_some_and(|source| removed.contains(source))
                })
            {
                removed.insert(part.clone());
            }
        }
        if removed.len() == before {
            break;
        }
    }

    let mut plan = SheetDeletionPlan::default();
    for part in &removed {
        for referrer in referrers.get(part).into_iter().flatten() {
            if from_workbook(referrer) {
                plan.workbook_relationship_ids
                    .insert(referrer.id.to_string());
                if relationship_type::is_pivot_cache_definition(&referrer.kind) {
                    plan.pivot_cache_relationship_ids
                        .insert(referrer.id.to_string());
                }
            }
        }
        let relationship_part = part
            .relationship_part()
            .map_err(|error| invalid_plan(part, error))?;
        if reader.contains(&relationship_part) {
            plan.removed_parts.insert(relationship_part);
        }
    }
    plan.removed_parts.extend(removed);
    Ok(plan)
}

/// Reads every relationship part of the package and returns, for each internal target, the
/// relationships that refer to it. Relationships are read leniently: one whose target cannot be a
/// part, or names a part the package does not hold, refers to nothing.
fn package_referrers(
    document: &XlsxDocument,
    reader: &mut PackageReader<'_>,
) -> Result<BTreeMap<PartPath, Vec<Referrer>>, XlsxWriteError> {
    let limits = document.read_options().limits();
    let mut sources = reader
        .parts()
        .map(|part| {
            part.relationship_part()
                .map(|relationship_part| (Some(part.clone()), relationship_part))
                .map_err(|error| invalid_plan(part, error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = PartPath::from_archive_name(ROOT_RELATIONSHIPS_PART)
        .map_err(|error| invalid_plan(document.workbook_part_path(), error))?;
    sources.push((None, root));
    let mut referrers = BTreeMap::<PartPath, Vec<Referrer>>::new();
    for (source, relationship_part) in sources {
        if !reader.contains(&relationship_part) {
            continue;
        }
        let bytes = reader.read_part(&relationship_part)?;
        let relationships =
            scan_internal_relationships(&bytes, &relationship_part, source.as_ref(), limits)
                .map_err(|error| {
                    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                        .at_source(relationship_part.source_id())
                        .with_cause(error)
                })?;
        // A relationship to a part the package does not hold, such as a dangling printer
        // settings target, has nothing to remove.
        for relationship in relationships
            .into_iter()
            .filter(|relationship| reader.contains(&relationship.target))
        {
            referrers
                .entry(relationship.target)
                .or_default()
                .push(Referrer {
                    source: source.clone(),
                    id: relationship.id,
                    kind: relationship.kind,
                });
        }
    }
    Ok(referrers)
}

/// Returns `<workbookView>` with `activeTab` and `firstSheet` moved past the deleted tabs: each
/// counts down by the deleted tabs before it, so a deleted active or first tab hands its role to
/// the tab now at its place, the last tab when it was last. `firstSheet` 0 is omitted as Excel
/// saves it.
pub(super) fn patch_workbook_view(
    element: &BytesStart<'_>,
    deleted_positions: &[usize],
    tab_count: usize,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let key = attribute.key.as_ref();
        let position = matches!(key, "activeTab" | "firstSheet")
            .then(|| {
                attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .flatten();
        let Some(position) = position else {
            patched.push_attribute(attribute);
            continue;
        };
        let moved = (position
            - deleted_positions
                .iter()
                .filter(|deleted| **deleted < position)
                .count())
        .min(tab_count.saturating_sub(1));
        if key == "firstSheet" && moved == 0 {
            continue;
        }
        let value = moved.to_string();
        patched.push_attribute((key, value.as_str()));
    }
    Ok(patched.into_owned())
}

/// Removes the `<pivotCache>` entries whose relationship is in `relationship_ids`, and the
/// `<pivotCaches>` container once it would be empty.
pub(super) fn remove_pivot_cache_entries(
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
    let mut skip_depth = None::<u64>;
    // The container and what precedes its first kept entry, held until one is kept.
    let mut container = None::<(u64, Vec<Event<'static>>, bool)>;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        if let Some(skip) = skip_depth {
            match event {
                Event::Start(_) => depth = depth.saturating_add(1),
                Event::End(_) => {
                    if depth == skip {
                        skip_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            buffer.clear();
            continue;
        }
        let entry_depth = container.as_ref().map(|(depth, _, _)| depth + 1);
        let event = match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 2 && element.local_name().as_ref() == "pivotCaches" {
                    container = Some((depth, vec![Event::Start(element.into_owned())], false));
                    buffer.clear();
                    continue;
                }
                if entry_depth == Some(depth) && removed_entry(&xml, &element, relationship_ids) {
                    skip_depth = Some(depth);
                    buffer.clear();
                    continue;
                }
                Event::Start(element.into_owned())
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if entry_depth == Some(depth + 1) && removed_entry(&xml, &element, relationship_ids)
                {
                    buffer.clear();
                    continue;
                }
                Event::Empty(element.into_owned())
            }
            Event::End(element) => {
                let closes_container = container
                    .as_ref()
                    .is_some_and(|(container_depth, _, _)| *container_depth == depth);
                depth = depth.saturating_sub(1);
                if closes_container {
                    if container.take().is_some_and(|(_, _, kept)| kept) {
                        write_event(&mut writer, Event::End(element.into_owned()), source)?;
                    }
                    buffer.clear();
                    continue;
                }
                Event::End(element.into_owned())
            }
            Event::Eof => break,
            other => other.into_owned(),
        };
        match &mut container {
            Some((_, held, kept)) if !*kept => {
                let entry = matches!(&event, Event::Start(_) | Event::Empty(_));
                held.push(event);
                if entry {
                    *kept = true;
                    for event in held.drain(..) {
                        write_event(&mut writer, event, source)?;
                    }
                }
            }
            _ => write_event(&mut writer, event, source)?,
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

/// Reports whether a `<pivotCache>` entry's relationship is one being removed.
fn removed_entry(
    xml: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
    relationship_ids: &BTreeSet<String>,
) -> bool {
    element.local_name().as_ref() == "pivotCache"
        && element.attributes().flatten().any(|attribute| {
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

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .with_detail(format!(
                    "{DETAIL_XML_DEPTH}: {depth} > {}",
                    limits.max_xml_depth()
                ))
                .at_source(source.source_id()),
        );
    }
    Ok(())
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

fn invalid_plan(
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

    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    use super::{patch_workbook_view, remove_pivot_cache_entries};
    use crate::WriteLimits;
    use crate::xlsx::package::PartPath;

    fn part() -> PartPath {
        PartPath::from_archive_name(b"xl/workbook.xml").expect("part")
    }

    fn view(xml: &str, deleted: &[usize], tab_count: usize) -> String {
        let mut reader = Reader::from_str(xml);
        let Event::Empty(element) = reader.read_event().expect("event") else {
            panic!("an empty workbookView");
        };
        let patched = patch_workbook_view(&element, deleted, tab_count, &part()).expect("view");
        let mut writer = quick_xml::Writer::new(Vec::new());
        writer
            .write_event(Event::Empty(patched))
            .expect("written view");
        String::from_utf8(writer.into_inner()).expect("UTF-8")
    }

    #[test]
    fn workbook_view_moves_past_deleted_tabs_as_excel_saves_it() {
        // The Excel probe: four tabs, `firstSheet="1" activeTab="2"`.
        let source = r#"<workbookView xWindow="0" firstSheet="1" activeTab="2"/>"#;
        assert_eq!(
            view(source, &[0], 3),
            r#"<workbookView xWindow="0" activeTab="1"/>"#
        );
        assert_eq!(
            view(source, &[1], 3),
            r#"<workbookView xWindow="0" firstSheet="1" activeTab="1"/>"#
        );
        assert_eq!(
            view(source, &[2], 3),
            r#"<workbookView xWindow="0" firstSheet="1" activeTab="2"/>"#
        );
        assert_eq!(
            view(source, &[3], 3),
            r#"<workbookView xWindow="0" firstSheet="1" activeTab="2"/>"#
        );
        // A deleted last active tab hands activity to the new last tab.
        assert_eq!(
            view(r#"<workbookView activeTab="3"/>"#, &[3], 3),
            r#"<workbookView activeTab="2"/>"#
        );
        assert_eq!(
            view(
                r#"<workbookView activeTab="3" firstSheet="2"/>"#,
                &[0, 1],
                2
            ),
            r#"<workbookView activeTab="1"/>"#
        );
    }

    #[test]
    fn removed_pivot_caches_leave_the_workbook_and_an_empty_container_goes_too() {
        let workbook = |caches: &str| {
            format!(
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets/>{caches}<extLst/></workbook>"#
            )
        };
        let remove = |xml: &str, ids: &[&str]| {
            let ids = ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<BTreeSet<_>>();
            String::from_utf8(
                remove_pivot_cache_entries(xml.as_bytes(), &part(), &ids, WriteLimits::default())
                    .expect("removed"),
            )
            .expect("UTF-8")
        };
        let two = workbook(
            r#"<pivotCaches> <pivotCache cacheId="1" r:id="rId5"/> <pivotCache cacheId="2" r:id="rId6"><extLst/></pivotCache></pivotCaches>"#,
        );
        assert_eq!(
            remove(&two, &["rId6"]),
            workbook(r#"<pivotCaches> <pivotCache cacheId="1" r:id="rId5"/> </pivotCaches>"#)
        );
        assert_eq!(
            remove(&two, &["rId5"]),
            workbook(
                r#"<pivotCaches>  <pivotCache cacheId="2" r:id="rId6"><extLst/></pivotCache></pivotCaches>"#
            )
        );
        assert_eq!(remove(&two, &["rId5", "rId6"]), workbook(""));
    }
}
