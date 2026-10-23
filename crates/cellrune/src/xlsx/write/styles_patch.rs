use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Write as _};

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::name::QName;
use quick_xml::reader::NsReader;

use super::canonical::StyleRegistry;
use super::format_xml;
use super::serialization::escape_attribute;
use super::style_records::SourceRecords;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{namespace_reader, read_event};
use crate::{
    Alignment, Border, CalculationCellId, CellFormat, DocumentPresentation, Fill, FillPattern,
    Font, NumberFormat, PatternFill,
};

const DETAIL_MISSING_CELL_XFS: &str = "styles part does not contain cellXfs";
const DETAIL_BASE_STYLE: &str = "cell references an unavailable base style";
const DETAIL_CUSTOM_CONFLICT: &str =
    "one custom number-format ID is associated with multiple format codes";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

/// Stylesheet children in schema order, up to `cellXfs`, which every styles part holds.
const SECTION_ORDER: [&str; 6] = [
    "numFmts",
    "fonts",
    "fills",
    "borders",
    "cellStyleXfs",
    "cellXfs",
];

pub(crate) struct StyleRequest {
    pub(crate) base_index: usize,
    pub(crate) format: NumberFormat,
    /// The font, fill, border, and alignment to display, or `None` to keep the base style's.
    pub(crate) presentation: Option<CellFormat>,
}

pub(crate) struct DocumentStylePlan {
    pub(crate) bytes: Vec<u8>,
    pub(crate) indexes: BTreeMap<CalculationCellId, usize>,
}

#[derive(Clone)]
struct XfTemplate {
    events: Vec<Event<'static>>,
}

/// The records of one `cellXfs` entry a request replaces; `None` keeps the base style's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct XfReplacement {
    font: Option<usize>,
    fill: Option<usize>,
    border: Option<usize>,
    alignment: Option<Alignment>,
}

impl XfReplacement {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Default)]
struct StyleAnalysis {
    custom_formats: BTreeMap<u32, String>,
    xfs: Vec<XfTemplate>,
    /// Record counts of the sections the styles part holds, by section name.
    sections: BTreeMap<&'static str, usize>,
    /// The source font, fill, border, and xf records an edit can reuse.
    records: SourceRecords,
}

impl StyleAnalysis {
    fn existing(&self, section: &str) -> usize {
        self.sections.get(section).copied().unwrap_or(0)
    }

    /// Keeps the `index`th record of a font, fill, border, or `cellXfs` section.
    fn record(
        &mut self,
        section: &'static str,
        index: usize,
        events: Vec<Event<'static>>,
        source: &PartPath,
    ) -> Result<(), XlsxWriteError> {
        if section != "cellXfs" {
            return self.records.insert(section, index, &events, source);
        }
        let is_xf = matches!(
            events.first(),
            Some(Event::Start(element) | Event::Empty(element))
                if element.local_name().as_ref() == "xf"
        );
        if is_xf {
            // Style indexes count `xf` entries only.
            self.records
                .insert(section, self.xfs.len(), &events, source)?;
            self.xfs.push(XfTemplate { events });
        }
        Ok(())
    }
}

/// Font, fill, and border records appended to the styles part.
#[derive(Default)]
struct RecordAdditions {
    fonts: Vec<Font>,
    fills: Vec<PatternFill>,
    borders: Vec<Border>,
}

impl RecordAdditions {
    /// Returns the index of a source font equal to `font`, else of an added one.
    fn font(
        &mut self,
        font: &Font,
        analysis: &StyleAnalysis,
        source: &PartPath,
    ) -> Result<usize, XlsxWriteError> {
        if let Some(index) = analysis
            .records
            .font(&format_xml::font_xml(font, "")?, source)?
        {
            return Ok(index);
        }
        Ok(analysis.existing("fonts") + position_or_push(&mut self.fonts, font))
    }

