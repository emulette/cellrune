//! Patches column widths and row heights into a preserved worksheet.

use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::NsReader;
use quick_xml::{Writer, XmlVersion};

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::presentation::standard_column_width;
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{namespace_reader, read_event};
use crate::{DocumentPresentation, SheetId};

const DETAIL_MISSING_SHEET_DATA: &str = "worksheet does not contain sheetData";
const DETAIL_COLUMN_SPAN: &str = "worksheet column span has no valid min and max";
const DETAIL_ROW_NUMBER: &str = "worksheet row has no row number";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

/// The column widths and row heights a draft changed on one sheet.
#[derive(Debug, Default, PartialEq)]
pub(super) struct LayoutChanges {
    /// Inclusive column spans and their new width; `None` clears the width.
    columns: Vec<(u32, u32, Option<f64>)>,
    /// Rows and their new height; `None` clears the height.
    rows: BTreeMap<u32, Option<f64>>,
}

impl LayoutChanges {
    /// Returns the changes from the source layout of a sheet to the draft's.
    /// Compares the draft sheet `sheet_id` with the source sheet `source_id`, which differs only
    /// for a copy, whose part duplicates its source sheet's.
    pub(super) fn between(
        source: &DocumentPresentation,
        source_id: SheetId,
        draft: &DocumentPresentation,
        sheet_id: SheetId,
    ) -> Self {
        let width_spans = |presentation: &DocumentPresentation, sheet_id: SheetId| {
            presentation
                .column_widths(sheet_id)
                .iter()
                .map(|span| (span.first().get(), span.last().get(), span.width()))
                .collect::<Vec<_>>()
        };
        let source_spans = width_spans(source, source_id);
        let draft_spans = width_spans(draft, sheet_id);
        let mut boundaries = source_spans
            .iter()
            .chain(&draft_spans)
            .flat_map(|(first, last, _)| [*first, last + 1])
            .collect::<Vec<_>>();
        boundaries.sort_unstable();
        boundaries.dedup();
        let width_at = |spans: &[(u32, u32, f64)], column: u32| {
            spans
                .iter()
                .find(|(first, last, _)| *first <= column && column <= *last)
                .map(|(_, _, width)| *width)
        };
        let mut columns = Vec::<(u32, u32, Option<f64>)>::new();
        for pair in boundaries.windows(2) {
            let (first, last) = (pair[0], pair[1] - 1);
            let width = width_at(&draft_spans, first);
            if width == width_at(&source_spans, first) {
                continue;
            }
            match columns.last_mut() {
                Some(previous) if previous.1 + 1 == first && previous.2 == width => {
                    previous.1 = last;
                }
                _ => columns.push((first, last, width)),
            }
        }
        let source_rows = source.row_heights(source_id).collect::<BTreeMap<_, _>>();
        let draft_rows = draft.row_heights(sheet_id).collect::<BTreeMap<_, _>>();
        let rows = source_rows
            .keys()
            .chain(draft_rows.keys())
            .filter(|row| source_rows.get(row) != draft_rows.get(row))
            .map(|row| (row.get(), draft_rows.get(row).copied()))
            .collect();
        Self { columns, rows }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.rows.is_empty()
    }

    fn width_change(&self, column: u32) -> Option<Option<f64>> {
        self.columns
            .iter()
            .find(|(first, last, _)| *first <= column && column <= *last)
            .map(|(_, _, width)| *width)
    }
}

/// One `<col>` span and its attributes other than `min` and `max`.
struct ColumnSpan {
    min: u32,
    max: u32,
    attributes: Vec<(String, String)>,
}

