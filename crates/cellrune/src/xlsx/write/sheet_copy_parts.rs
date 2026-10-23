//! The parts a copied sheet gets in a preserved package: a duplicate of every part its source
//! sheet's worksheet reaches through relationships, with the relationships pointing at the
//! duplicates, as Excel's Move or Copy writes them.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};

use super::package::PackageReader;
use super::table_patch::{patch_table_xml, remove_table_columns};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::{ContentTypes, PartPath};
use crate::xlsx::xml::{namespace_reader, read_event};
use crate::{SheetId, Table};

const DETAIL_UNDUPLICATABLE: &str = "a sheet copy cannot duplicate a part its source sheet reaches";
const DETAIL_MISSING_TABLE: &str = "a copied table part has no table in the copied sheet";

/// Relationship types whose parts a copy cannot duplicate faithfully: pivot tables and their
/// caches, form controls, threaded comments, slicers, timelines, query tables, and ActiveX
/// controls, which name sheets, caches, or people outside the copied sheet.
const UNDUPLICATABLE: &[&str] = &[
    "pivotTable",
    "ctrlProp",
    "threadedComment",
    "slicer",
    "timeline",
    "queryTable",
    "control",
];

/// The duplicated parts of one copied sheet.
#[derive(Debug, Default)]
pub(super) struct SheetCopyParts {
    /// Each duplicated part other than the worksheet, by its new path.
    pub(super) parts: BTreeMap<PartPath, Vec<u8>>,
    /// The content-type override of each duplicated part that needs one.
    pub(super) content_types: BTreeMap<PartPath, Cow<'static, str>>,
    /// The duplicated chart parts, whose series still name the source sheet.
    pub(super) charts: Vec<PartPath>,
    /// The relationship type that reached each duplicated part, as the last segment of the type.
    pub(super) kinds: BTreeMap<PartPath, Box<str>>,
}

/// What a copy duplicates from: its source sheet's parts and the copy's model.
pub(super) struct CopySource<'a> {
    pub(super) origin: &'a PartPath,
    pub(super) copy: &'a PartPath,
    pub(super) copy_id: SheetId,
    /// For each source table part, the source table and the copy's table that duplicates it.
    pub(super) tables: &'a BTreeMap<PartPath, (&'a Table, &'a Table)>,
    pub(super) content_types: &'a ContentTypes,
    pub(super) chart_parts: &'a [PartPath],
}

/// Duplicates every part the source worksheet reaches and writes their relationship parts, the
/// worksheet's included, pointing at the duplicates. A table part is patched to the copy's table,
/// a VML drawing gets shape IDs of its own block (`next_vml_block`, advanced), and every other
/// part keeps its bytes. External relationships, such as hyperlinks, are kept as they are.
///
/// # Errors
///
/// Returns [`XlsxWriteErrorCode::UnsupportedPreservation`] naming the part for a pivot table, form
/// control, threaded comment, slicer, timeline, query table, or ActiveX control.
pub(super) fn duplicate_sheet_parts(
    reader: &mut PackageReader<'_>,
    copy: &CopySource<'_>,
    next_vml_block: &mut u32,
    limits: WriteLimits,
) -> Result<SheetCopyParts, XlsxWriteError> {
    let mut output = SheetCopyParts::default();
    let mut mapped = BTreeMap::from([(copy.origin.clone(), copy.copy.clone())]);
    let mut kinds = BTreeMap::<PartPath, Box<str>>::new();
    let mut pending = vec![copy.origin.clone()];
    while let Some(original) = pending.pop() {
        let relationships = original
            .relationship_part()
            .map_err(|error| invalid_plan(&original, error))?;
        if !reader.contains(&relationships) {
            continue;
        }
        let bytes = reader.read_part(&relationships)?;
        let new_part = mapped[&original].clone();
        let rewritten = rewrite_relationships(&bytes, &relationships, limits, |kind, target| {
            let Ok(target) = PartPath::resolve_relationship(Some(&original), target) else {
                return Ok(None);
            };
            if !reader.contains(&target) {
                return Ok(None);
            }
            let local_kind = kind.rsplit('/').next().unwrap_or(kind);
            if UNDUPLICATABLE.contains(&local_kind) {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                        .with_detail(format!("{DETAIL_UNDUPLICATABLE}: {}", target.as_str()))
                        .at_source(target.source_id()),
                );
            }
            let duplicate = match mapped.get(&target) {
                Some(duplicate) => duplicate.clone(),
                None => {
                    let duplicate = duplicate_name(&target, copy)?;
                    mapped.insert(target.clone(), duplicate.clone());
                    kinds.insert(target.clone(), local_kind.into());
                    pending.push(target.clone());
                    duplicate
                }
            };
            Ok(Some(relative_target(&new_part, &duplicate)))
        })?;
        let new_relationships = new_part
            .relationship_part()
            .map_err(|error| invalid_plan(&new_part, error))?;
        output.parts.insert(new_relationships, rewritten);
    }
    for (original, duplicate) in &mapped {
        if original == copy.origin {
            continue;
        }
        let bytes = reader.read_part(original)?;
        let kind = kinds.get(original).map_or("", AsRef::as_ref);
        let bytes = match kind {
            "table" => {
                let (source_table, table) = copy.tables.get(original).ok_or_else(|| {
                    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                        .with_detail(DETAIL_MISSING_TABLE)
                        .at_source(original.source_id())
                })?;
                // A range change of the copied table can take columns out of it.
                let removed_columns = source_table
                    .columns()
                    .iter()
                    .map(crate::TableColumn::id)
                    .filter(|id| !table.columns().iter().any(|column| column.id() == *id))
                    .collect::<BTreeSet<_>>();
                let bytes = if removed_columns.is_empty() {
                    bytes
                } else {
                    remove_table_columns(&bytes, original, &removed_columns, limits)?
                };
                patch_table_xml(&bytes, duplicate, table, limits)?
            }
            "vmlDrawing" => {
                let block = *next_vml_block;
                *next_vml_block += 1;
                renumber_vml(&bytes, block)
            }
            _ => bytes,
        };
        if copy.chart_parts.contains(original) {
            output.charts.push(duplicate.clone());
        }
        output.kinds.insert(duplicate.clone(), kind.into());
        if copy.content_types.overrides(original)
            && let Some(content_type) = copy.content_types.content_type(original)
        {
            let content_type: String = content_type.to_owned();
            output
                .content_types
                .insert(duplicate.clone(), Cow::Owned(content_type));
        }
        output.parts.insert(duplicate.clone(), bytes);
    }
    Ok(output)
}

