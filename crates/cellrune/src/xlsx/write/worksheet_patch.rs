use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::serialization::escape_text;
use super::serialization::number_to_xlsx_text;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{SPREADSHEETML_STRICT, SPREADSHEETML_TRANSITIONAL};
use crate::{CellAddress, CellValue, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

const DETAIL_MISSING_FORMULA_CELL: &str =
    "worksheet does not contain the formula cell selected for materialization";
const DETAIL_MISSING_FORMULA_ELEMENT: &str =
    "materialization target does not contain a formula element";
const DETAIL_DUPLICATE_TARGET_CELL: &str =
    "worksheet contains a duplicate materialization target cell";
const DETAIL_ROW_ORDER: &str = "worksheet rows are not in ascending order";
const DETAIL_CELL_ORDER: &str = "worksheet cells are not in ascending column order";
const DETAIL_ROW_RANGE: &str = "worksheet row number is outside the supported range";
const DETAIL_MISSING_ATTRIBUTE: &str = "worksheet element does not declare a required attribute";
const DETAIL_CELL_WITHOUT_ROW: &str = "worksheet cell appears outside an open row element";
const DETAIL_MISSING_SHEET_DATA: &str = "worksheet does not contain sheetData";
const DETAIL_BLANK_CACHE: &str = "blank calculation results have no XLSX cache representation";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug, Clone)]
pub(crate) enum WorksheetCacheAction {
    Set(CellValue),
    Invalidate,
}

#[derive(Debug, Clone)]
pub(crate) struct WorksheetCellUpdate {
    pub(crate) action: WorksheetCacheAction,
    pub(crate) requires_formula: bool,
}

struct CellPatch {
    depth: u64,
    update: WorksheetCellUpdate,
    saw_formula: bool,
    wrote_value: bool,
    skipped_value_depth: Option<u64>,
}

struct RowPatch {
    depth: u64,
    number: u32,
    last_column: u32,
    qualified_name: Vec<u8>,
}

