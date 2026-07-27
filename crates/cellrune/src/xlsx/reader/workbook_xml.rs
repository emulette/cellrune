use std::collections::BTreeSet;

use quick_xml::events::Event;

use super::super::error::detail;
use super::super::package::PartPath;
use super::super::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, XmlAttributes, XmlBudget,
    decode_cdata, decode_reference, decode_text, is_spreadsheet_element, read_attributes, reader,
    require_spreadsheet_element,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use super::defined_name::DefinedNamesState;
use crate::{
    CalculationHints, CalculationMode, DateSystem, DefinedName, SheetId, SheetName, SheetVisibility,
};

const WORKBOOK: &[u8] = b"workbook";
const WORKBOOK_PROPERTIES: &[u8] = b"workbookPr";
const SHEETS: &[u8] = b"sheets";
const SHEET: &[u8] = b"sheet";
const DEFINED_NAMES: &[u8] = b"definedNames";
const DEFINED_NAME: &[u8] = b"definedName";
const CALCULATION_PROPERTIES: &[u8] = b"calcPr";

#[derive(Debug)]
pub(super) struct WorkbookMetadata {
    pub(super) sheets: Vec<SheetMetadata>,
    pub(super) defined_names: Vec<DefinedName>,
    pub(super) date_system: DateSystem,
    pub(super) calculation_hints: CalculationHints,
}

#[derive(Debug)]
pub(super) struct SheetMetadata {
    pub(super) id: SheetId,
    pub(super) name: SheetName,
    pub(super) visibility: SheetVisibility,
    pub(super) relationship_id: Box<str>,
}

#[derive(Debug)]
struct WorkbookParseState {
    stack: Vec<Box<[u8]>>,
    saw_root: bool,
    saw_workbook_properties: bool,
    saw_sheets: bool,
    saw_calculation_properties: bool,
    date_system: DateSystem,
    calculation_hints: CalculationHints,
    sheets: Vec<SheetMetadata>,
    relationship_ids: BTreeSet<Box<str>>,
    defined_names: DefinedNamesState,
}

impl Default for WorkbookParseState {
    fn default() -> Self {
        Self {
            stack: Vec::new(),
            saw_root: false,
            saw_workbook_properties: false,
            saw_sheets: false,
            saw_calculation_properties: false,
            date_system: DateSystem::Excel1900,
            calculation_hints: CalculationHints::default(),
            sheets: Vec::new(),
            relationship_ids: BTreeSet::new(),
            defined_names: DefinedNamesState::default(),
        }
    }
}

pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
) -> Result<WorkbookMetadata, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidWorkbook);
    let mut state = WorkbookParseState::default();

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidWorkbook)
                .with_cause(error)
        })?;
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                process_element(
                    is_spreadsheet,
                    &local_name,
                    depth,
                    &attributes,
                    false,
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
                process_element(
                    is_spreadsheet,
                    &local_name,
                    depth,
                    &attributes,
                    true,
                    &budget,
                    &mut state,
                )?;
                state.defined_names.finish(&local_name, depth, &budget)?;
            }
            Event::Text(text) => {
                state
                    .defined_names
                    .append(decode_text(&text, &budget)?, &budget)?;
            }
            Event::CData(text) => {
                state
                    .defined_names
                    .append(decode_cdata(&text, &budget)?, &budget)?;
            }
            Event::GeneralRef(reference) => {
                state
                    .defined_names
                    .append(decode_reference(&reference, &budget)?, &budget)?;
            }
            Event::End(element) => {
                let depth = budget.end()?;
                let local_name = element.local_name().as_ref().to_vec();
                state.defined_names.finish(&local_name, depth, &budget)?;
                state
                    .stack
                    .pop()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidWorkbook))?;
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
    if state.sheets.is_empty() {
        return Err(budget.error(XlsxErrorCode::InvalidWorkbook));
    }
    let defined_names = std::mem::take(&mut state.defined_names).resolve(&state.sheets, &budget)?;
    Ok(WorkbookMetadata {
        sheets: state.sheets,
        defined_names,
        date_system: state.date_system,
        calculation_hints: state.calculation_hints,
    })
}

