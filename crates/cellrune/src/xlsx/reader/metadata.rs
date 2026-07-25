use std::collections::{BTreeMap, BTreeSet};

use quick_xml::events::Event;

use super::super::error::detail;
use super::super::package::PartPath;
use super::super::xml::{
    XmlAttributes, XmlBudget, is_element_in_namespace, is_spreadsheet_element, read_attributes,
    reader, require_spreadsheet_element,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};

const METADATA: &[u8] = b"metadata";
const METADATA_TYPES: &[u8] = b"metadataTypes";
const METADATA_TYPE: &[u8] = b"metadataType";
const FUTURE_METADATA: &[u8] = b"futureMetadata";
const CELL_METADATA: &[u8] = b"cellMetadata";
const BLOCK: &[u8] = b"bk";
const RECORD: &[u8] = b"rc";
const DYNAMIC_ARRAY_PROPERTIES: &[u8] = b"dynamicArrayProperties";
const DYNAMIC_ARRAY_NAMESPACE: &[u8] =
    b"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray";
const DYNAMIC_ARRAY_METADATA_TYPE: &str = "XLDAPR";

#[derive(Debug, Default)]
pub(super) struct CellMetadata(Vec<bool>);

impl CellMetadata {
    pub(super) fn is_dynamic_array(&self, one_based_index: u32) -> Option<bool> {
        let index = one_based_index.checked_sub(1)? as usize;
        self.0.get(index).copied()
    }
}

#[derive(Debug)]
struct MetadataRecord {
    type_index: usize,
    value_index: usize,
}

#[derive(Debug)]
struct FutureMetadataGroup {
    depth: u64,
    name: Box<str>,
    records: Vec<bool>,
}

struct MetadataElement<'a> {
    is_spreadsheet: bool,
    is_dynamic: bool,
    local_name: &'a [u8],
    depth: u64,
    attributes: &'a XmlAttributes,
    empty: bool,
}

#[derive(Debug, Default)]
struct MetadataParseState {
    stack: Vec<Box<[u8]>>,
    saw_root: bool,
    saw_metadata_types: bool,
    saw_cell_metadata: bool,
    metadata_types: Vec<Box<str>>,
    metadata_type_names: BTreeSet<Box<str>>,
    future_records: BTreeMap<Box<str>, Vec<bool>>,
    current_future: Option<FutureMetadataGroup>,
    future_block: Option<(u64, bool)>,
    cell_metadata_depth: Option<u64>,
    cell_block: Option<(u64, Vec<MetadataRecord>)>,
    cell_blocks: Vec<Vec<MetadataRecord>>,
    metadata_record_count: u64,
}

pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<CellMetadata, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(
        limits,
        source.source_id(),
        XlsxErrorCode::InvalidCellMetadata,
    );
    let mut state = MetadataParseState::default();

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidCellMetadata)
                .with_cause(error)
        })?;
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                let is_dynamic = is_element_in_namespace(
                    &xml,
                    element.name(),
                    DYNAMIC_ARRAY_NAMESPACE,
                    &budget,
                )?;
                process_element(
                    MetadataElement {
                        is_spreadsheet,
                        is_dynamic,
                        local_name: &local_name,
                        depth,
                        attributes: &attributes,
                        empty: false,
                    },
                    &budget,
                    &mut state,
                )?;
                state.stack.push(local_name.into_boxed_slice());
            }
            Event::Empty(element) => {
                let depth = budget.empty()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                let is_dynamic = is_element_in_namespace(
                    &xml,
                    element.name(),
                    DYNAMIC_ARRAY_NAMESPACE,
                    &budget,
                )?;
                process_element(
                    MetadataElement {
                        is_spreadsheet,
                        is_dynamic,
                        local_name: &local_name,
                        depth,
                        attributes: &attributes,
                        empty: true,
                    },
                    &budget,
                    &mut state,
                )?;
            }
            Event::End(element) => {
                let depth = budget.end()?;
                let local_name = element.local_name().as_ref().to_vec();
                finish_element(&local_name, depth, &budget, &mut state)?;
                state
                    .stack
                    .pop()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
            }
            Event::DocType(_) => {
                return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    budget.finish(state.saw_root)?;
    if state.current_future.is_some()
        || state.future_block.is_some()
        || state.cell_metadata_depth.is_some()
        || state.cell_block.is_some()
    {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    resolve_cell_metadata(state, &budget)
}

fn process_element(
    element: MetadataElement<'_>,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    if element.depth == 1 {
        require_spreadsheet_element(element.is_spreadsheet, element.local_name, METADATA, budget)?;
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
        }
        state.saw_root = true;
        return Ok(());
    }
    if element.is_dynamic && element.local_name == DYNAMIC_ARRAY_PROPERTIES {
        if let Some((_, dynamic)) = &mut state.future_block {
            *dynamic = parse_bool(element.attributes.unqualified("fDynamic"), false, budget)?;
        }
        return Ok(());
    }
    if !element.is_spreadsheet {
        return Ok(());
    }
    if element.depth == 2 && element.local_name == METADATA_TYPES {
        if std::mem::replace(&mut state.saw_metadata_types, true) {
            return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
        }
    } else if element.depth == 2 && element.local_name == FUTURE_METADATA {
        begin_future_metadata(
            element.attributes,
            element.depth,
            element.empty,
            budget,
            state,
        )?;
    } else if element.depth == 2 && element.local_name == CELL_METADATA {
        if std::mem::replace(&mut state.saw_cell_metadata, true)
            || (!element.empty && state.cell_metadata_depth.replace(element.depth).is_some())
        {
            return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
        }
    } else if element.depth == 3
        && element.local_name == METADATA_TYPE
        && state.stack.get(1).map(|name| name.as_ref()) == Some(METADATA_TYPES)
    {
        insert_metadata_type(element.attributes, budget, state)?;
    } else if element.depth == 3 && element.local_name == BLOCK && state.current_future.is_some() {
        begin_future_block(element.depth, element.empty, budget, state)?;
    } else if element.depth == 3
        && element.local_name == BLOCK
        && state.cell_metadata_depth.is_some()
    {
        begin_cell_block(element.depth, element.empty, budget, state)?;
    } else if element.depth == 4 && element.local_name == RECORD && state.cell_block.is_some() {
        insert_cell_record(element.attributes, budget, state)?;
    }
    Ok(())
}