/// Rewrites `<cols>` and the `ht`/`customHeight` of changed rows, inserting rows that hold no
/// cell when they gain a height; every other byte of the worksheet keeps its source form.
pub(super) fn patch_sheet_layout(
    bytes: &[u8],
    source: &PartPath,
    changes: &LayoutChanges,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut xml = configured_reader(bytes, limits);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut source_columns = Vec::<ColumnSpan>::new();
    let mut cols_name = None::<String>;
    let mut in_cols = false;
    let mut wrote_cols = changes.columns.is_empty();
    let mut sheet_data = None::<String>;
    let mut standard_width = StandardWidth::default();
    let mut pending_rows = changes
        .rows
        .iter()
        .filter_map(|(row, height)| height.map(|height| (*row, height)))
        .collect::<BTreeMap<_, _>>();
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                let name = element.local_name().as_ref().to_owned();
                if depth == 2 && name == "sheetFormatPr" {
                    standard_width = StandardWidth::of(&element);
                }
                if depth == 2 && name == "cols" && !changes.columns.is_empty() {
                    in_cols = true;
                    cols_name = Some(element.name().as_ref().to_owned());
                } else if in_cols {
                    if depth == 3 && name == "col" {
                        source_columns.push(column_span(&element, source)?);
                    }
                } else if depth == 2 && name == "sheetData" {
                    if !wrote_cols {
                        write_columns(
                            &mut writer,
                            &element,
                            cols_name.as_deref(),
                            &source_columns,
                            changes,
                            &standard_width,
                            source,
                        )?;
                        wrote_cols = true;
                    }
                    sheet_data = Some(element.name().as_ref().to_owned());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if depth == 3 && sheet_data.is_some() && name == "row" {
                    let row = row_number(&element, source)?;
                    write_pending_rows(&mut writer, &mut pending_rows, row, &element, source)?;
                    write_event(
                        &mut writer,
                        Event::Start(patch_row(&element, row, changes, source)?),
                        source,
                    )?;
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                let name = element.local_name().as_ref().to_owned();
                if depth + 1 == 2 && name == "sheetFormatPr" {
                    standard_width = StandardWidth::of(&element);
                }
                if depth + 1 == 2 && name == "cols" && !changes.columns.is_empty() {
                    cols_name = Some(element.name().as_ref().to_owned());
                } else if in_cols {
                    if depth + 1 == 3 && name == "col" {
                        source_columns.push(column_span(&element, source)?);
                    }
                } else if depth + 1 == 2 && name == "sheetData" {
                    if !wrote_cols {
                        write_columns(
                            &mut writer,
                            &element,
                            cols_name.as_deref(),
                            &source_columns,
                            changes,
                            &standard_width,
                            source,
                        )?;
                        wrote_cols = true;
                    }
                    let qualified = element.name().as_ref().to_owned();
                    if pending_rows.is_empty() {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    } else {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                        write_pending_rows(
                            &mut writer,
                            &mut pending_rows,
                            u32::MAX,
                            &BytesStart::new(qualified_sibling(&qualified, "row")),
                            source,
                        )?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(qualified.as_str())),
                            source,
                        )?;
                    }
                    sheet_data = Some(qualified);
                } else if depth + 1 == 3 && sheet_data.is_some() && name == "row" {
                    let row = row_number(&element, source)?;
                    write_pending_rows(&mut writer, &mut pending_rows, row, &element, source)?;
                    write_event(
                        &mut writer,
                        Event::Empty(patch_row(&element, row, changes, source)?),
                        source,
                    )?;
                } else {
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if in_cols {
                    if depth == 2 {
                        in_cols = false;
                    }
                } else if depth == 2 && element.local_name().as_ref() == "sheetData" {
                    let row_name = qualified_sibling(element.name().as_ref(), "row");
                    write_pending_rows(
                        &mut writer,
                        &mut pending_rows,
                        u32::MAX,
                        &BytesStart::new(row_name),
                        source,
                    )?;
                    write_event(&mut writer, Event::End(element.into_owned()), source)?;
                } else {
                    write_event(&mut writer, Event::End(element.into_owned()), source)?;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => {
                if !in_cols {
                    write_event(&mut writer, other.into_owned(), source)?;
                }
            }
        }
        buffer.clear();
    }
    if sheet_data.is_none() {
        return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
            .with_detail(DETAIL_MISSING_SHEET_DATA)
            .at_source(source.source_id()));
    }
    let output = writer.into_inner().into_inner();
    if output.len() as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            DETAIL_XML_BYTES,
            output.len() as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    Ok(output)
}

/// Writes the patched `<cols>` before `<sheetData>`, the position the schema gives it.
fn write_columns(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sheet_data: &BytesStart<'_>,
    cols_name: Option<&str>,
    source_columns: &[ColumnSpan],
    changes: &LayoutChanges,
    standard_width: &StandardWidth,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let columns = patched_columns(source_columns, changes, standard_width);
    if columns.is_empty() {
        return Ok(());
    }
    let cols = cols_name
        .map(str::to_owned)
        .unwrap_or_else(|| qualified_sibling(sheet_data.name().as_ref(), "cols"));
    let col = qualified_sibling(&cols, "col");
    write_event(writer, Event::Start(BytesStart::new(cols.as_str())), source)?;
    for span in columns {
        let mut element = BytesStart::new(col.as_str());
        let min = span.min.to_string();
        let max = span.max.to_string();
        element.push_attribute(("min", min.as_str()));
        element.push_attribute(("max", max.as_str()));
        for (key, value) in &span.attributes {
            element.push_attribute((key.as_str(), value.as_str()));
        }
        write_event(writer, Event::Empty(element), source)?;
    }
    write_event(writer, Event::End(BytesEnd::new(cols.as_str())), source)
}

/// The width a column without its own shows, from the worksheet's `sheetFormatPr`.
#[derive(Debug, Default)]
struct StandardWidth {
    declared: Option<f64>,
    base: Option<u32>,
}

impl StandardWidth {
    fn of(element: &BytesStart<'_>) -> Self {
        let attribute = |name: &str| {
            element
                .try_get_attribute(name)
                .ok()
                .flatten()
                .map(|value| value.value.into_owned())
        };
        Self {
            declared: attribute("defaultColWidth").and_then(|value| value.parse().ok()),
            base: attribute("baseColWidth").and_then(|value| value.parse().ok()),
        }
    }