    /// Returns the index of a source fill equal to `fill`, else of an added one.
    fn fill(
        &mut self,
        fill: &PatternFill,
        analysis: &StyleAnalysis,
        source: &PartPath,
    ) -> Result<usize, XlsxWriteError> {
        if let Some(index) = analysis
            .records
            .fill(&format_xml::fill_xml(fill, ""), source)?
        {
            return Ok(index);
        }
        let existing = analysis.existing("fills");
        // Excel reads fill records 0 and 1 as `none` and `gray125` whatever they hold, so a source
        // with fewer than two fills first receives the missing reserved records.
        if self.fills.is_empty() {
            let reserved = [
                PatternFill::default(),
                PatternFill::new(FillPattern::Gray125),
            ];
            self.fills
                .extend(reserved.into_iter().skip(existing.min(reserved.len())));
        }
        Ok(existing + position_or_push(&mut self.fills, fill))
    }

    /// Returns the index of a source border equal to `border`, else of an added one.
    fn border(
        &mut self,
        border: &Border,
        analysis: &StyleAnalysis,
        source: &PartPath,
    ) -> Result<usize, XlsxWriteError> {
        if let Some(index) = analysis
            .records
            .border(&format_xml::border_xml(border, ""), source)?
        {
            return Ok(index);
        }
        Ok(analysis.existing("borders") + position_or_push(&mut self.borders, border))
    }

    fn section_xml(&self, section: &str, prefix: &str) -> Result<Vec<String>, XlsxWriteError> {
        match section {
            "fonts" => self
                .fonts
                .iter()
                .map(|font| format_xml::font_xml(font, prefix))
                .collect(),
            "fills" => Ok(self
                .fills
                .iter()
                .map(|fill| format_xml::fill_xml(fill, prefix))
                .collect()),
            "borders" => Ok(self
                .borders
                .iter()
                .map(|border| format_xml::border_xml(border, prefix))
                .collect()),
            _ => Ok(Vec::new()),
        }
    }

    fn count(&self, section: &str) -> usize {
        match section {
            "fonts" => self.fonts.len(),
            "fills" => self.fills.len(),
            "borders" => self.borders.len(),
            _ => 0,
        }
    }
}

fn position_or_push<T: Clone + PartialEq>(values: &mut Vec<T>, value: &T) -> usize {
    if let Some(index) = values.iter().position(|candidate| candidate == value) {
        return index;
    }
    values.push(value.clone());
    values.len() - 1
}

pub(crate) fn plan_document_styles(
    bytes: Option<&[u8]>,
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    base_formats: &DocumentPresentation,
    limits: WriteLimits,
) -> Result<Option<DocumentStylePlan>, XlsxWriteError> {
    if requests.is_empty() {
        return Ok(None);
    }
    match bytes {
        Some(bytes) => {
            patch_existing_styles(bytes, source, requests, base_formats, limits).map(Some)
        }
        None => generate_styles(source, requests, limits).map(Some),
    }
}

fn generate_styles(
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    limits: WriteLimits,
) -> Result<DocumentStylePlan, XlsxWriteError> {
    if requests.values().any(|request| request.base_index != 0) {
        return Err(invalid_generated(source, DETAIL_BASE_STYLE));
    }
    let registry = StyleRegistry::for_formats(
        requests
            .values()
            .map(|request| (&request.format, request.presentation.as_ref())),
    )?;
    let indexes = requests
        .iter()
        .map(|(id, request)| {
            (
                *id,
                registry.index(&request.format, request.presentation.as_ref()),
            )
        })
        .collect();
    let bytes = registry.to_xml()?.into_bytes();
    enforce_bytes(bytes.len(), limits, source)?;
    Ok(DocumentStylePlan { bytes, indexes })
}

