//! Gives the rows and cells of a worksheet the writer rewrites an explicit `r`.
//!
//! `r` is optional on `<row>` and `<c>`: a row without it follows the last row named so far, and
//! a cell without it follows the previous cell of its row. The patchers locate rows and cells by
//! `r`, and inserting or removing a cell would move every later cell that has none, so a
//! worksheet is given explicit positions, the ones a read assigns, before it is patched.

use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{
    SPREADSHEETML_STRICT, SPREADSHEETML_TRANSITIONAL, namespace_reader, read_event,
};
use crate::{CellAddress, EXCEL_MAX_ROWS};

const DETAIL_IMPLIED_POSITION: &str = "worksheet row or cell without r follows the last position";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";

/// The positions a worksheet read assigns, following rows and cells without `r`.
#[derive(Debug, Default)]
struct ImpliedPositions {
    /// The last row a `<row>` or cell named.
    last_row: u32,
    /// The open row's number.
    row: u32,
    /// The column of the open row's previous cell.
    last_column: u32,
}

impl ImpliedPositions {
    fn begin_row(&mut self, explicit: Option<u32>) -> u32 {
        self.row = explicit.unwrap_or_else(|| self.last_row.saturating_add(1));
        self.last_row = self.last_row.max(self.row);
        self.last_column = 0;
        self.row
    }

    fn cell(&mut self, explicit: Option<CellAddress>) -> Option<CellAddress> {
        let address = match explicit {
            Some(address) => address,
            None => CellAddress::from_indices(self.row, self.last_column.saturating_add(1)).ok()?,
        };
        self.last_column = address.column().get();
        self.last_row = self.last_row.max(address.row().get());
        Some(address)
    }
}

/// Returns the worksheet with an explicit `r` on every row and cell of `<sheetData>` that has
/// none, or `None` when every one already has one.
pub(super) fn with_explicit_positions(
    bytes: &[u8],
    source: &PartPath,
    limits: WriteLimits,
) -> Result<Option<Vec<u8>>, XlsxWriteError> {
    let mut xml = namespace_reader(bytes, limits.max_xml_depth());
    let config = xml.config_mut();
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut sheet_data_depth = None::<u64>;
    let mut positions = ImpliedPositions::default();
    let mut changed = false;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let element_depth = depth.saturating_add(1);
                if element_depth > limits.max_xml_depth() {
                    return Err(
                        XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                            .with_detail(format!(
                                "{DETAIL_XML_DEPTH}: {element_depth} > {}",
                                limits.max_xml_depth()
                            ))
                            .at_source(source.source_id()),
                    );
                }
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                let local = element.local_name().as_ref().to_owned();
                let positioned = match sheet_data_depth {
                    None if spreadsheet && local == "sheetData" => {
                        if !empty {
                            sheet_data_depth = Some(element_depth);
                        }
                        None
                    }
                    Some(parent)
                        if element_depth == parent + 1 && spreadsheet && local == "row" =>
                    {
                        let text = reference(&element, source)?;
                        let row = positions
                            .begin_row(text.as_deref().and_then(|value| value.parse::<u32>().ok()));
                        if row > EXCEL_MAX_ROWS {
                            return Err(invalid_generated(source));
                        }
                        text.is_none()
                            .then(|| with_reference(&element, &row.to_string(), source))
                    }
                    Some(parent) if element_depth == parent + 2 && spreadsheet && local == "c" => {
                        let text = reference(&element, source)?;
                        let explicit = text
                            .as_deref()
                            .and_then(|value| CellAddress::from_a1(value).ok());
                        let address = positions
                            .cell(explicit)
                            .ok_or_else(|| invalid_generated(source))?;
                        text.is_none()
                            .then(|| with_reference(&element, &address.to_string(), source))
                    }
                    _ => None,
                };
                let element = match positioned {
                    Some(element) => {
                        changed = true;
                        element?
                    }
                    None => element.into_owned(),
                };
                let event = if empty {
                    Event::Empty(element)
                } else {
                    depth = element_depth;
                    Event::Start(element)
                };
                writer
                    .write_event(event)
                    .map_err(|error| invalid_xml(source, error))?;
            }
            Event::End(element) => {
                if sheet_data_depth == Some(depth) {
                    sheet_data_depth = None;
                }
                depth = depth.saturating_sub(1);
                writer
                    .write_event(Event::End(element.into_owned()))
                    .map_err(|error| invalid_xml(source, error))?;
            }
            Event::Eof => break,
            other => writer
                .write_event(other.into_owned())
                .map_err(|error| invalid_xml(source, error))?,
        }
        buffer.clear();
    }
    Ok(changed.then(|| writer.into_inner().into_inner()))
}

/// Returns the unqualified `r` attribute's value.
fn reference(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<Option<String>, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == "r" {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map(|value| Some(value.into_owned()))
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Ok(None)
}

/// Returns `element` with `r` as its first attribute, the place Excel writes it.
fn with_reference(
    element: &BytesStart<'_>,
    reference: &str,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    patched.push_attribute(("r", reference));
    for attribute in element.attributes().with_checks(true) {
        patched.push_attribute(attribute.map_err(|error| invalid_xml(source, error))?);
    }
    Ok(patched.into_owned())
}

fn is_spreadsheet_element(xml: &NsReader<&[u8]>, name: QName<'_>) -> bool {
    matches!(
        xml.resolver().resolve_element(name).0,
        ResolveResult::Bound(namespace)
            if namespace.as_ref() == SPREADSHEETML_TRANSITIONAL
                || namespace.as_ref() == SPREADSHEETML_STRICT
    )
}

fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn invalid_generated(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(DETAIL_IMPLIED_POSITION)
        .at_source(source.source_id())
}