fn process_element(
    is_spreadsheet: bool,
    local_name: &[u8],
    depth: u64,
    attributes: &XmlAttributes,
    empty: bool,
    budget: &XmlBudget,
    state: &mut WorkbookParseState,
) -> Result<(), XlsxReadError> {
    if depth == 1 {
        require_spreadsheet_element(is_spreadsheet, local_name, WORKBOOK, budget)?;
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidWorkbook));
        }
        state.saw_root = true;
        return Ok(());
    }
    if !is_spreadsheet {
        return Ok(());
    }
    if depth == 2 && local_name == WORKBOOK_PROPERTIES {
        if std::mem::replace(&mut state.saw_workbook_properties, true) {
            return Err(budget.error(XlsxErrorCode::InvalidWorkbook));
        }
        state.date_system = match attributes.unqualified("date1904") {
            None | Some("0" | "false") => DateSystem::Excel1900,
            Some("1" | "true") => DateSystem::Excel1904,
            Some(value) => {
                return Err(budget
                    .error(XlsxErrorCode::InvalidWorkbook)
                    .with_detail(value.to_owned()));
            }
        };
    } else if depth == 2 && local_name == SHEETS {
        if std::mem::replace(&mut state.saw_sheets, true) {
            return Err(budget.error(XlsxErrorCode::InvalidWorkbook));
        }
    } else if depth == 2 && local_name == DEFINED_NAMES {
        state.defined_names.begin_container(budget)?;
    } else if depth == 2 && local_name == CALCULATION_PROPERTIES {
        if std::mem::replace(&mut state.saw_calculation_properties, true) {
            return Err(budget.error(XlsxErrorCode::InvalidWorkbook));
        }
        state.calculation_hints = parse_calculation_hints(attributes, budget)?;
    } else if depth == 3
        && local_name == SHEET
        && state.stack.get(1).map(|name| name.as_ref()) == Some(SHEETS)
    {
        if state.sheets.len() as u64 >= budget.limits().max_sheets() {
            return Err(budget.error(XlsxErrorCode::TooManySheets));
        }
        let sheet = parse_sheet(attributes, budget)?;
        if !state.relationship_ids.insert(sheet.relationship_id.clone()) {
            return Err(budget
                .error(XlsxErrorCode::InvalidWorkbook)
                .with_detail(detail::DUPLICATE_SHEET_RELATIONSHIP));
        }
        state.sheets.push(sheet);
    } else if depth == 3
        && local_name == DEFINED_NAME
        && state.stack.get(1).map(|name| name.as_ref()) == Some(DEFINED_NAMES)
    {
        state.defined_names.begin(attributes, depth, budget)?;
        if empty {
            state.defined_names.finish(local_name, depth, budget)?;
        }
    }
    Ok(())
}

fn parse_sheet(
    attributes: &XmlAttributes,
    budget: &XmlBudget,
) -> Result<SheetMetadata, XlsxReadError> {
    let name = required(attributes.unqualified("name"), "name", budget)?;
    let sheet_id = required(attributes.unqualified("sheetId"), "sheetId", budget)?;
    let relationship_id = attributes
        .namespaced(DOCUMENT_RELATIONSHIPS_TRANSITIONAL, "id")
        .or_else(|| attributes.namespaced(DOCUMENT_RELATIONSHIPS_STRICT, "id"));
    let relationship_id = required(relationship_id, "relationship id", budget)?;
    let id = sheet_id.parse::<u32>().map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorkbook)
            .with_cause(error)
    })?;
    let id = SheetId::new(id).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorkbook)
            .with_cause(error)
    })?;
    let name = SheetName::new(name.to_owned()).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorkbook)
            .with_cause(error)
    })?;
    let visibility = match attributes.unqualified("state") {
        None | Some("visible") => SheetVisibility::Visible,
        Some("hidden") => SheetVisibility::Hidden,
        Some("veryHidden") => SheetVisibility::VeryHidden,
        Some(value) => {
            return Err(budget
                .error(XlsxErrorCode::InvalidWorkbook)
                .with_detail(value.to_owned()));
        }
    };
    Ok(SheetMetadata {
        id,
        name,
        visibility,
        relationship_id: relationship_id.to_owned().into_boxed_str(),
    })
}

fn parse_calculation_hints(
    attributes: &XmlAttributes,
    budget: &XmlBudget,
) -> Result<CalculationHints, XlsxReadError> {
    let mode = match attributes.unqualified("calcMode") {
        None => None,
        Some("auto") => Some(CalculationMode::Automatic),
        Some("autoNoTable") => Some(CalculationMode::AutomaticExceptDataTables),
        Some("manual") => Some(CalculationMode::Manual),
        Some(value) => {
            return Err(budget
                .error(XlsxErrorCode::InvalidWorkbook)
                .with_detail(value.to_owned()));
        }
    };
    let calculation_id = optional_u32(attributes.unqualified("calcId"), budget)?;
    let full_calculation_on_load = optional_bool(attributes.unqualified("fullCalcOnLoad"), budget)?;
    let force_full_calculation = optional_bool(attributes.unqualified("forceFullCalc"), budget)?;
    let iterative_calculation = optional_bool(attributes.unqualified("iterate"), budget)?;
    Ok(CalculationHints::new(
        mode,
        calculation_id,
        full_calculation_on_load,
        force_full_calculation,
    )
    .with_iterative_calculation(iterative_calculation))
}

fn required<'a>(
    value: Option<&'a str>,
    name: &str,
    budget: &XmlBudget,
) -> Result<&'a str, XlsxReadError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidWorkbook)
            .with_detail(format!("{} {name}", detail::MISSING_ATTRIBUTE))),
    }
}

fn optional_u32(value: Option<&str>, budget: &XmlBudget) -> Result<Option<u32>, XlsxReadError> {
    value
        .map(|value| {
            value.parse::<u32>().map_err(|error| {
                budget
                    .error(XlsxErrorCode::InvalidWorkbook)
                    .with_cause(error)
            })
        })
        .transpose()
}

fn optional_bool(value: Option<&str>, budget: &XmlBudget) -> Result<Option<bool>, XlsxReadError> {
    value
        .map(|value| match value {
            "0" | "false" => Ok(false),
            "1" | "true" => Ok(true),
            _ => Err(budget
                .error(XlsxErrorCode::InvalidWorkbook)
                .with_detail(value.to_owned())),
        })
        .transpose()
}