fn patch_existing_styles(
    bytes: &[u8],
    source: &PartPath,
    requests: &BTreeMap<CalculationCellId, StyleRequest>,
    base_formats: &DocumentPresentation,
    limits: WriteLimits,
) -> Result<DocumentStylePlan, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let analysis = analyze_styles(bytes, source, limits)?;
    let mut records = RecordAdditions::default();
    let mut additions = Vec::<XfTemplate>::new();
    let mut indexes = BTreeMap::new();
    let mut requested_custom = BTreeMap::<u32, String>::new();
    let mut added_indexes = HashMap::<(usize, u32, XfReplacement), usize>::new();
    for (id, request) in requests {
        let base = analysis
            .xfs
            .get(request.base_index)
            .ok_or_else(|| invalid_generated(source, DETAIL_BASE_STYLE))?;
        if request.format.id() >= 164 {
            let code = request
                .format
                .code()
                .ok_or_else(|| invalid_generated(source, DETAIL_CUSTOM_CONFLICT))?;
            if analysis
                .custom_formats
                .get(&request.format.id())
                .is_some_and(|existing| existing != code)
                || requested_custom
                    .get(&request.format.id())
                    .is_some_and(|existing| existing != code)
            {
                return Err(invalid_generated(source, DETAIL_CUSTOM_CONFLICT));
            }
            if !analysis.custom_formats.contains_key(&request.format.id()) {
                requested_custom.insert(request.format.id(), code.to_owned());
            }
        }
        // Only the components that differ from the base style get new records, so a record's
        // attributes the model does not carry stay on every component the edit left alone.
        let replacement = match &request.presentation {
            Some(format) => {
                let base_format = base_formats.style_format(request.base_index);
                XfReplacement {
                    font: (format.font() != base_format.font())
                        .then(|| records.font(format.font(), &analysis, source))
                        .transpose()?,
                    fill: match format.fill() {
                        Fill::Pattern(fill) if format.fill() != base_format.fill() => {
                            Some(records.fill(&fill, &analysis, source)?)
                        }
                        _ => None,
                    },
                    border: (format.border() != base_format.border())
                        .then(|| records.border(&format.border(), &analysis, source))
                        .transpose()?,
                    alignment: (format.alignment() != base_format.alignment())
                        .then_some(format.alignment()),
                }
            }
            None => XfReplacement::default(),
        };
        if replacement.is_empty() && template_number_format(base, source)? == request.format.id() {
            indexes.insert(*id, request.base_index);
            continue;
        }
        let key = (request.base_index, request.format.id(), replacement);
        let style_index = if let Some(index) = added_indexes.get(&key) {
            *index
        } else {
            let template = patch_template(base, request.format.id(), &replacement, source)?;
            // An earlier session may already have added this exact xf.
            let index = match analysis.records.xf(&template.events, source)? {
                Some(index) => index,
                None => {
                    additions.push(template);
                    analysis.xfs.len().saturating_add(additions.len() - 1)
                }
            };
            added_indexes.insert(key, index);
            index
        };
        indexes.insert(*id, style_index);
    }
    let output = rewrite_styles(
        bytes,
        source,
        &analysis,
        &StyleAdditions {
            custom_formats: &requested_custom,
            records: &records,
            xfs: &additions,
        },
        limits,
    )?;
    Ok(DocumentStylePlan {
        bytes: output,
        indexes,
    })
}

fn analyze_styles(
    bytes: &[u8],
    source: &PartPath,
    limits: WriteLimits,
) -> Result<StyleAnalysis, XlsxWriteError> {
    let mut xml = configured_reader(bytes, limits);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut section = None::<&'static str>;
    let mut analysis = StyleAnalysis::default();
    // The depth, section, and index of the record being captured, with its events.
    let mut capture = None::<(u64, &'static str, usize, Vec<Event<'static>>)>;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                let element_depth = depth.saturating_add(1);
                enforce_depth(element_depth, limits, source)?;
                if let Some((_, _, _, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                } else if element_depth == 2 {
                    section = SECTION_ORDER
                        .iter()
                        .find(|name| element.local_name().as_ref() == **name)
                        .copied();
                    if let Some(name) = section {
                        analysis.sections.insert(name, 0);
                    }
                    if empty {
                        section = None;
                    }
                } else if element_depth == 3
                    && let Some(name) = section
                {
                    let count = analysis.sections.entry(name).or_default();
                    let index = *count;
                    *count += 1;
                    if name == "numFmts" && element.local_name().as_ref() == "numFmt" {
                        insert_custom_format(element, &mut analysis.custom_formats, source)?;
                    } else if name != "numFmts" && name != "cellStyleXfs" {
                        let events = vec![event.clone().into_owned()];
                        if empty {
                            analysis.record(name, index, events, source)?;
                        } else {
                            capture = Some((element_depth, name, index, events));
                        }
                    }
                }
                if !empty {
                    depth = element_depth;
                }
            }
            Event::End(_) => {
                if let Some((capture_depth, _, _, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                    if depth == *capture_depth {
                        let (_, name, index, events) = capture
                            .take()
                            .ok_or_else(|| invalid_generated(source, DETAIL_MISSING_CELL_XFS))?;
                        analysis.record(name, index, events, source)?;
                    }
                } else if depth == 2 {
                    section = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {
                if let Some((_, _, _, events)) = &mut capture {
                    events.push(event.clone().into_owned());
                }
            }
        }
        buffer.clear();
    }
    if analysis.xfs.is_empty() || !analysis.sections.contains_key("cellXfs") {
        return Err(invalid_generated(source, DETAIL_MISSING_CELL_XFS));
    }
    Ok(analysis)
}

struct StyleAdditions<'a> {
    custom_formats: &'a BTreeMap<u32, String>,
    records: &'a RecordAdditions,
    xfs: &'a [XfTemplate],
}

