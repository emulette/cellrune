use quick_xml::events::Event;

use super::super::error::compatibility;
use super::super::package::PartPath;
use super::super::xml::{XmlAttributes, XmlBudget, is_spreadsheet_element, read_attributes, reader};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::{
    CellAddress, CellRange, Diagnostic, DiagnosticCode, DiagnosticSeverity, SheetId,
    SourceLocation, Table, TableColumn, TableName, TotalsRowFunction,
};

const TABLE: &[u8] = b"table";
const TABLE_COLUMNS: &[u8] = b"tableColumns";
const TABLE_COLUMN: &[u8] = b"tableColumn";

mod reason {
    pub(super) const ROOT_NOT_TABLE: &str = "root element is not a table";
    pub(super) const MISSING_DISPLAY_NAME: &str = "missing displayName attribute";
    pub(super) const MISSING_REF: &str = "missing ref attribute";
    pub(super) const INVALID_REF: &str = "invalid ref attribute";
    pub(super) const INVALID_ROW_COUNT: &str = "invalid header or totals row count";
    pub(super) const DUPLICATE_TABLE_COLUMNS: &str = "duplicate tableColumns element";
    pub(super) const MISSING_COLUMN_ID: &str = "missing or invalid tableColumn id";
    pub(super) const MISSING_COLUMN_NAME: &str = "missing tableColumn name";
    pub(super) const INVALID_TOTALS_FUNCTION: &str = "unknown totalsRowFunction token";
}

#[derive(Debug, Default)]
struct TableParseState {
    saw_root: bool,
    name: Option<Box<str>>,
    display_name: Option<Box<str>>,
    reference: Option<CellRange>,
    header_row_count: u32,
    totals_row_count: u32,
    columns: Vec<TableColumn>,
    column_count: u64,
    table_columns_depth: Option<u64>,
    saw_table_columns: bool,
    invalid: Option<String>,
}

impl TableParseState {
    fn invalidate(&mut self, reason: impl Into<String>) {
        if self.invalid.is_none() {
            self.invalid = Some(reason.into());
        }
    }
}

/// Parses one CT_Table part into a validated [`Table`].
///
/// Semantic invalidity (bad references, name-rule violations, inconsistent counts) reports
/// an `xlsx.table.invalid` diagnostic and returns `Ok(None)` so only that table is dropped.
/// Only malformed XML and configured read limits fail the read as a whole.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the XML is malformed, exceeds a configured XML budget,
/// or exceeds the table column-count or name byte-length limits.
pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
    sheet_id: SheetId,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Table>, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidXml);
    let mut state = TableParseState::default();

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| {
            budget.error(XlsxErrorCode::InvalidXml).with_cause(error)
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
                    true,
                    &attributes,
                    limits,
                    &mut state,
                    &budget,
                )?;
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
                    false,
                    &attributes,
                    limits,
                    &mut state,
                    &budget,
                )?;
            }
            Event::End(element) => {
                let depth = budget.end()?;
                if state.table_columns_depth == Some(depth)
                    && element.local_name().as_ref() == TABLE_COLUMNS
                {
                    state.table_columns_depth = None;
                }
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

    if let Some(reason) = state.invalid.take() {
        push_invalid_diagnostic(diagnostics, &reason, sheet_id, &budget)?;
        return Ok(None);
    }
    let Some(display_name) = state.display_name.take() else {
        push_invalid_diagnostic(diagnostics, reason::MISSING_DISPLAY_NAME, sheet_id, &budget)?;
        return Ok(None);
    };
    let Some(reference) = state.reference.take() else {
        push_invalid_diagnostic(diagnostics, reason::MISSING_REF, sheet_id, &budget)?;
        return Ok(None);
    };
    // OOXML defaults @name to @displayName when absent.
    let raw_name = state.name.take().unwrap_or_else(|| display_name.clone());
    let name = match TableName::new(raw_name.as_ref()) {
        Ok(name) => name,
        Err(error) => {
            push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
            return Ok(None);
        }
    };
    match Table::new(
        name,
        display_name.as_ref(),
        reference,
        state.header_row_count,
        state.totals_row_count,
        std::mem::take(&mut state.columns),
    ) {
        Ok(table) => Ok(Some(table)),
        Err(error) => {
            push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
            Ok(None)
        }
    }
}

fn process_element(
    is_spreadsheet: bool,
    local_name: &[u8],
    depth: u64,
    has_children: bool,
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if depth == 1 {
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidXml));
        }
        state.saw_root = true;
        if !is_spreadsheet || local_name != TABLE {
            state.invalidate(reason::ROOT_NOT_TABLE);
            return Ok(());
        }
        process_root_attributes(attributes, limits, state, budget)?;
        return Ok(());
    }
    if !is_spreadsheet {
        return Ok(());
    }
    if depth == 2 && local_name == TABLE_COLUMNS {
        if state.saw_table_columns {
            state.invalidate(reason::DUPLICATE_TABLE_COLUMNS);
            return Ok(());
        }
        state.saw_table_columns = true;
        // A self-closing <tableColumns/> has no children, so it must not leave the depth
        // marker armed for unrelated elements that happen to sit at the same depth later.
        if has_children {
            state.table_columns_depth = Some(depth);
        }
    } else if state
        .table_columns_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == TABLE_COLUMN
    {
        process_column(attributes, limits, state, budget)?;
    }
    Ok(())
}