/// Names the duplicate of `part`: a copied table part by its table's new ID, every other part
/// in its folder with the copy's sheet ID before its file name, keeping its extension.
fn duplicate_name(part: &PartPath, copy: &CopySource<'_>) -> Result<PartPath, XlsxWriteError> {
    let (folder, file) = part
        .as_str()
        .rsplit_once('/')
        .unwrap_or(("", part.as_str()));
    let file = match copy.tables.get(part) {
        Some((_, table)) => format!("cellrune-table-{}.xml", table.id().get()),
        None => format!("cellrune-{}-{file}", copy.copy_id.get()),
    };
    let name = if folder.is_empty() {
        file
    } else {
        format!("{folder}/{file}")
    };
    PartPath::from_archive_name(name.as_bytes()).map_err(|error| invalid_plan(part, error))
}

/// Returns `to` relative to the folder of `from`, as a relationship target.
fn relative_target(from: &PartPath, to: &PartPath) -> String {
    let from = from.as_str().split('/').collect::<Vec<_>>();
    let to = to.as_str().split('/').collect::<Vec<_>>();
    let folder = &from[..from.len() - 1];
    let shared = folder
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut target = "../".repeat(folder.len() - shared);
    target.push_str(&to[shared..].join("/"));
    target
}

/// Gives a duplicated VML drawing the shape ID block `block`: its `o:idmap` data and each
/// `_x0000_s` shape number, which Excel keeps unique across a workbook's drawings.
fn renumber_vml(bytes: &[u8], block: u32) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let mut output = String::with_capacity(text.len());
    let mut rest = text.as_ref();
    while let Some(index) = rest.find("_x0000_s") {
        let (before, after) = rest.split_at(index + "_x0000_s".len());
        output.push_str(before);
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        match after[..digits].parse::<u32>() {
            Ok(number) => output.push_str(&(block * 1024 + number % 1024).to_string()),
            Err(_) => output.push_str(&after[..digits]),
        }
        rest = &after[digits..];
    }
    output.push_str(rest);
    let mut renumbered = String::with_capacity(output.len());
    let mut rest = output.as_str();
    while let Some(index) = rest.find("idmap") {
        let (before, after) = rest.split_at(index);
        renumbered.push_str(before);
        let tag_end = after.find('>').unwrap_or(after.len());
        let tag = &after[..tag_end];
        match tag.find("data=\"") {
            Some(data) => {
                let value_start = data + "data=\"".len();
                let value_end = tag[value_start..]
                    .find('"')
                    .map_or(tag.len(), |end| value_start + end);
                renumbered.push_str(&tag[..value_start]);
                renumbered.push_str(&block.to_string());
                renumbered.push_str(&tag[value_end..]);
            }
            None => renumbered.push_str(tag),
        }
        rest = &after[tag_end..];
    }
    renumbered.push_str(rest);
    renumbered.into_bytes()
}