impl StyleAdditions<'_> {
    fn count(&self, section: &str) -> usize {
        match section {
            "numFmts" => self.custom_formats.len(),
            "cellXfs" => self.xfs.len(),
            other => self.records.count(other),
        }
    }
}

fn rewrite_styles(
    bytes: &[u8],
    source: &PartPath,
    analysis: &StyleAnalysis,
    additions: &StyleAdditions<'_>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut xml = configured_reader(bytes, limits);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut open_section = None::<(&'static str, String)>;
    let mut root_prefix = String::new();
    // Sections absent from the source that additions need are inserted before the first later
    // stylesheet child, in schema order.
    let mut missing = SECTION_ORDER
        .iter()
        .copied()
        .filter(|name| !analysis.sections.contains_key(name) && additions.count(name) > 0)
        .collect::<Vec<_>>();
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 1 {
                    root_prefix = qualified_prefix(element.name().as_ref());
                }
                if depth == 2 {
                    insert_missing_sections(
                        &mut writer,
                        &mut missing,
                        element.local_name().as_ref(),
                        &root_prefix,
                        additions,
                        source,
                    )?;
                }
                match section_of(depth, &element) {
                    Some(name) => {
                        let total = analysis.sections[name] + additions.count(name);
                        let patched = patch_count(&element, total, source)?;
                        open_section = Some((name, qualified_prefix(element.name().as_ref())));
                        write_event(&mut writer, Event::Start(patched), source)?;
                    }
                    None => {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                    }
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if depth + 1 == 2 {
                    insert_missing_sections(
                        &mut writer,
                        &mut missing,
                        element.local_name().as_ref(),
                        &root_prefix,
                        additions,
                        source,
                    )?;
                }
                match section_of(depth + 1, &element) {
                    Some(name) if additions.count(name) > 0 => {
                        let total = analysis.sections[name] + additions.count(name);
                        let patched = patch_count(&element, total, source)?;
                        let prefix = qualified_prefix(element.name().as_ref());
                        write_event(&mut writer, Event::Start(patched), source)?;
                        write_section_additions(&mut writer, name, &prefix, additions, source)?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(element.name().as_ref().to_owned())),
                            source,
                        )?;
                    }
                    _ => write_event(&mut writer, Event::Empty(element.into_owned()), source)?,
                }
            }
            Event::End(element) => {
                if depth == 2
                    && let Some((name, prefix)) = open_section.take()
                {
                    write_section_additions(&mut writer, name, &prefix, additions, source)?;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if !missing.is_empty() {
        return Err(invalid_generated(source, DETAIL_MISSING_CELL_XFS));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

/// Returns the stylesheet section a depth-2 element is.
fn section_of(depth: u64, element: &BytesStart<'_>) -> Option<&'static str> {
    if depth != 2 {
        return None;
    }
    SECTION_ORDER
        .iter()
        .find(|name| element.local_name().as_ref() == **name)
        .copied()
}

fn insert_missing_sections(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    missing: &mut Vec<&'static str>,
    next_local_name: &str,
    prefix: &str,
    additions: &StyleAdditions<'_>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let next_position = SECTION_ORDER
        .iter()
        .position(|name| *name == next_local_name)
        .unwrap_or(SECTION_ORDER.len());
    while let Some(name) = missing.first().copied() {
        let position = SECTION_ORDER
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0);
        if position >= next_position {
            break;
        }
        missing.remove(0);
        let qualified = format!("{prefix}{name}");
        let mut start = BytesStart::new(qualified.as_str());
        let count = additions.count(name).to_string();
        start.push_attribute(("count", count.as_str()));
        write_event(writer, Event::Start(start), source)?;
        write_section_additions(writer, name, prefix, additions, source)?;
        write_event(
            writer,
            Event::End(BytesEnd::new(qualified.as_str())),
            source,
        )?;
    }
    Ok(())
}

fn write_section_additions(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    section: &str,
    prefix: &str,
    additions: &StyleAdditions<'_>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    match section {
        "numFmts" => write_custom_formats(writer, additions.custom_formats, prefix, source),
        "cellXfs" => write_templates(writer, additions.xfs, source),
        other => {
            for record in additions.records.section_xml(other, prefix)? {
                writer
                    .get_mut()
                    .write_all(record.as_bytes())
                    .map_err(|error| invalid_xml(source, error))?;
            }
            Ok(())
        }
    }
}

fn patch_template(
    template: &XfTemplate,
    number_format_id: u32,
    replacement: &XfReplacement,
    source: &PartPath,
) -> Result<XfTemplate, XlsxWriteError> {
    let (first, children) = template
        .events
        .split_first()
        .ok_or_else(|| invalid_generated(source, DETAIL_BASE_STYLE))?;
    let (element, empty) = match first {
        Event::Start(element) => (element, false),
        Event::Empty(element) => (element, true),
        _ => return Err(invalid_generated(source, DETAIL_BASE_STYLE)),
    };
    let patched = patch_xf(element, number_format_id, replacement, source)?;
    let Some(alignment) = replacement.alignment else {
        let first = if empty {
            Event::Empty(patched)
        } else {
            Event::Start(patched)
        };
        let mut events = vec![first];
        events.extend(children.iter().cloned());
        return Ok(XfTemplate { events });
    };
    let name = element.name().as_ref().to_owned();
    let prefix = qualified_prefix(&name);
    let mut events = vec![Event::Start(patched)];
    if let Some(xml) = format_xml::alignment_xml(&alignment, &prefix) {
        events.push(single_element_event(&xml, source)?);
    }
    // The base alignment is replaced; every other child keeps its source events.
    let body = if empty {
        &[][..]
    } else {
        &children[..children.len().saturating_sub(1)]
    };
    let mut skip_depth = None::<usize>;
    let mut nested = 0_usize;
    for event in body {
        match event {
            Event::Start(child) => {
                nested += 1;
                if skip_depth.is_none() && nested == 1 && child.local_name().as_ref() == "alignment"
                {
                    skip_depth = Some(nested);
                }
                if skip_depth.is_none() {
                    events.push(event.clone());
                }
            }
            Event::End(_) => {
                if skip_depth.is_none() {
                    events.push(event.clone());
                }
                if skip_depth == Some(nested) {
                    skip_depth = None;
                }
                nested = nested.saturating_sub(1);
            }
            Event::Empty(child) if nested == 0 && child.local_name().as_ref() == "alignment" => {}
            other => {
                if skip_depth.is_none() {
                    events.push(other.clone());
                }
            }
        }
    }
    events.push(Event::End(BytesEnd::new(name)));
    Ok(XfTemplate { events })
}

/// Parses one generated self-closing element into an owned event.
fn single_element_event(xml: &str, source: &PartPath) -> Result<Event<'static>, XlsxWriteError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    match reader
        .read_event()
        .map_err(|error| invalid_xml(source, error))?
    {
        Event::Empty(element) => Ok(Event::Empty(element.into_owned())),
        _ => Err(invalid_generated(source, DETAIL_BASE_STYLE)),
    }
}

fn patch_xf(
    element: &BytesStart<'_>,
    number_format_id: u32,
    replacement: &XfReplacement,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut replaced = vec!["numFmtId", "applyNumberFormat"];
    let mut added = vec![
        ("numFmtId", number_format_id.to_string()),
        ("applyNumberFormat", "1".to_owned()),
    ];
    for (record, id_name, apply_name) in [
        (replacement.font, "fontId", "applyFont"),
        (replacement.fill, "fillId", "applyFill"),
        (replacement.border, "borderId", "applyBorder"),
    ] {
        if let Some(index) = record {
            replaced.extend([id_name, apply_name]);
            added.extend([(id_name, index.to_string()), (apply_name, "1".to_owned())]);
        }
    }
    if replacement.alignment.is_some() {
        replaced.push("applyAlignment");
        added.push(("applyAlignment", "1".to_owned()));
    }
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let key = attribute.key.as_ref();
        if !replaced.contains(&key) {
            patched.push_attribute(attribute);
        }
    }
    for (name, value) in &added {
        patched.push_attribute((*name, value.as_str()));
    }
    Ok(patched.into_owned())
}

