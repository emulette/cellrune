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

/// The worksheet-level `<autoFilter>` being rewritten.
struct FilterPatch<'a> {
    filter: &'a SheetAutoFilter,
    saw_filter: bool,
    open_depth: Option<u64>,
    next_column: usize,
}

impl FilterPatch<'_> {
    /// Rewrites the filter reference or the next `<filterColumn>` identifier and passes every
    /// other element through.
    fn patch(
        &mut self,
        xml: &NsReader<&[u8]>,
        element: BytesStart<'_>,
        depth: u64,
        has_children: bool,
        source: &PartPath,
    ) -> Result<BytesStart<'static>, XlsxWriteError> {
        let local = element.local_name();
        if depth == 2
            && local.as_ref() == "autoFilter"
            && is_spreadsheet_element(xml, element.name())
        {
            if std::mem::replace(&mut self.saw_filter, true) {
                return Err(invalid_structure(source));
            }
            if has_children {
                self.open_depth = Some(depth);
            }
            return patch_attributes(
                &element,
                source,
                &[("ref", Some(range_text(self.filter.range())))],
            );
        }
        if self.open_depth.is_some_and(|parent| depth == parent + 1)
            && local.as_ref() == "filterColumn"
            && is_spreadsheet_element(xml, element.name())
        {
            let column_id = self
                .filter
                .filter_column_ids()
                .get(self.next_column)
                .ok_or_else(|| invalid_structure(source))?;
            self.next_column += 1;
            return replace_changed_attribute(element, source, "colId", column_id.to_string());
        }
        Ok(element.into_owned())
    }
}

/// Rewrites the worksheet-level `<autoFilter>` reference and the `colId` of each of its
/// `<filterColumn>` children in declaration order. Filter criteria, sort state, and every other
/// worksheet element pass through unchanged.
pub(crate) fn patch_sheet_auto_filter(
    source_bytes: &[u8],
    source: &PartPath,
    filter: &SheetAutoFilter,
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
    let mut patch = FilterPatch {
        filter,
        saw_filter: false,
        open_depth: None,
        next_column: 0,
    };

    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                let element = patch.patch(&xml, element, depth, true, source)?;
                write_event(&mut writer, Event::Start(element), source)?;
            }
            Event::Empty(element) => {
                let element_depth = depth.saturating_add(1);
                enforce_depth(element_depth, limits, source)?;
                let element = patch.patch(&xml, element, element_depth, false, source)?;
                write_event(&mut writer, Event::Empty(element), source)?;
            }
            Event::End(element) => {
                if patch.open_depth == Some(depth) {
                    patch.open_depth = None;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if !patch.saw_filter || patch.next_column != filter.filter_column_ids().len() {
        return Err(invalid_structure(source));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
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
