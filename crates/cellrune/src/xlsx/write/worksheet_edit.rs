use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::NsReader;

use super::serialization::serialize_cell;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::{Cell, CellAddress, CellPresentation, CellRange};

const DETAIL_ROW_ORDER: &str = "worksheet rows are not in ascending order";
const DETAIL_CELL_ORDER: &str = "worksheet cells are not in ascending column order";
const DETAIL_MISSING_SHEET_DATA: &str = "worksheet does not contain sheetData";
const DETAIL_MISSING_EDIT_TARGET: &str = "worksheet does not contain the declared edit target";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Clone)]
pub(crate) enum WorksheetSemanticEdit {
    Remove,
    Upsert {
        cell: Cell,
        style_index: usize,
        content_changed: bool,
        presentation: Option<CellPresentation>,
    },
}

struct RowState {
    depth: u64,
    number: u32,
    last_column: u32,
    qualified_name: Vec<u8>,
}

pub(crate) fn read_cell_style_indices(
    bytes: &[u8],
    source: &PartPath,
    targets: &BTreeSet<CellAddress>,
    limits: WriteLimits,
) -> Result<BTreeMap<CellAddress, usize>, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = configured_reader(bytes);
    let mut buffer = Vec::new();
    let mut output = BTreeMap::new();
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        if let Event::Start(element) | Event::Empty(element) = &event
            && element.local_name().as_ref() == b"c"
        {
            let address = required_cell_reference(element, source)?;
            if targets.contains(&address) {
                let index = optional_u32_attribute(element, b"s", source)?.unwrap_or(0);
                let index = usize::try_from(index).map_err(|error| invalid_xml(source, error))?;
                output.insert(address, index);
            }
        }
        if matches!(event, Event::Eof) {
            break;
        }
        buffer.clear();
    }
    Ok(output)
}