fn template_number_format(template: &XfTemplate, source: &PartPath) -> Result<u32, XlsxWriteError> {
    let element = match template.events.first() {
        Some(Event::Start(element) | Event::Empty(element)) => element,
        _ => return Err(invalid_generated(source, DETAIL_BASE_STYLE)),
    };
    // `numFmtId` is optional on `xf`; an absent one selects General.
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == "numFmtId" {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| invalid_xml(source, error))?
                .parse::<u32>()
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Ok(0)
}

fn insert_custom_format(
    element: &BytesStart<'_>,
    formats: &mut BTreeMap<u32, String>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let id = required_u32_attribute(element, "numFmtId", source)?;
    let code = required_attribute(element, "formatCode", source)?;
    if formats.insert(id, code).is_some() {
        return Err(invalid_generated(source, DETAIL_CUSTOM_CONFLICT));
    }
    Ok(())
}

fn write_custom_formats(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    formats: &BTreeMap<u32, String>,
    prefix: &str,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = format!("{prefix}numFmt");
    for (id, code) in formats {
        let mut format = BytesStart::new(name.as_str());
        let id = id.to_string();
        format.push_attribute(("numFmtId", id.as_str()));
        let escaped_code = escape_attribute(code)?;
        format.push_attribute(Attribute {
            key: QName("formatCode"),
            value: Cow::Borrowed(&escaped_code),
        });
        write_event(writer, Event::Empty(format), source)?;
    }
    Ok(())
}