fn begin_future_metadata(
    attributes: &XmlAttributes,
    depth: u64,
    empty: bool,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    let name = required(attributes.unqualified("name"), "name", budget)?;
    if state.future_records.contains_key(name) || state.current_future.is_some() {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    if empty {
        state
            .future_records
            .insert(name.to_owned().into_boxed_str(), Vec::new());
    } else {
        state.current_future = Some(FutureMetadataGroup {
            depth,
            name: name.to_owned().into_boxed_str(),
            records: Vec::new(),
        });
    }
    Ok(())
}

fn insert_metadata_type(
    attributes: &XmlAttributes,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    let name = required(attributes.unqualified("name"), "name", budget)?;
    let name = name.to_owned().into_boxed_str();
    if !state.metadata_type_names.insert(name.clone()) {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    state.metadata_types.push(name);
    Ok(())
}

fn begin_future_block(
    depth: u64,
    empty: bool,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    if state.future_block.is_some() {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    if empty {
        state
            .current_future
            .as_mut()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?
            .records
            .push(false);
    } else {
        state.future_block = Some((depth, false));
    }
    Ok(())
}

fn begin_cell_block(
    depth: u64,
    empty: bool,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    if state.cell_blocks.len() as u64 >= budget.limits().max_total_cells()
        || state.cell_block.is_some()
    {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    if empty {
        state.cell_blocks.push(Vec::new());
    } else {
        state.cell_block = Some((depth, Vec::new()));
    }
    Ok(())
}

fn insert_cell_record(
    attributes: &XmlAttributes,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    state.metadata_record_count = state.metadata_record_count.saturating_add(1);
    if state.metadata_record_count > budget.limits().max_total_cells() {
        return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
    }
    let type_index = required_usize(attributes.unqualified("t"), "t", budget)?;
    let value_index = required_usize(attributes.unqualified("v"), "v", budget)?;
    state
        .cell_block
        .as_mut()
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?
        .1
        .push(MetadataRecord {
            type_index,
            value_index,
        });
    Ok(())
}

fn finish_element(
    local_name: &[u8],
    depth: u64,
    budget: &XmlBudget,
    state: &mut MetadataParseState,
) -> Result<(), XlsxReadError> {
    if state
        .future_block
        .as_ref()
        .is_some_and(|(block, _)| *block == depth && local_name == BLOCK)
    {
        let (_, dynamic) = state
            .future_block
            .take()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
        state
            .current_future
            .as_mut()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?
            .records
            .push(dynamic);
    } else if state
        .cell_block
        .as_ref()
        .is_some_and(|(block, _)| *block == depth && local_name == BLOCK)
    {
        let (_, records) = state
            .cell_block
            .take()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
        state.cell_blocks.push(records);
    } else if state
        .current_future
        .as_ref()
        .is_some_and(|future| future.depth == depth && local_name == FUTURE_METADATA)
    {
        let future = state
            .current_future
            .take()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
        if state
            .future_records
            .insert(future.name, future.records)
            .is_some()
        {
            return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
        }
    } else if state.cell_metadata_depth == Some(depth) && local_name == CELL_METADATA {
        state.cell_metadata_depth = None;
    }
    Ok(())
}

fn resolve_cell_metadata(
    state: MetadataParseState,
    budget: &XmlBudget,
) -> Result<CellMetadata, XlsxReadError> {
    let mut dynamic_cells = Vec::with_capacity(state.cell_blocks.len());
    for records in state.cell_blocks {
        let mut dynamic = false;
        let mut saw_dynamic_record = false;
        for record in records {
            let type_index = record
                .type_index
                .checked_sub(1)
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
            let metadata_type = state
                .metadata_types
                .get(type_index)
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
            if metadata_type.as_ref() == DYNAMIC_ARRAY_METADATA_TYPE {
                if std::mem::replace(&mut saw_dynamic_record, true) {
                    return Err(budget.error(XlsxErrorCode::InvalidCellMetadata));
                }
                dynamic = *state
                    .future_records
                    .get(metadata_type)
                    .and_then(|values| values.get(record.value_index))
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))?;
            }
        }
        dynamic_cells.push(dynamic);
    }
    Ok(CellMetadata(dynamic_cells))
}

fn required<'a>(
    value: Option<&'a str>,
    name: &str,
    budget: &XmlBudget,
) -> Result<&'a str, XlsxReadError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidCellMetadata)
            .with_detail(format!("{} {name}", detail::MISSING_ATTRIBUTE))),
    }
}

fn required_usize(
    value: Option<&str>,
    name: &str,
    budget: &XmlBudget,
) -> Result<usize, XlsxReadError> {
    required(value, name, budget)?
        .parse::<usize>()
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidCellMetadata)
                .with_cause(error)
        })
}

fn parse_bool(
    value: Option<&str>,
    default: bool,
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    match value {
        None => Ok(default),
        Some("0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        Some(value) => Err(budget
            .error(XlsxErrorCode::InvalidCellMetadata)
            .with_detail(value.to_owned())),
    }
}