/// Returns the highest `o:idmap` block the VML drawings use.
pub(super) fn highest_vml_block(drawings: &[Vec<u8>]) -> u32 {
    drawings
        .iter()
        .flat_map(|bytes| {
            let text = String::from_utf8_lossy(bytes).into_owned();
            text.match_indices("idmap")
                .filter_map(|(index, _)| {
                    let tag = &text[index
                        ..text[index..]
                            .find('>')
                            .map_or(text.len(), |end| index + end)];
                    let data = tag.find("data=\"")? + "data=\"".len();
                    let end = tag[data..].find('"')?;
                    Some(
                        tag[data..data + end]
                            .split(',')
                            .filter_map(|block| block.trim().parse::<u32>().ok())
                            .max()
                            .unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>()
        })
        .max()
        .unwrap_or(0)
}

/// Rewrites each internal relationship's `Target` with `retarget`, given its type and target;
/// `None` keeps the target, and external relationships are kept as they are.
fn rewrite_relationships(
    bytes: &[u8],
    source: &PartPath,
    limits: WriteLimits,
    mut retarget: impl FnMut(&str, &str) -> Result<Option<String>, XlsxWriteError>,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut xml = namespace_reader(bytes, limits.max_xml_depth());
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        let event = match event {
            Event::Empty(element) if element.local_name().as_ref() == "Relationship" => {
                Event::Empty(retargeted(&element, source, &mut retarget)?)
            }
            Event::Start(element) if element.local_name().as_ref() == "Relationship" => {
                Event::Start(retargeted(&element, source, &mut retarget)?)
            }
            Event::Eof => break,
            other => other.into_owned(),
        };
        writer
            .write_event(event)
            .map_err(|error| invalid_xml(source, error))?;
        buffer.clear();
    }
    Ok(writer.into_inner().into_inner())
}

fn retargeted(
    element: &BytesStart<'_>,
    source: &PartPath,
    retarget: &mut impl FnMut(&str, &str) -> Result<Option<String>, XlsxWriteError>,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let attribute = |name: &str| {
        element
            .try_get_attribute(name)
            .map_err(|error| invalid_xml(source, error))
            .map(|value| value.map(|value| value.value.into_owned()))
    };
    let external = attribute("TargetMode")?.is_some_and(|mode| mode == "External");
    let (Some(kind), Some(target)) = (attribute("Type")?, attribute("Target")?) else {
        return Ok(element.to_owned());
    };
    let replacement = if external {
        None
    } else {
        retarget(&kind, &target)?
    };
    let Some(replacement) = replacement else {
        return Ok(element.to_owned());
    };
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == "Target" {
            patched.push_attribute(("Target", replacement.as_str()));
        } else {
            patched.push_attribute(attribute);
        }
    }
    Ok(patched.into_owned())
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
    use super::{highest_vml_block, relative_target, renumber_vml};
    use crate::xlsx::package::PartPath;

    fn part(name: &str) -> PartPath {
        PartPath::from_archive_name(name.as_bytes()).expect("part")
    }

    #[test]
    fn relationship_targets_are_relative_to_the_source_folder() {
        assert_eq!(
            relative_target(
                &part("xl/worksheets/cellrune-sheet-4.xml"),
                &part("xl/drawings/cellrune-4-drawing1.xml")
            ),
            "../drawings/cellrune-4-drawing1.xml"
        );
        assert_eq!(
            relative_target(
                &part("xl/drawings/cellrune-4-drawing1.xml"),
                &part("xl/drawings/cellrune-4-vmlDrawing1.vml")
            ),
            "cellrune-4-vmlDrawing1.vml"
        );
    }

    #[test]
    fn a_duplicated_vml_drawing_takes_its_own_shape_block() {
        let vml = br#"<xml><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shape id="_x0000_s1025" o:spid="_x0000_s1026"/></xml>"#;
        assert_eq!(highest_vml_block(&[vml.to_vec()]), 1);
        assert_eq!(
            String::from_utf8(renumber_vml(vml, 3)).expect("UTF-8"),
            r#"<xml><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="3"/></o:shapelayout><v:shape id="_x0000_s3073" o:spid="_x0000_s3074"/></xml>"#
        );
    }
}