fn write_templates(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    templates: &[XfTemplate],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    for template in templates {
        for event in &template.events {
            write_event(writer, event.clone(), source)?;
        }
    }
    Ok(())
}

/// Returns the `prefix:` of a qualified element name, or `""` for an unprefixed one.
fn qualified_prefix(name: &str) -> String {
    name.rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default()
}

fn patch_count(
    element: &BytesStart<'_>,
    count: usize,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != "count" {
            patched.push_attribute(attribute);
        }
    }
    let count = count.to_string();
    patched.push_attribute(("count", count.as_str()));
    Ok(patched.into_owned())
}

fn required_u32_attribute(
    element: &BytesStart<'_>,
    name: &str,
    source: &PartPath,
) -> Result<u32, XlsxWriteError> {
    required_attribute(element, name, source)?
        .parse::<u32>()
        .map_err(|error| invalid_xml(source, error))
}

fn required_attribute(
    element: &BytesStart<'_>,
    name: &str,
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
    Err(invalid_generated(source, DETAIL_BASE_STYLE))
}

fn configured_reader(bytes: &[u8], limits: WriteLimits) -> NsReader<&[u8]> {
    let mut reader = namespace_reader(bytes, limits.max_xml_depth());
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(resource_error(
            source,
            DETAIL_XML_DEPTH,
            depth,
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

pub(super) fn invalid_xml(
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
