use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::NsReader;

use super::table_patch::{
    is_spreadsheet_element, patch_attributes, range_text, replace_changed_attribute,
};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::SheetAutoFilter;
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{namespace_reader, read_event};

const DETAIL_AUTO_FILTER_STRUCTURE: &str =
    "source worksheet auto-filter XML does not match the sheet auto-filter model";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

/// Worksheet elements that follow `<autoFilter>` in CT_Worksheet order, before one of which a new
/// filter is inserted.
const AFTER_AUTO_FILTER: &[&str] = &[
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

/// What happens to one worksheet element as the filter is rewritten.
enum FilterAction {
    Keep(BytesStart<'static>),
    Drop,
}

/// Rewrites the worksheet-level `<autoFilter>` from `original` to `draft`: a changed filter gets
/// its new reference, and each `<filterColumn>` keeps its settings under the `colId` of the same
/// worksheet column, or leaves with its column; a removed filter leaves with its content, and a
/// new one is written where CT_Worksheet places it. Every other worksheet element passes through.
pub(crate) fn rewrite_sheet_auto_filter(
    source_bytes: &[u8],
    source: &PartPath,
    original: Option<&SheetAutoFilter>,
    draft: Option<&SheetAutoFilter>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    enforce_bytes(source_bytes.len(), limits, source)?;
    let mut xml = namespace_reader(source_bytes, limits.max_xml_depth());
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut root_name = String::new();
    let mut saw_filter = false;
    let mut inserted = original.is_some() || draft.is_none();
    // The depth of an element being dropped with its content.
    let mut skip_depth = None::<u64>;
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
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if depth == 1 {
                    root_name = element.name().as_ref().to_owned();
                }
                if !inserted
                    && depth == 2
                    && AFTER_AUTO_FILTER.contains(&element.local_name().as_ref())
                {
                    write_new_filter(&mut writer, &root_name, draft, source)?;
                    inserted = true;
                }
                match filter_action(
                    &xml,
                    element,
                    depth,
                    original,
                    draft,
                    &mut saw_filter,
                    source,
                )? {
                    FilterAction::Keep(element) => {
                        write_event(&mut writer, Event::Start(element), source)?;
                    }
                    FilterAction::Drop => skip_depth = Some(depth),
                }
            }
            Event::Empty(element) => {
                let element_depth = depth.saturating_add(1);
                enforce_depth(element_depth, limits, source)?;
                if !inserted
                    && element_depth == 2
                    && AFTER_AUTO_FILTER.contains(&element.local_name().as_ref())
                {
                    write_new_filter(&mut writer, &root_name, draft, source)?;
                    inserted = true;
                }
                if let FilterAction::Keep(element) = filter_action(
                    &xml,
                    element,
                    element_depth,
                    original,
                    draft,
                    &mut saw_filter,
                    source,
                )? {
                    write_event(&mut writer, Event::Empty(element), source)?;
                }
            }
            Event::End(element) => {
                if !inserted && depth == 1 {
                    write_new_filter(&mut writer, &root_name, draft, source)?;
                    inserted = true;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if original.is_some() != saw_filter || !inserted {
        return Err(invalid_structure(source));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

/// Decides what happens to the worksheet-level `<autoFilter>` and its `<filterColumn>` children;
/// every other element is kept as it is.
fn filter_action(
    xml: &NsReader<&[u8]>,
    element: BytesStart<'_>,
    depth: u64,
    original: Option<&SheetAutoFilter>,
    draft: Option<&SheetAutoFilter>,
    saw_filter: &mut bool,
    source: &PartPath,
) -> Result<FilterAction, XlsxWriteError> {
    let local = element.local_name();
    if depth == 2 && local.as_ref() == "autoFilter" && is_spreadsheet_element(xml, element.name()) {
        if std::mem::replace(saw_filter, true) {
            return Err(invalid_structure(source));
        }
        return Ok(match draft {
            None => FilterAction::Drop,
            Some(filter) => FilterAction::Keep(patch_attributes(
                &element,
                source,
                &[("ref", Some(range_text(filter.range())))],
            )?),
        });
    }
    if depth == 3
        && *saw_filter
        && local.as_ref() == "filterColumn"
        && is_spreadsheet_element(xml, element.name())
    {
        let (Some(original), Some(draft)) = (original, draft) else {
            return Err(invalid_structure(source));
        };
        let column_id = element
            .try_get_attribute("colId")
            .map_err(|error| invalid_xml(source, error))?
            .and_then(|attribute| attribute.value.parse::<u32>().ok())
            .ok_or_else(|| invalid_structure(source))?;
        let column = original.range().start().column().get() + column_id;
        let range = draft.range();
        if column < range.start().column().get() || column > range.end().column().get() {
            return Ok(FilterAction::Drop);
        }
        return Ok(FilterAction::Keep(replace_changed_attribute(
            element,
            source,
            "colId",
            (column - range.start().column().get()).to_string(),
        )?));
    }
    Ok(FilterAction::Keep(element.into_owned()))
}

/// Writes a new `<autoFilter>` named like the worksheet root's children.
fn write_new_filter(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    root_name: &str,
    filter: Option<&SheetAutoFilter>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let Some(filter) = filter else {
        return Ok(());
    };
    let prefix = root_name
        .rfind(':')
        .map_or("", |index| &root_name[..=index]);
    let name = format!("{prefix}autoFilter");
    let mut element = BytesStart::new(name.as_str());
    let range = range_text(filter.range());
    element.push_attribute(("ref", range.as_str()));
    write_event(writer, Event::Empty(element), source)
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

fn invalid_structure(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(DETAIL_AUTO_FILTER_STRUCTURE)
        .at_source(source.source_id())
}

fn invalid_xml(
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