fn process_root_attributes(
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if let Some(name) = attributes.unqualified("name") {
        check_name_bytes(name, limits, budget)?;
        state.name = Some(Box::from(name));
    }
    if let Some(display_name) = attributes.unqualified("displayName") {
        check_name_bytes(display_name, limits, budget)?;
        state.display_name = Some(Box::from(display_name));
    }
    match attributes.unqualified("ref") {
        Some(reference) => match parse_reference(reference) {
            Some(range) => state.reference = Some(range),
            None => state.invalidate(reason::INVALID_REF),
        },
        None => state.invalidate(reason::MISSING_REF),
    }
    match parse_count(attributes.unqualified("headerRowCount"), 1) {
        Some(value) => state.header_row_count = value,
        None => state.invalidate(reason::INVALID_ROW_COUNT),
    }
    match parse_count(attributes.unqualified("totalsRowCount"), 0) {
        Some(value) => state.totals_row_count = value,
        None => state.invalidate(reason::INVALID_ROW_COUNT),
    }
    Ok(())
}

fn process_column(
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    // Read limits are a parse budget: they apply to every declared column, including the
    // ones in a table that has already been marked semantically invalid.
    state.column_count = state.column_count.saturating_add(1);
    if state.column_count > limits.max_table_columns() {
        return Err(budget.error(XlsxErrorCode::TooManyTableColumns));
    }
    if let Some(name) = attributes.unqualified("name") {
        check_name_bytes(name, limits, budget)?;
    }
    if state.invalid.is_some() {
        return Ok(());
    }
    let Some(id) = attributes
        .unqualified("id")
        .and_then(|value| value.parse::<u32>().ok())
    else {
        state.invalidate(reason::MISSING_COLUMN_ID);
        return Ok(());
    };
    let Some(name) = attributes.unqualified("name") else {
        state.invalidate(reason::MISSING_COLUMN_NAME);
        return Ok(());
    };
    let totals_row_function = match attributes.unqualified("totalsRowFunction") {
        None | Some("none") => None,
        Some(token) => match parse_totals_row_function(token) {
            Some(function) => Some(function),
            None => {
                state.invalidate(reason::INVALID_TOTALS_FUNCTION);
                return Ok(());
            }
        },
    };
    match TableColumn::new(id, name, totals_row_function) {
        Ok(column) => state.columns.push(column),
        Err(error) => state.invalidate(error.to_string()),
    }
    Ok(())
}

fn parse_reference(reference: &str) -> Option<CellRange> {
    let mut parts = reference.split(':');
    let start = CellAddress::from_a1(parts.next()?).ok()?;
    let end = match parts.next() {
        Some(end) => CellAddress::from_a1(end).ok()?,
        None => start,
    };
    if parts.next().is_some() {
        return None;
    }
    CellRange::new(start, end).ok()
}

fn parse_count(value: Option<&str>, default: u32) -> Option<u32> {
    match value {
        None => Some(default),
        Some(value) => value.parse::<u32>().ok(),
    }
}

fn parse_totals_row_function(token: &str) -> Option<TotalsRowFunction> {
    match token {
        "sum" => Some(TotalsRowFunction::Sum),
        "min" => Some(TotalsRowFunction::Min),
        "max" => Some(TotalsRowFunction::Max),
        "average" => Some(TotalsRowFunction::Average),
        "count" => Some(TotalsRowFunction::Count),
        "countNums" => Some(TotalsRowFunction::CountNumbers),
        "stdDev" => Some(TotalsRowFunction::StdDev),
        "var" => Some(TotalsRowFunction::Var),
        "custom" => Some(TotalsRowFunction::Custom),
        _ => None,
    }
}

fn check_name_bytes(
    value: &str,
    limits: ReadLimits,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if value.len() as u64 > limits.max_table_name_bytes() {
        return Err(budget.error(XlsxErrorCode::TableNameTooLarge));
    }
    Ok(())
}

pub(super) fn push_invalid_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    detail: &str,
    sheet_id: SheetId,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    push_table_diagnostic(
        diagnostics,
        compatibility::TABLE_INVALID_CODE,
        compatibility::TABLE_INVALID_MESSAGE,
        detail,
        sheet_id,
        budget,
    )
}

pub(super) fn push_table_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: &'static str,
    message: &'static str,
    detail: &str,
    sheet_id: SheetId,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let code = DiagnosticCode::new(code).map_err(|error| {
        budget.error(XlsxErrorCode::InvalidXml).with_cause(error)
    })?;
    let diagnostic = Diagnostic::new(
        code,
        DiagnosticSeverity::Warning,
        format!("{message}: {detail}"),
        Some(SourceLocation::sheet(budget.source_id().clone(), sheet_id)),
    )
    .map_err(|error| budget.error(XlsxErrorCode::InvalidXml).with_cause(error))?;
    diagnostics.push(diagnostic);
    Ok(())
}