    /// Returns the `width` of a `<col>` that keeps the standard width. Excel reads a `<col>`
    /// without `width` as zero wide, hiding the column (Excel 16.114, excel-measure I), so a span
    /// kept for its style or other attributes needs one; the reader reads it back as no width.
    fn text(&self) -> String {
        standard_column_width(self.declared, self.base).to_string()
    }
}

/// Splits the source spans at the changed columns, sets or clears their width, and adds spans for
/// changed columns no source span covers. A span whose width is cleared but that keeps other
/// attributes, such as a style, takes the standard width.
fn patched_columns(
    source_columns: &[ColumnSpan],
    changes: &LayoutChanges,
    standard_width: &StandardWidth,
) -> Vec<ColumnSpan> {
    let mut boundaries = source_columns
        .iter()
        .flat_map(|span| [span.min, span.max.saturating_add(1)])
        .chain(
            changes
                .columns
                .iter()
                .flat_map(|(first, last, _)| [*first, last.saturating_add(1)]),
        )
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut spans = Vec::<ColumnSpan>::new();
    for pair in boundaries.windows(2) {
        let (min, max) = (pair[0], pair[1] - 1);
        let source_span = source_columns
            .iter()
            .rev()
            .find(|span| span.min <= min && max <= span.max);
        let mut attributes = source_span.map_or_else(Vec::new, |span| span.attributes.clone());
        match changes.width_change(min) {
            Some(Some(width)) => {
                attributes.retain(|(key, _)| key != "width" && key != "customWidth");
                attributes.push(("width".to_owned(), width.to_string()));
                attributes.push(("customWidth".to_owned(), "1".to_owned()));
            }
            Some(None) => {
                attributes.retain(|(key, _)| key != "width" && key != "customWidth");
                if !attributes.is_empty() {
                    attributes.push(("width".to_owned(), standard_width.text()));
                }
            }
            None if source_span.is_none() => continue,
            None => {}
        }
        if attributes.is_empty() {
            continue;
        }
        match spans.last_mut() {
            Some(previous) if previous.max + 1 == min && previous.attributes == attributes => {
                previous.max = max;
            }
            _ => spans.push(ColumnSpan {
                min,
                max,
                attributes,
            }),
        }
    }
    spans
}

/// Writes the rows that gain a height and precede `before`, which hold no cell.
fn write_pending_rows(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    pending: &mut BTreeMap<u32, f64>,
    before: u32,
    row_template: &BytesStart<'_>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = row_template.name().as_ref().to_owned();
    while let Some((&row, &height)) = pending.first_key_value() {
        if row >= before {
            if row == before {
                pending.remove(&row);
            }
            break;
        }
        pending.remove(&row);
        let mut element = BytesStart::new(name.as_str());
        let number = row.to_string();
        let height = height.to_string();
        element.push_attribute(("r", number.as_str()));
        element.push_attribute(("ht", height.as_str()));
        element.push_attribute(("customHeight", "1"));
        write_event(writer, Event::Empty(element), source)?;
    }
    Ok(())
}

fn patch_row(
    element: &BytesStart<'_>,
    row: u32,
    changes: &LayoutChanges,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let Some(height) = changes.rows.get(&row) else {
        return Ok(element.clone().into_owned());
    };
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if !matches!(attribute.key.as_ref(), "ht" | "customHeight") {
            patched.push_attribute(attribute);
        }
    }
    if let Some(height) = height {
        let height = height.to_string();
        patched.push_attribute(("ht", height.as_str()));
        patched.push_attribute(("customHeight", "1"));
    }
    Ok(patched.into_owned())
}

fn column_span(element: &BytesStart<'_>, source: &PartPath) -> Result<ColumnSpan, XlsxWriteError> {
    let mut min = None;
    let mut max = None;
    let mut attributes = Vec::new();
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|error| invalid_xml(source, error))?
            .into_owned();
        match attribute.key.as_ref() {
            "min" => min = value.parse::<u32>().ok(),
            "max" => max = value.parse::<u32>().ok(),
            key => attributes.push((key.to_owned(), value)),
        }
    }
    match (min, max) {
        (Some(min), Some(max)) if min <= max => Ok(ColumnSpan {
            min,
            max,
            attributes,
        }),
        _ => Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
            .with_detail(DETAIL_COLUMN_SPAN)
            .at_source(source.source_id())),
    }
}

fn row_number(element: &BytesStart<'_>, source: &PartPath) -> Result<u32, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == "r" {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| invalid_xml(source, error))?
                .parse::<u32>()
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(DETAIL_ROW_NUMBER)
        .at_source(source.source_id()))
}

/// Returns `local_name` qualified with the prefix of `template`.
fn qualified_sibling(template: &str, local_name: &str) -> String {
    let prefix = template.rfind(':').map_or("", |index| &template[..=index]);
    format!("{prefix}{local_name}")
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

fn write_event<'a>(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    event: Event<'a>,
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