pub(crate) fn patch_worksheet(
    bytes: &[u8],
    source: &PartPath,
    updates: &BTreeMap<CellAddress, WorksheetCellUpdate>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    if bytes.len() as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            DETAIL_XML_BYTES,
            bytes.len() as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    let serialized = updates
        .iter()
        .map(|(address, update)| {
            Ok((
                *address,
                (
                    update.clone(),
                    SerializedCache::from_action(&update.action, source)?,
                ),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, XlsxWriteError>>()?;
    let mut xml = NsReader::from_reader(bytes);
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut sheet_data_depth = None;
    let mut sheet_data_name = None;
    let mut row = None::<RowPatch>;
    let mut cell = None::<CellPatch>;
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
                if cell
                    .as_ref()
                    .and_then(|state| state.skipped_value_depth)
                    .is_some()
                {
                    buffer.clear();
                    continue;
                }
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                let local = element.local_name();
                if let Some(state) = &mut cell {
                    if depth == state.depth + 1 && spreadsheet && local.as_ref() == b"v" {
                        state.skipped_value_depth = Some(depth);
                    } else {
                        if depth == state.depth + 1 && spreadsheet && local.as_ref() == b"f" {
                            state.saw_formula = true;
                        }
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                    }
                } else if spreadsheet
                    && local.as_ref() == b"sheetData"
                    && sheet_data_depth.is_none()
                {
                    sheet_data_depth = Some(depth);
                    sheet_data_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheet_data_depth.is_some_and(|value| depth == value + 1)
                    && spreadsheet
                    && local.as_ref() == b"row"
                {
                    let number = required_row_number(&element, source)?;
                    if number <= last_row {
                        return Err(invalid_generated(source, DETAIL_ROW_ORDER));
                    }
                    insert_missing_rows_before(
                        &mut writer,
                        &serialized,
                        &mut seen,
                        last_row,
                        number,
                        sheet_data_name.as_deref().unwrap_or(b"sheetData"),
                        source,
                    )?;
                    last_row = number;
                    row = Some(RowPatch {
                        depth,
                        number,
                        last_column: 0,
                        qualified_name: element.name().as_ref().to_vec(),
                    });
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if row.as_ref().is_some_and(|value| depth == value.depth + 1)
                    && spreadsheet
                    && local.as_ref() == b"c"
                {
                    let address = required_cell_reference(&element, source)?;
                    let row_state = row
                        .as_mut()
                        .ok_or_else(|| invalid_generated(source, DETAIL_CELL_WITHOUT_ROW))?;
                    if address.row().get() != row_state.number
                        || address.column().get() <= row_state.last_column
                    {
                        return Err(invalid_generated(source, DETAIL_CELL_ORDER));
                    }
                    insert_missing_cells_before(
                        &mut writer,
                        &serialized,
                        &mut seen,
                        row_state,
                        address.column().get(),
                        source,
                    )?;
                    row_state.last_column = address.column().get();
                    if let Some((update, cache)) = serialized.get(&address) {
                        if !seen.insert(address) {
                            return Err(invalid_generated(source, DETAIL_DUPLICATE_TARGET_CELL));
                        }
                        let patched = patch_cell_start(&element, cache.cell_type, source)?;
                        write_event(&mut writer, Event::Start(patched), source)?;
                        cell = Some(CellPatch {
                            depth,
                            update: update.clone(),
                            saw_formula: false,
                            wrote_value: false,
                            skipped_value_depth: None,
                        });
                    } else {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                    }
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if cell
                    .as_ref()
                    .and_then(|state| state.skipped_value_depth)
                    .is_some()
                {
                    buffer.clear();
                    continue;
                }
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                let local = element.local_name().as_ref().to_vec();
                let value_name = qualified_sibling_name(element.name().as_ref(), b"v");
                if let Some(state) = &mut cell {
                    if depth + 1 == state.depth + 1 && spreadsheet && local == b"v" {
                        // The stale empty cache is removed.
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                        if depth + 1 == state.depth + 1 && spreadsheet && local == b"f" {
                            state.saw_formula = true;
                            write_cache_if_set(
                                &mut writer,
                                serialized_cache(&state.update, source)?,
                                value_name,
                                source,
                            )?;
                            state.wrote_value = true;
                        }
                    }
                } else if sheet_data_depth == Some(depth) && spreadsheet && local == b"row" {
                    let number = required_row_number(&element, source)?;
                    if number <= last_row {
                        return Err(invalid_generated(source, DETAIL_ROW_ORDER));
                    }
                    insert_missing_rows_before(
                        &mut writer,
                        &serialized,
                        &mut seen,
                        last_row,
                        number,
                        sheet_data_name.as_deref().unwrap_or(b"sheetData"),
                        source,
                    )?;
                    last_row = number;
                    let qualified_name = element.name().as_ref().to_vec();
                    let (row_start, row_end) = row_address_bounds(number);
                    let has_updates = serialized
                        .range(row_start..=row_end)
                        .any(|(address, _)| !seen.contains(address));
                    if has_updates {
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                        let mut row_state = RowPatch {
                            depth: depth + 1,
                            number,
                            last_column: 0,
                            qualified_name: qualified_name.clone(),
                        };
                        insert_missing_cells_before(
                            &mut writer,
                            &serialized,
                            &mut seen,
                            &mut row_state,
                            u32::MAX,
                            source,
                        )?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(decode_name(&qualified_name, source)?)),
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if row.as_ref().is_some_and(|value| depth == value.depth)
                    && spreadsheet
                    && local == b"c"
                {
                    let address = required_cell_reference(&element, source)?;
                    let row_state = row
                        .as_mut()
                        .ok_or_else(|| invalid_generated(source, DETAIL_CELL_WITHOUT_ROW))?;
                    if address.row().get() != row_state.number
                        || address.column().get() <= row_state.last_column
                    {
                        return Err(invalid_generated(source, DETAIL_CELL_ORDER));
                    }
                    insert_missing_cells_before(
                        &mut writer,
                        &serialized,
                        &mut seen,
                        row_state,
                        address.column().get(),
                        source,
                    )?;
                    row_state.last_column = address.column().get();
                    if let Some((update, cache)) = serialized.get(&address) {
                        if !seen.insert(address) {
                            return Err(invalid_generated(source, DETAIL_DUPLICATE_TARGET_CELL));
                        }
                        if update.requires_formula {
                            return Err(invalid_generated(source, DETAIL_MISSING_FORMULA_ELEMENT));
                        }
                        let qualified_name = element.name().as_ref().to_vec();
                        let patched = patch_cell_start(&element, cache.cell_type, source)?;
                        write_event(&mut writer, Event::Start(patched), source)?;
                        write_cache_if_set(&mut writer, cache.clone(), value_name, source)?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(decode_name(&qualified_name, source)?)),
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
                let current_depth = depth;
                let local = element.local_name().as_ref().to_vec();
                let value_name = qualified_sibling_name(element.name().as_ref(), b"v");
                if let Some(state) = &mut cell {
                    if let Some(skipped) = state.skipped_value_depth {
                        if current_depth == skipped {
                            state.skipped_value_depth = None;
                        }
                        depth = depth.saturating_sub(1);
                        buffer.clear();
                        continue;
                    }
                    if current_depth == state.depth + 1 && local == b"f" {
                        write_event(&mut writer, Event::End(element.into_owned()), source)?;
                        let cache = serialized_cache(&state.update, source)?;
                        write_cache_if_set(&mut writer, cache, value_name, source)?;
                        state.wrote_value = true;
                        depth = depth.saturating_sub(1);
                        buffer.clear();
                        continue;
                    }
                    if current_depth == state.depth && local == b"c" {
                        if state.update.requires_formula && !state.saw_formula {
                            return Err(invalid_generated(source, DETAIL_MISSING_FORMULA_ELEMENT));
                        }
                        if !state.wrote_value {
                            let cache = serialized_cache(&state.update, source)?;
                            write_cache_if_set(&mut writer, cache, value_name, source)?;
                        }
                        write_event(&mut writer, Event::End(element.into_owned()), source)?;
                        cell = None;
                        depth = depth.saturating_sub(1);
                        buffer.clear();
                        continue;
                    }
                }
                if let Some(row_state) = &mut row
                    && current_depth == row_state.depth
                    && local == b"row"
                {
                    insert_missing_cells_before(
                        &mut writer,
                        &serialized,
                        &mut seen,
                        row_state,
                        u32::MAX,
                        source,
                    )?;
                    write_event(&mut writer, Event::End(element.into_owned()), source)?;
                    row = None;
                } else if sheet_data_depth == Some(current_depth) && local == b"sheetData" {
                    insert_missing_rows_before(
                        &mut writer,
                        &serialized,
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
                if cell
                    .as_ref()
                    .and_then(|state| state.skipped_value_depth)
                    .is_none()
                {
                    write_event(&mut writer, other.into_owned(), source)?;
                }
            }
        }
        buffer.clear();
    }
    if sheet_data_depth.is_none() {
        return Err(invalid_generated(source, DETAIL_MISSING_SHEET_DATA));
    }
    if let Some((address, update)) = serialized
        .iter()
        .find(|(address, _)| !seen.contains(*address))
    {
        let detail = if update.0.requires_formula {
            DETAIL_MISSING_FORMULA_CELL
        } else {
            DETAIL_MISSING_SHEET_DATA
        };
        return Err(invalid_generated(source, detail).with_detail(format!("{detail}: {address}")));
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

#[derive(Clone)]
struct SerializedCache {
    cell_type: Option<&'static str>,
    value: Option<String>,
}

impl SerializedCache {
    fn from_action(
        action: &WorksheetCacheAction,
        source: &PartPath,
    ) -> Result<Self, XlsxWriteError> {
        match action {
            WorksheetCacheAction::Invalidate => Ok(Self {
                cell_type: None,
                value: None,
            }),
            WorksheetCacheAction::Set(CellValue::Blank) => Err(XlsxWriteError::new(
                XlsxWriteErrorCode::UnsupportedResultMaterialization,
            )
            .with_detail(DETAIL_BLANK_CACHE)
            .at_source(source.source_id())),
            WorksheetCacheAction::Set(CellValue::Number(number)) => Ok(Self {
                cell_type: Some("n"),
                value: Some(number_to_xlsx_text(number.get())),
            }),
            WorksheetCacheAction::Set(CellValue::Text(text)) => Ok(Self {
                cell_type: Some("str"),
                value: Some(text.clone()),
            }),
            WorksheetCacheAction::Set(CellValue::Logical(value)) => Ok(Self {
                cell_type: Some("b"),
                value: Some(if *value { "1" } else { "0" }.to_owned()),
            }),
            WorksheetCacheAction::Set(CellValue::Error(error)) => Ok(Self {
                cell_type: Some("e"),
                value: Some(error.as_str().to_owned()),
            }),
        }
    }
}

fn serialized_cache(
    update: &WorksheetCellUpdate,
    source: &PartPath,
) -> Result<SerializedCache, XlsxWriteError> {
    SerializedCache::from_action(&update.action, source)
}

fn insert_missing_rows_before(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    updates: &BTreeMap<CellAddress, (WorksheetCellUpdate, SerializedCache)>,
    seen: &mut BTreeSet<CellAddress>,
    after_row: u32,
    before_row: u32,
    sheet_data_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let first_row = after_row.saturating_add(1).max(1);
    let last_row = before_row.saturating_sub(1).min(EXCEL_MAX_ROWS);
    if first_row > last_row {
        return Ok(());
    }
    let first = cell_address(first_row, 1);
    let last = cell_address(last_row, EXCEL_MAX_COLUMNS);
    let rows = updates
        .range(first..=last)
        .filter(|(address, _)| !seen.contains(*address))
        .map(|(address, _)| address.row().get())
        .collect::<BTreeSet<_>>();
    let row_name = qualified_sibling_name(sheet_data_name, b"row");
    for row in rows {
        let mut start = BytesStart::new(decode_name(&row_name, source)?);
        let row_text = row.to_string();
        start.push_attribute(("r", row_text.as_str()));
        write_event(writer, Event::Start(start), source)?;
        let (first, last) = row_address_bounds(row);
        let addresses = updates
            .range(first..=last)
            .filter(|(address, _)| !seen.contains(*address))
            .map(|(address, _)| *address)
            .collect::<Vec<_>>();
        for address in addresses {
            let (update, cache) = updates
                .get(&address)
                .expect("address was selected from the same update map");
            if update.requires_formula {
                return Err(invalid_generated(source, DETAIL_MISSING_FORMULA_CELL)
                    .with_detail(format!("{DETAIL_MISSING_FORMULA_CELL}: {address}")));
            }
            write_inserted_cell(writer, address, cache, &row_name, source)?;
            seen.insert(address);
        }
        write_event(
            writer,
            Event::End(BytesEnd::new(decode_name(&row_name, source)?)),
            source,
        )?;
    }
    Ok(())
}

fn insert_missing_cells_before(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    updates: &BTreeMap<CellAddress, (WorksheetCellUpdate, SerializedCache)>,
    seen: &mut BTreeSet<CellAddress>,
    row: &mut RowPatch,
    before_column: u32,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let first_column = row.last_column.saturating_add(1).max(1);
    let last_column = before_column.saturating_sub(1).min(EXCEL_MAX_COLUMNS);
    if first_column > last_column {
        return Ok(());
    }
    let first = cell_address(row.number, first_column);
    let last = cell_address(row.number, last_column);
    let addresses = updates
        .range(first..=last)
        .filter(|(address, _)| !seen.contains(*address))
        .map(|(address, _)| *address)
        .collect::<Vec<_>>();
    for address in addresses {
        let (update, cache) = updates
            .get(&address)
            .expect("address was selected from the same update map");
        if update.requires_formula {
            return Err(invalid_generated(source, DETAIL_MISSING_FORMULA_CELL)
                .with_detail(format!("{DETAIL_MISSING_FORMULA_CELL}: {address}")));
        }
        write_inserted_cell(writer, address, cache, &row.qualified_name, source)?;
        seen.insert(address);
        row.last_column = address.column().get();
    }
    Ok(())
}

fn row_address_bounds(row: u32) -> (CellAddress, CellAddress) {
    (cell_address(row, 1), cell_address(row, EXCEL_MAX_COLUMNS))
}

fn cell_address(row: u32, column: u32) -> CellAddress {
    CellAddress::from_indices(row, column)
        .expect("worksheet patch bounds are valid Excel coordinates")
}

fn write_inserted_cell(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    address: CellAddress,
    cache: &SerializedCache,
    row_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let cell_name = qualified_sibling_name(row_name, b"c");
    let mut start = BytesStart::new(decode_name(&cell_name, source)?);
    let reference = address.to_string();
    start.push_attribute(("r", reference.as_str()));
    if let Some(cell_type) = cache.cell_type {
        start.push_attribute(("t", cell_type));
    }
    write_event(writer, Event::Start(start), source)?;
    write_cache_if_set(
        writer,
        cache.clone(),
        qualified_sibling_name(row_name, b"v"),
        source,
    )?;
    write_event(
        writer,
        Event::End(BytesEnd::new(decode_name(&cell_name, source)?)),
        source,
    )
}

fn write_cache_if_set(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    cache: SerializedCache,
    qualified_name: Vec<u8>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let Some(value) = cache.value else {
        return Ok(());
    };
    let name = decode_name(&qualified_name, source)?;
    write_event(writer, Event::Start(BytesStart::new(name)), source)?;
    write_event(
        writer,
        Event::Text(BytesText::from_escaped(escape_text(&value)?)),
        source,
    )?;
    write_event(writer, Event::End(BytesEnd::new(name)), source)
}

fn patch_cell_start(
    element: &BytesStart<'_>,
    cell_type: Option<&str>,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != b"t" {
            patched.push_attribute(attribute);
        }
    }
    if let Some(cell_type) = cell_type {
        patched.push_attribute(("t", cell_type));
    }
    Ok(patched.into_owned())
}

fn required_cell_reference(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<CellAddress, XlsxWriteError> {
    let value = required_attribute(element, b"r", source)?;
    CellAddress::from_a1(&value).map_err(|error| {
        invalid_generated(source, DETAIL_CELL_ORDER)
            .with_detail(value)
            .with_cause(error)
    })
}

fn required_u32_attribute(
    element: &BytesStart<'_>,
    name: &[u8],
    source: &PartPath,
) -> Result<u32, XlsxWriteError> {
    let value = required_attribute(element, name, source)?;
    value
        .parse::<u32>()
        .map_err(|error| invalid_xml(source, error))
}

fn required_row_number(element: &BytesStart<'_>, source: &PartPath) -> Result<u32, XlsxWriteError> {
    let row = required_u32_attribute(element, b"r", source)?;
    if !(1..=EXCEL_MAX_ROWS).contains(&row) {
        return Err(invalid_generated_context(
            source,
            DETAIL_ROW_RANGE,
            &row.to_string(),
        ));
    }
    Ok(row)
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
    Err(invalid_generated_context(
        source,
        DETAIL_MISSING_ATTRIBUTE,
        &format!(
            "{} on <{}>",
            decode_name(name, source)?,
            decode_name(element.name().as_ref(), source)?
        ),
    ))
}

fn qualified_sibling_name(template: &[u8], local_name: &[u8]) -> Vec<u8> {
    let mut output = template
        .iter()
        .rposition(|byte| *byte == b':')
        .map_or_else(Vec::new, |index| template[..=index].to_vec());
    output.extend_from_slice(local_name);
    output
}

fn decode_name<'a>(name: &'a [u8], source: &PartPath) -> Result<&'a str, XlsxWriteError> {
    std::str::from_utf8(name).map_err(|error| invalid_xml(source, error))
}

fn is_spreadsheet_element(xml: &NsReader<&[u8]>, name: QName<'_>) -> bool {
    matches!(
        xml.resolver().resolve_element(name).0,
        ResolveResult::Bound(namespace)
            if namespace.as_ref() == SPREADSHEETML_TRANSITIONAL
                || namespace.as_ref() == SPREADSHEETML_STRICT
    )
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

fn invalid_generated(source: &PartPath, detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(detail)
        .at_source(source.source_id())
}

/// Reports a generated-XML failure with the shared message plus what triggered it.
///
/// `with_detail` replaces the detail rather than appending to it, so the shared
/// constant is formatted in explicitly. Without this the caller-visible detail
/// would be only the bare value, with no statement of what was wrong with it.
fn invalid_generated_context(
    source: &PartPath,
    detail: &'static str,
    context: &str,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(format!("{detail}: {context}"))
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
#[path = "worksheet_patch_tests.rs"]
mod tests;