pub(crate) fn patch_worksheet_semantics(
    bytes: &[u8],
    source: &PartPath,
    edits: &BTreeMap<CellAddress, WorksheetSemanticEdit>,
    used_range: Option<CellRange>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut worksheet_depth = None;
    let mut sheet_data_depth = None;
    let mut sheet_data_name = None;
    let mut row = None::<RowState>;
    let mut skip_cell_depth = None::<u64>;
    let mut seen = BTreeSet::new();
    let mut last_row = 0_u32;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if skip_cell_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if depth == 1 && element.local_name().as_ref() == b"worksheet" {
                    worksheet_depth = Some(depth);
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if worksheet_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"dimension"
                {
                    let patched = patch_dimension(&element, used_range, source)?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if worksheet_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"sheetData"
                {
                    sheet_data_depth = Some(depth);
                    sheet_data_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheet_data_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"row"
                {
                    let number = required_u32_attribute(&element, b"r", source)?;
                    if number <= last_row {
                        return Err(invalid_generated(source, DETAIL_ROW_ORDER));
                    }
                    insert_missing_rows(
                        &mut writer,
                        edits,
                        &mut seen,
                        last_row,
                        number,
                        sheet_data_name.as_deref().unwrap_or(b"sheetData"),
                        source,
                    )?;
                    last_row = number;
                    row = Some(RowState {
                        depth,
                        number,
                        last_column: 0,
                        qualified_name: element.name().as_ref().to_vec(),
                    });
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if row.as_ref().is_some_and(|state| depth == state.depth + 1)
                    && element.local_name().as_ref() == b"c"
                {
                    let address = required_cell_reference(&element, source)?;
                    let row_state = row
                        .as_mut()
                        .ok_or_else(|| invalid_generated(source, DETAIL_ROW_ORDER))?;
                    validate_cell_order(row_state, address, source)?;
                    insert_missing_cells(
                        &mut writer,
                        edits,
                        &mut seen,
                        row_state,
                        address.column().get(),
                        source,
                    )?;
                    row_state.last_column = address.column().get();
                    if let Some(edit) = edits.get(&address) {
                        if !seen.insert(address) {
                            return Err(invalid_generated(source, DETAIL_CELL_ORDER));
                        }
                        write_existing_edit(
                            &mut writer,
                            edit,
                            &element,
                            &row_state.qualified_name,
                            source,
                        )?;
                        if !matches!(
                            edit,
                            WorksheetSemanticEdit::Upsert {
                                content_changed: false,
                                ..
                            }
                        ) {
                            skip_cell_depth = Some(depth);
                        }
                    } else {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                    }
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if skip_cell_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if worksheet_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"dimension"
                {
                    let patched = patch_dimension(&element, used_range, source)?;
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else if sheet_data_depth == Some(depth) && element.local_name().as_ref() == b"row"
                {
                    let number = required_u32_attribute(&element, b"r", source)?;
                    if number <= last_row {
                        return Err(invalid_generated(source, DETAIL_ROW_ORDER));
                    }
                    insert_missing_rows(
                        &mut writer,
                        edits,
                        &mut seen,
                        last_row,
                        number,
                        sheet_data_name.as_deref().unwrap_or(b"sheetData"),
                        source,
                    )?;
                    last_row = number;
                    let qualified = element.name().as_ref().to_vec();
                    if edits
                        .keys()
                        .any(|address| address.row().get() == number && !seen.contains(address))
                    {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                        let mut state = RowState {
                            depth: depth + 1,
                            number,
                            last_column: 0,
                            qualified_name: qualified.clone(),
                        };
                        insert_missing_cells(
                            &mut writer,
                            edits,
                            &mut seen,
                            &mut state,
                            u32::MAX,
                            source,
                        )?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(decode_name(&qualified, source)?)),
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if row.as_ref().is_some_and(|state| depth == state.depth)
                    && element.local_name().as_ref() == b"c"
                {
                    let address = required_cell_reference(&element, source)?;
                    let row_state = row
                        .as_mut()
                        .ok_or_else(|| invalid_generated(source, DETAIL_ROW_ORDER))?;
                    validate_cell_order(row_state, address, source)?;
                    insert_missing_cells(
                        &mut writer,
                        edits,
                        &mut seen,
                        row_state,
                        address.column().get(),
                        source,
                    )?;
                    row_state.last_column = address.column().get();
                    if let Some(edit) = edits.get(&address) {
                        if !seen.insert(address) {
                            return Err(invalid_generated(source, DETAIL_CELL_ORDER));
                        }
                        write_existing_edit(
                            &mut writer,
                            edit,
                            &element,
                            &row_state.qualified_name,
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else {
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if let Some(skipped) = skip_cell_depth {
                    if depth == skipped {
                        skip_cell_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                    buffer.clear();
                    continue;
                }
                if row.as_ref().is_some_and(|state| depth == state.depth)
                    && element.local_name().as_ref() == b"row"
                {
                    let row_state = row
                        .as_mut()
                        .ok_or_else(|| invalid_generated(source, DETAIL_ROW_ORDER))?;
                    insert_missing_cells(
                        &mut writer,
                        edits,
                        &mut seen,
                        row_state,
                        u32::MAX,
                        source,
                    )?;
                    write_event(&mut writer, Event::End(element.into_owned()), source)?;
                    row = None;
                } else if sheet_data_depth == Some(depth)
                    && element.local_name().as_ref() == b"sheetData"
                {
                    insert_missing_rows(
                        &mut writer,
                        edits,
                        &mut seen,
                        last_row,
                        u32::MAX,
                        sheet_data_name.as_deref().unwrap_or(b"sheetData"),
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
                if skip_cell_depth.is_none() {
                    write_event(&mut writer, other.into_owned(), source)?;
                }
            }
        }
        buffer.clear();
    }
    if sheet_data_depth.is_none() {
        return Err(invalid_generated(source, DETAIL_MISSING_SHEET_DATA));
    }
    if edits.keys().any(|address| !seen.contains(address)) {
        return Err(invalid_generated(source, DETAIL_MISSING_EDIT_TARGET));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

fn write_existing_edit(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    edit: &WorksheetSemanticEdit,
    original: &BytesStart<'_>,
    row_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    match edit {
        WorksheetSemanticEdit::Remove => Ok(()),
        WorksheetSemanticEdit::Upsert {
            cell: _,
            style_index,
            content_changed: false,
            ..
        } => {
            let patched = patch_style(original, *style_index, source)?;
            if original.is_empty() {
                write_event(writer, Event::Empty(patched), source)
            } else {
                write_event(writer, Event::Start(patched), source)
            }
        }
        WorksheetSemanticEdit::Upsert {
            cell,
            style_index,
            content_changed: true,
            presentation,
        } => write_serialized_cell(
            writer,
            cell,
            *style_index,
            presentation.as_ref(),
            row_name,
            source,
        ),
    }
}

fn insert_missing_rows(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    edits: &BTreeMap<CellAddress, WorksheetSemanticEdit>,
    seen: &mut BTreeSet<CellAddress>,
    after: u32,
    before: u32,
    sheet_data_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let rows = edits
        .keys()
        .filter(|address| {
            address.row().get() > after && address.row().get() < before && !seen.contains(*address)
        })
        .map(|address| address.row().get())
        .collect::<BTreeSet<_>>();
    let row_name = qualified_sibling_name(sheet_data_name, b"row");
    for number in rows {
        let actionable = edits.iter().any(|(address, edit)| {
            address.row().get() == number
                && !seen.contains(address)
                && matches!(
                    edit,
                    WorksheetSemanticEdit::Upsert {
                        content_changed: true,
                        ..
                    }
                )
        });
        if !actionable {
            continue;
        }
        let mut start = BytesStart::new(decode_name(&row_name, source)?);
        let number_text = number.to_string();
        start.push_attribute(("r", number_text.as_str()));
        write_event(writer, Event::Start(start), source)?;
        let mut state = RowState {
            depth: 0,
            number,
            last_column: 0,
            qualified_name: row_name.clone(),
        };
        insert_missing_cells(writer, edits, seen, &mut state, u32::MAX, source)?;
        write_event(
            writer,
            Event::End(BytesEnd::new(decode_name(&row_name, source)?)),
            source,
        )?;
    }
    Ok(())
}

fn insert_missing_cells(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    edits: &BTreeMap<CellAddress, WorksheetSemanticEdit>,
    seen: &mut BTreeSet<CellAddress>,
    row: &mut RowState,
    before_column: u32,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let addresses = edits
        .keys()
        .filter(|address| {
            address.row().get() == row.number
                && address.column().get() > row.last_column
                && address.column().get() < before_column
                && !seen.contains(*address)
        })
        .copied()
        .collect::<Vec<_>>();
    for address in addresses {
        match edits
            .get(&address)
            .ok_or_else(|| invalid_generated(source, DETAIL_CELL_ORDER))?
        {
            WorksheetSemanticEdit::Upsert {
                cell,
                style_index,
                content_changed: true,
                presentation,
            } => {
                write_serialized_cell(
                    writer,
                    cell,
                    *style_index,
                    presentation.as_ref(),
                    &row.qualified_name,
                    source,
                )?;
                seen.insert(address);
                row.last_column = address.column().get();
            }
            WorksheetSemanticEdit::Remove
            | WorksheetSemanticEdit::Upsert {
                content_changed: false,
                ..
            } => {}
        }
    }
    Ok(())
}

fn write_serialized_cell(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    cell: &Cell,
    style_index: usize,
    presentation: Option<&CellPresentation>,
    row_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let serialized = serialize_cell(cell, style_index, None, presentation)?;
    let prefix = qualified_prefix(row_name);
    let qualified = if prefix.is_empty() {
        serialized
    } else {
        qualify_cell_xml(&serialized, prefix)?
    };
    writer
        .get_mut()
        .write_all(qualified.as_bytes())
        .map_err(|error| invalid_xml(source, error))?;
    enforce_fragment_xml(&qualified, source)
}

fn qualify_cell_xml(xml: &str, prefix: &str) -> Result<String, XlsxWriteError> {
    let mut output = xml.to_owned();
    for name in ["c", "f", "v", "is", "t", "rPh", "phoneticPr"] {
        output = output.replace(&format!("<{name}"), &format!("<{prefix}{name}"));
        output = output.replace(&format!("</{name}>"), &format!("</{prefix}{name}>"));
    }
    Ok(output)
}

fn enforce_fragment_xml(xml: &str, source: &PartPath) -> Result<(), XlsxWriteError> {
    let wrapped = format!("<root>{xml}</root>");
    let mut reader = quick_xml::Reader::from_str(&wrapped);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(error) => return Err(invalid_xml(source, error)),
        }
    }
}

fn patch_style(
    element: &BytesStart<'_>,
    style_index: usize,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != b"s" {
            patched.push_attribute(attribute);
        }
    }
    if style_index != 0 {
        let index = style_index.to_string();
        patched.push_attribute(("s", index.as_str()));
    }
    Ok(patched.into_owned())
}

fn patch_dimension(
    element: &BytesStart<'_>,
    used_range: Option<CellRange>,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != b"ref" {
            patched.push_attribute(attribute);
        }
    }
    let reference = used_range.map_or_else(
        || "A1".to_owned(),
        |range| {
            if range.start() == range.end() {
                range.start().to_string()
            } else {
                format!("{}:{}", range.start(), range.end())
            }
        },
    );
    patched.push_attribute(("ref", reference.as_str()));
    Ok(patched.into_owned())
}

fn validate_cell_order(
    row: &RowState,
    address: CellAddress,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if address.row().get() != row.number || address.column().get() <= row.last_column {
        Err(invalid_generated(source, DETAIL_CELL_ORDER))
    } else {
        Ok(())
    }
}

fn required_cell_reference(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<CellAddress, XlsxWriteError> {
    let value = required_attribute(element, b"r", source)?;
    CellAddress::from_a1(&value).map_err(|error| invalid_xml(source, error))
}

fn required_u32_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<u32, XlsxWriteError> {
    required_attribute(element, name, source)?
        .parse::<u32>()
        .map_err(|error| invalid_xml(source, error))
}

fn optional_u32_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<Option<u32>, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == name {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| invalid_xml(source, error))?
                .parse::<u32>()
                .map(Some)
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Ok(None)
}

fn required_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
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
    Err(invalid_generated(source, DETAIL_CELL_ORDER))
}

fn configured_reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn qualified_sibling_name(template: &[u8], local_name: &[u8]) -> Vec<u8> {
    let mut output = qualified_prefix(template).as_bytes().to_vec();
    output.extend_from_slice(local_name);
    output
}

fn qualified_prefix(template: &[u8]) -> &str {
    template
        .iter()
        .rposition(|byte| *byte == b':')
        .and_then(|index| std::str::from_utf8(&template[..=index]).ok())
        .unwrap_or("")
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

fn decode_name<'a>(name: &'a [u8], source: &PartPath) -> Result<&'a str, XlsxWriteError> {
    std::str::from_utf8(name).map_err(|error| invalid_xml(source, error))
}

fn invalid_xml(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{WorksheetSemanticEdit, patch_worksheet_semantics};
    use crate::xlsx::package::PartPath;
    use crate::{
        Cell, CellAddress, CellContent, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
        NumberFormat, SavedResult, WriteLimits,
    };

    #[test]
    fn content_replacement_keeps_the_target_cell_in_row_order() {
        let source = PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part");
        let address = CellAddress::from_a1("B1").expect("B1");
        let formula = FormulaCell::new(
            FormulaDialect::ExcelA1,
            FormulaText::from_xlsx("A1*2").expect("formula"),
            SavedResult::Missing,
            FormulaMetadata::Normal,
        );
        let cell = Cell::with_content_and_number_format(
            address,
            CellContent::Formula(formula),
            NumberFormat::default(),
        );
        let edits = BTreeMap::from([(
            address,
            WorksheetSemanticEdit::Upsert {
                cell,
                style_index: 0,
                content_changed: true,
                presentation: None,
            },
        )]);
        let output = patch_worksheet_semantics(
            br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>2</v></c><c r="B1"><f>A1+1</f><v>3</v></c></row></sheetData></worksheet>"#,
            &source,
            &edits,
            None,
            WriteLimits::default(),
        )
        .expect("patch");
        let output = String::from_utf8(output).expect("UTF-8");
        assert!(output.contains(r#"<c r="B1"><f>A1*2</f></c>"#), "{output}");
    }
}
