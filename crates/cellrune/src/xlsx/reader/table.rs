use std::collections::BTreeSet;
use std::sync::Arc;

use quick_xml::events::Event;

use super::super::error::compatibility;
use super::super::package::PartPath;
use super::super::xml::{
    SPREADSHEETML_STRICT, XmlAttributes, XmlBudget, decode_cdata, decode_reference, decode_text,
    is_element_in_namespace, is_spreadsheet_element, read_attributes, reader,
    validate_processing_instruction, validate_xml_declaration,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::{
    CellAddress, CellRange, Diagnostic, DiagnosticCode, DiagnosticSeverity, SheetId,
    SourceLocation, Table, TableAutoFilter, TableCalendarType, TableColorFilter, TableColumn,
    TableCustomFilter, TableCustomFilterOperator, TableCustomFilters, TableDateGroupItem,
    TableDateTimeGrouping, TableDateTimeValue, TableDynamicFilter, TableDynamicFilterType,
    TableFilterColumn, TableFilterCriteria, TableFilterItem, TableFormula, TableIconFilter,
    TableIconSet, TableId, TableName, TableNumericValue, TableSortBy, TableSortCondition,
    TableSortMethod, TableSortState, TableStyleInfo, TableTopFilter, TableType, TableValueFilters,
    TotalsRowFunction,
};

const TABLE: &[u8] = b"table";
const AUTO_FILTER: &[u8] = b"autoFilter";
const SORT_STATE: &[u8] = b"sortState";
const SORT_CONDITION: &[u8] = b"sortCondition";
const FILTER_COLUMN: &[u8] = b"filterColumn";
const FILTERS: &[u8] = b"filters";
const FILTER: &[u8] = b"filter";
const DATE_GROUP_ITEM: &[u8] = b"dateGroupItem";
const CUSTOM_FILTERS: &[u8] = b"customFilters";
const CUSTOM_FILTER: &[u8] = b"customFilter";
const DYNAMIC_FILTER: &[u8] = b"dynamicFilter";
const COLOR_FILTER: &[u8] = b"colorFilter";
const ICON_FILTER: &[u8] = b"iconFilter";
const TOP_TEN: &[u8] = b"top10";
const TABLE_COLUMNS: &[u8] = b"tableColumns";
const TABLE_COLUMN: &[u8] = b"tableColumn";
const CALCULATED_COLUMN_FORMULA: &[u8] = b"calculatedColumnFormula";
const TOTALS_ROW_FORMULA: &[u8] = b"totalsRowFormula";
const XML_COLUMN_PROPERTIES: &[u8] = b"xmlColumnPr";
const TABLE_STYLE_INFO: &[u8] = b"tableStyleInfo";
const EXTENSION_LIST: &[u8] = b"extLst";
const MAX_SORT_CONDITIONS: usize = 64;

mod reason {
    pub(super) const ROOT_NOT_TABLE: &str = "root element is not a table";
    pub(super) const MISSING_TABLE_ID: &str = "missing or invalid table id";
    pub(super) const MISSING_DISPLAY_NAME: &str = "missing displayName attribute";
    pub(super) const MISSING_REF: &str = "missing ref attribute";
    pub(super) const INVALID_REF: &str = "invalid ref attribute";
    pub(super) const INVALID_ROW_COUNT: &str = "invalid header or totals row count";
    pub(super) const INVALID_TABLE_TYPE: &str = "invalid tableType token";
    pub(super) const INVALID_BOOLEAN: &str = "invalid table boolean attribute";
    pub(super) const DUPLICATE_TABLE_COLUMNS: &str = "duplicate tableColumns element";
    pub(super) const DUPLICATE_AUTO_FILTER: &str = "duplicate autoFilter element";
    pub(super) const DUPLICATE_SORT_STATE: &str = "duplicate sortState element";
    pub(super) const DUPLICATE_TABLE_STYLE: &str = "duplicate tableStyleInfo element";
    pub(super) const INVALID_TABLE_COLUMNS_COUNT: &str = "invalid tableColumns count";
    pub(super) const TABLE_COLUMNS_COUNT_MISMATCH: &str =
        "tableColumns count does not match the number of tableColumn children";
    pub(super) const MISSING_COLUMN_ID: &str = "missing or invalid tableColumn id";
    pub(super) const MISSING_COLUMN_NAME: &str = "missing tableColumn name";
    pub(super) const INVALID_TOTALS_FUNCTION: &str = "unknown totalsRowFunction token";
    pub(super) const DUPLICATE_COLUMN_FORMULA: &str = "duplicate table column formula element";
    pub(super) const INVALID_AUTO_FILTER_REF: &str = "missing or invalid autoFilter ref";
    pub(super) const INVALID_SORT_STATE_REF: &str = "missing or invalid sortState ref";
    pub(super) const INVALID_FILTER_COLUMN_ID: &str = "missing or invalid filterColumn colId";
    pub(super) const DUPLICATE_FILTER_COLUMN_ID: &str =
        "duplicate filterColumn colId in one autoFilter";
    pub(super) const FILTER_COLUMN_OUT_OF_RANGE: &str =
        "filterColumn colId exceeds the autoFilter range width";
    pub(super) const INVALID_FILTER_CRITERIA: &str = "invalid table filter criteria";
    pub(super) const DUPLICATE_FILTER_CRITERIA: &str =
        "filterColumn declares more than one filter criteria";
    pub(super) const TOO_MANY_SORT_CONDITIONS: &str =
        "sortState exceeds the 64-condition OOXML limit";
    pub(super) const INVALID_CHILD_ORDER: &str = "table children do not follow the OOXML sequence";
}

struct ParsedColumn {
    column: TableColumn,
    totals_row_label: Option<String>,
    calculated_column_formula: Option<TableFormula>,
    totals_row_formula: Option<TableFormula>,
}

#[derive(Clone, Copy)]
enum FormulaKind {
    CalculatedColumn,
    TotalsRow,
}

struct FormulaCapture {
    depth: u64,
    column_index: Option<usize>,
    kind: FormulaKind,
    array: bool,
    text: String,
    bytes_seen: u64,
}

struct PendingSortState {
    depth: u64,
    range: CellRange,
    case_sensitive: bool,
    column_sort: bool,
    sort_method: Option<TableSortMethod>,
    conditions: Vec<TableSortCondition>,
}

impl PendingSortState {
    fn finish(self) -> TableSortState {
        TableSortState::from_xlsx(
            self.range,
            self.case_sensitive,
            self.column_sort,
            self.sort_method,
            self.conditions,
        )
    }
}

struct PendingFilterColumn {
    depth: u64,
    column: TableFilterColumn,
    saw_child: bool,
}

struct PendingAutoFilter {
    range: Option<CellRange>,
    range_is_explicit: bool,
    filter_columns: Vec<TableFilterColumn>,
    seen_filter_column_ids: BTreeSet<u32>,
    pending_filter_column: Option<PendingFilterColumn>,
    sort_state: Option<TableSortState>,
    pending_sort: Option<PendingSortState>,
}

enum FragmentKind {
    AutoFilter(Box<PendingAutoFilter>),
    SortState(PendingSortState),
    Invalid,
}

struct FragmentCapture {
    root_depth: u64,
    kind: FragmentKind,
    open_elements: Vec<Box<[u8]>>,
    child_phase: u8,
}

impl FragmentCapture {
    fn new(root_depth: u64, root_name: &[u8], kind: FragmentKind) -> Self {
        Self {
            root_depth,
            kind,
            open_elements: vec![Box::from(root_name)],
            child_phase: 0,
        }
    }
}

struct TableParseState {
    saw_root: bool,
    strict_spreadsheet: Option<bool>,
    root_child_phase: u8,
    id: Option<TableId>,
    name: Option<Box<str>>,
    display_name: Option<Box<str>>,
    reference: Option<CellRange>,
    table_type: TableType,
    header_row_count: u32,
    totals_row_count: u32,
    totals_row_shown: bool,
    columns: Vec<ParsedColumn>,
    column_count: u64,
    declared_column_count: Option<u32>,
    table_columns_depth: Option<u64>,
    table_column_depth: Option<(u64, Option<usize>)>,
    table_column_child_phase: u8,
    formula_capture: Option<FormulaCapture>,
    fragment_capture: Option<FragmentCapture>,
    auto_filter: Option<TableAutoFilter>,
    sort_state: Option<TableSortState>,
    style_info: Option<TableStyleInfo>,
    saw_table_columns: bool,
    saw_auto_filter: bool,
    saw_sort_state: bool,
    saw_table_style: bool,
    has_opaque_metadata: bool,
    filter_item_count: u64,
    filter_text_bytes: u64,
    normalization_warnings: Vec<&'static str>,
    invalid: Option<String>,
}

impl Default for TableParseState {
    fn default() -> Self {
        Self {
            saw_root: false,
            strict_spreadsheet: None,
            root_child_phase: 0,
            id: None,
            name: None,
            display_name: None,
            reference: None,
            table_type: TableType::Worksheet,
            header_row_count: 0,
            totals_row_count: 0,
            totals_row_shown: true,
            columns: Vec::new(),
            column_count: 0,
            declared_column_count: None,
            table_columns_depth: None,
            table_column_depth: None,
            table_column_child_phase: 0,
            formula_capture: None,
            fragment_capture: None,
            auto_filter: None,
            sort_state: None,
            style_info: None,
            saw_table_columns: false,
            saw_auto_filter: false,
            saw_sort_state: false,
            saw_table_style: false,
            has_opaque_metadata: false,
            filter_item_count: 0,
            filter_text_bytes: 0,
            normalization_warnings: Vec::new(),
            invalid: None,
        }
    }
}

impl TableParseState {
    fn invalidate(&mut self, reason: impl Into<String>) {
        if self.invalid.is_none() {
            self.invalid = Some(reason.into());
        }
    }

    fn mark_opaque(&mut self) {
        self.has_opaque_metadata = true;
    }
}

/// Parses one CT_Table part into a validated [`Table`].
///
/// Semantic invalidity (bad references or name-rule violations) reports an
/// `xlsx.table.invalid` diagnostic and returns `Ok(None)` so only that table is dropped.
/// A missing or mismatched advisory `tableColumns@count` is normalized from the actual children.
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
    total_formula_bytes: &mut u64,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Table>, XlsxReadError> {
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidXml);
    let mut state = TableParseState::default();
    let mut declaration_allowed = true;

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| budget.error(XlsxErrorCode::InvalidXml).with_cause(error))?;
        if !matches!(&event, Event::Decl(_) | Event::Eof) {
            declaration_allowed = false;
        }
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                let is_strict_spreadsheet =
                    is_element_in_namespace(&xml, element.name(), SPREADSHEETML_STRICT, &budget)?;
                let is_spreadsheet = is_spreadsheet
                    && (depth == 1 || state.strict_spreadsheet == Some(is_strict_spreadsheet));
                let element_event = ElementEvent {
                    is_spreadsheet,
                    is_strict_spreadsheet,
                    local_name: &local_name,
                    depth,
                    has_children: true,
                    attributes: &attributes,
                };
                validate_element_sequence(element_event, &mut state);
                if state.fragment_capture.is_some() {
                    process_fragment_element(element_event, limits, &mut state, &budget)?;
                } else if start_fragment(element_event, limits, &mut state, &budget)? {
                    // The fragment parser owns this subtree.
                } else {
                    process_element(element_event, limits, &mut state, &budget)?;
                }
            }
            Event::Empty(element) => {
                let depth = budget.empty()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                let is_strict_spreadsheet =
                    is_element_in_namespace(&xml, element.name(), SPREADSHEETML_STRICT, &budget)?;
                let is_spreadsheet = is_spreadsheet
                    && (depth == 1 || state.strict_spreadsheet == Some(is_strict_spreadsheet));
                let element_event = ElementEvent {
                    is_spreadsheet,
                    is_strict_spreadsheet,
                    local_name: &local_name,
                    depth,
                    has_children: false,
                    attributes: &attributes,
                };
                validate_element_sequence(element_event, &mut state);
                if state.fragment_capture.is_some() {
                    process_fragment_element(element_event, limits, &mut state, &budget)?;
                } else if start_fragment(element_event, limits, &mut state, &budget)? {
                    // The fragment parser owns this element.
                } else {
                    process_element(element_event, limits, &mut state, &budget)?;
                }
            }
            Event::End(element) => {
                let depth = budget.end()?;
                let local_name = element.local_name().as_ref().to_vec();
                if state
                    .fragment_capture
                    .as_ref()
                    .is_some_and(|capture| capture.root_depth == depth)
                {
                    finish_fragment(&mut state);
                } else if state.fragment_capture.is_some() {
                    finish_filter_column(depth, &local_name, &mut state);
                    finish_nested_sort_state(depth, &local_name, &mut state);
                    close_fragment_element(&mut state);
                } else {
                    finish_formula(depth, &local_name, &mut state, total_formula_bytes, &budget)?;
                    if state
                        .table_column_depth
                        .is_some_and(|(column_depth, _)| column_depth == depth)
                        && local_name == TABLE_COLUMN
                    {
                        state.table_column_depth = None;
                        state.table_column_child_phase = 0;
                    }
                    if state.table_columns_depth == Some(depth) && local_name == TABLE_COLUMNS {
                        state.table_columns_depth = None;
                    }
                }
            }
            Event::Text(text) => {
                process_character_data(
                    decode_text(&text, &budget)?,
                    CharacterDataKind::Text,
                    &mut state,
                    &budget,
                )?;
            }
            Event::CData(text) => {
                process_character_data(
                    decode_cdata(&text, &budget)?,
                    CharacterDataKind::CData,
                    &mut state,
                    &budget,
                )?;
            }
            Event::GeneralRef(reference) => {
                let decoded = decode_reference(&reference, &budget)?;
                process_character_data(decoded, CharacterDataKind::Reference, &mut state, &budget)?;
            }
            Event::DocType(_) => {
                return Err(budget.error(XlsxErrorCode::ForbiddenXmlConstruct));
            }
            Event::Decl(declaration) => {
                if !declaration_allowed || state.saw_root || budget.current_depth() != 0 {
                    return Err(budget.error(XlsxErrorCode::InvalidXml));
                }
                validate_xml_declaration(&declaration, &budget)?;
                declaration_allowed = false;
            }
            Event::PI(instruction) => {
                validate_processing_instruction(&instruction, &budget)?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    budget.finish(state.saw_root)?;

    if state
        .declared_column_count
        .is_some_and(|count| u64::from(count) != state.column_count)
    {
        state
            .normalization_warnings
            .push(reason::TABLE_COLUMNS_COUNT_MISMATCH);
    }
    if let Some(reason) = state.invalid.take() {
        push_invalid_diagnostic(diagnostics, &reason, sheet_id, &budget)?;
        return Ok(None);
    }
    for warning in state.normalization_warnings.drain(..) {
        push_table_diagnostic(
            diagnostics,
            compatibility::TABLE_NORMALIZED_CODE,
            compatibility::TABLE_NORMALIZED_MESSAGE,
            warning,
            sheet_id,
            &budget,
        )?;
    }
    let Some(id) = state.id else {
        push_invalid_diagnostic(diagnostics, reason::MISSING_TABLE_ID, sheet_id, &budget)?;
        return Ok(None);
    };
    let Some(display_name) = state.display_name.take() else {
        push_invalid_diagnostic(diagnostics, reason::MISSING_DISPLAY_NAME, sheet_id, &budget)?;
        return Ok(None);
    };
    let Some(reference) = state.reference.take() else {
        push_invalid_diagnostic(diagnostics, reason::MISSING_REF, sheet_id, &budget)?;
        return Ok(None);
    };
    let display_name = match TableName::from_xlsx(display_name.as_ref()) {
        Ok(name) => name,
        Err(error) => {
            push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
            return Ok(None);
        }
    };
    // OOXML defaults @name to @displayName when absent.
    let raw_name = state
        .name
        .take()
        .unwrap_or_else(|| Box::from(display_name.as_str()));
    let name = match TableName::from_xlsx(raw_name.as_ref()) {
        Ok(name) => name,
        Err(error) => {
            push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
            return Ok(None);
        }
    };
    let columns = std::mem::take(&mut state.columns)
        .into_iter()
        .map(|column| {
            column.column.with_metadata(
                column.totals_row_label,
                column.calculated_column_formula,
                column.totals_row_formula,
            )
        })
        .collect();
    match Table::new(
        id,
        name,
        display_name,
        reference,
        state.header_row_count,
        state.totals_row_count,
        columns,
    ) {
        Ok(table) => {
            match table.try_with_metadata(
                state.table_type,
                state.totals_row_shown,
                state.auto_filter,
                state.sort_state,
                state.style_info,
                state.has_opaque_metadata.then(|| bytes.to_vec()),
            ) {
                Ok(table) => Ok(Some(table)),
                Err(error) => {
                    push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
                    Ok(None)
                }
            }
        }
        Err(error) => {
            push_invalid_diagnostic(diagnostics, &error.to_string(), sheet_id, &budget)?;
            Ok(None)
        }
    }
}

fn start_fragment(
    element: ElementEvent<'_>,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    let ElementEvent {
        is_spreadsheet,
        is_strict_spreadsheet: _,
        local_name,
        depth,
        has_children,
        attributes,
    } = element;
    if !is_spreadsheet || depth != 2 || !matches!(local_name, AUTO_FILTER | SORT_STATE) {
        return Ok(false);
    }
    charge_fragment_resources(local_name, attributes, limits, state, budget)?;
    let kind = if local_name == AUTO_FILTER {
        if state.saw_auto_filter {
            state.invalidate(reason::DUPLICATE_AUTO_FILTER);
        }
        state.saw_auto_filter = true;
        mark_unknown_attributes(attributes, &["ref"], state);
        let declared_ref = attributes.unqualified("ref");
        let range_is_explicit = declared_ref.is_some();
        let range = match declared_ref {
            Some(reference) => match parse_reference(reference) {
                Some(range) => Some(range),
                None => {
                    state.invalidate(reason::INVALID_AUTO_FILTER_REF);
                    inherited_auto_filter_range(state)
                }
            },
            None => inherited_auto_filter_range(state),
        };
        FragmentKind::AutoFilter(Box::new(PendingAutoFilter {
            range,
            range_is_explicit,
            filter_columns: Vec::new(),
            seen_filter_column_ids: BTreeSet::new(),
            pending_filter_column: None,
            sort_state: None,
            pending_sort: None,
        }))
    } else {
        if state.saw_sort_state {
            state.invalidate(reason::DUPLICATE_SORT_STATE);
        }
        state.saw_sort_state = true;
        mark_unknown_attributes(
            attributes,
            &["ref", "caseSensitive", "columnSort", "sortMethod"],
            state,
        );
        match parse_sort_state(depth, attributes, state) {
            Some(pending) => FragmentKind::SortState(pending),
            None => FragmentKind::Invalid,
        }
    };
    let capture = FragmentCapture::new(depth, local_name, kind);
    state.fragment_capture = Some(capture);
    if !has_children {
        finish_fragment(state);
    }
    Ok(true)
}

fn process_fragment_element(
    element: ElementEvent<'_>,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let ElementEvent {
        is_spreadsheet,
        is_strict_spreadsheet: _,
        local_name,
        depth,
        has_children,
        attributes,
    } = element;
    charge_fragment_resources(local_name, attributes, limits, state, budget)?;
    let Some(mut capture) = state.fragment_capture.take() else {
        return Ok(());
    };
    validate_fragment_sequence(&mut capture, local_name, depth, state);
    let placement_is_known = fragment_child_allowed(&capture, local_name);
    let known_attributes = fragment_attributes(local_name);
    if !is_spreadsheet || !placement_is_known || known_attributes.is_none() {
        state.mark_opaque();
    }
    if attributes
        .iter()
        .any(|(_, namespace, _)| namespace.is_some())
    {
        state.mark_opaque();
    }
    if let Some(known_attributes) = known_attributes {
        mark_unknown_attributes(attributes, known_attributes, state);
    }
    if local_name == EXTENSION_LIST {
        state.mark_opaque();
    }
    if is_spreadsheet && placement_is_known && known_attributes.is_some() {
        process_known_fragment_element(
            &mut capture,
            local_name,
            depth,
            attributes,
            has_children,
            state,
        );
    }
    if has_children {
        capture.open_elements.push(Box::from(local_name));
    }
    state.fragment_capture = Some(capture);
    Ok(())
}

fn charge_fragment_resources(
    local_name: &[u8],
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if matches!(
        local_name,
        FILTER
            | DATE_GROUP_ITEM
            | CUSTOM_FILTER
            | DYNAMIC_FILTER
            | COLOR_FILTER
            | ICON_FILTER
            | TOP_TEN
    ) {
        state.filter_item_count = state.filter_item_count.saturating_add(1);
        if state.filter_item_count > limits.max_table_filter_items() {
            return Err(budget.error(XlsxErrorCode::TooManyTableFilterItems));
        }
    }
    let attribute_bytes = attributes.iter().fold(0_u64, |total, (_, _, value)| {
        total.saturating_add(value.len() as u64)
    });
    state.filter_text_bytes = state.filter_text_bytes.saturating_add(attribute_bytes);
    if state.filter_text_bytes > limits.max_table_filter_text_bytes() {
        return Err(budget.error(XlsxErrorCode::TableFilterTextTooLarge));
    }
    Ok(())
}

fn process_known_fragment_element(
    capture: &mut FragmentCapture,
    local_name: &[u8],
    depth: u64,
    attributes: &XmlAttributes,
    has_children: bool,
    state: &mut TableParseState,
) {
    match &mut capture.kind {
        FragmentKind::AutoFilter(pending) => {
            if depth == capture.root_depth + 1 && local_name == FILTER_COLUMN {
                let Some(column_id) = attributes
                    .unqualified("colId")
                    .and_then(|value| value.parse::<u32>().ok())
                else {
                    state.invalidate(reason::INVALID_FILTER_COLUMN_ID);
                    return;
                };
                if pending
                    .range
                    .is_some_and(|range| column_id >= range.width())
                {
                    state.invalidate(reason::FILTER_COLUMN_OUT_OF_RANGE);
                } else if !pending.seen_filter_column_ids.insert(column_id) {
                    state.invalidate(reason::DUPLICATE_FILTER_COLUMN_ID);
                } else {
                    let column = TableFilterColumn::from_xlsx(
                        column_id,
                        parse_optional_bool(attributes.unqualified("hiddenButton"), false, state),
                        parse_optional_bool(attributes.unqualified("showButton"), true, state),
                        None,
                    );
                    if has_children {
                        pending.pending_filter_column = Some(PendingFilterColumn {
                            depth,
                            column,
                            saw_child: false,
                        });
                    } else {
                        pending.filter_columns.push(column);
                    }
                }
            } else if depth == capture.root_depth + 1 && local_name == SORT_STATE {
                if pending.sort_state.is_some() || pending.pending_sort.is_some() {
                    state.invalidate(reason::DUPLICATE_SORT_STATE);
                } else if let Some(parsed) = parse_sort_state(depth, attributes, state) {
                    if has_children {
                        pending.pending_sort = Some(parsed);
                    } else {
                        pending.sort_state = Some(parsed.finish());
                    }
                }
            } else if pending
                .pending_sort
                .as_ref()
                .is_some_and(|pending| depth == pending.depth + 1)
                && local_name == SORT_CONDITION
            {
                push_sort_condition(
                    pending
                        .pending_sort
                        .as_mut()
                        .expect("pending sort checked above"),
                    attributes,
                    state,
                );
            } else if let Some(column) = &mut pending.pending_filter_column {
                process_filter_column_element(column, local_name, depth, attributes, state);
            }
        }
        FragmentKind::SortState(pending) => {
            if depth == pending.depth + 1 && local_name == SORT_CONDITION {
                push_sort_condition(pending, attributes, state);
            }
        }
        FragmentKind::Invalid => {}
    }
}

fn process_filter_column_element(
    pending: &mut PendingFilterColumn,
    local_name: &[u8],
    depth: u64,
    attributes: &XmlAttributes,
    state: &mut TableParseState,
) {
    if depth == pending.depth + 1 {
        if pending.saw_child {
            state.invalidate(reason::DUPLICATE_FILTER_CRITERIA);
            return;
        }
        pending.saw_child = true;
        if local_name == EXTENSION_LIST {
            return;
        }
        let criteria = match local_name {
            FILTERS => Some(TableFilterCriteria::Values(TableValueFilters::from_xlsx(
                parse_optional_bool(attributes.unqualified("blank"), false, state),
                parse_optional_token(
                    attributes,
                    "calendarType",
                    TableCalendarType::from_xlsx,
                    state,
                ),
                Vec::new(),
            ))),
            CUSTOM_FILTERS => Some(TableFilterCriteria::Custom(TableCustomFilters::from_xlsx(
                parse_optional_bool(attributes.unqualified("and"), false, state),
                Vec::new(),
            ))),
            DYNAMIC_FILTER => {
                parse_dynamic_filter(attributes, state).map(TableFilterCriteria::Dynamic)
            }
            COLOR_FILTER => Some(TableFilterCriteria::Color(TableColorFilter::from_xlsx(
                parse_optional_u32(attributes, "dxfId", state),
                parse_optional_bool(attributes.unqualified("cellColor"), true, state),
            ))),
            ICON_FILTER => {
                parse_required_token(attributes, "iconSet", TableIconSet::from_xlsx, state).map(
                    |icon_set| {
                        TableFilterCriteria::Icon(TableIconFilter::from_xlsx(
                            icon_set,
                            parse_optional_u32(attributes, "iconId", state),
                        ))
                    },
                )
            }
            TOP_TEN => parse_required_numeric(attributes, "val", state).map(|value| {
                TableFilterCriteria::Top(TableTopFilter::from_xlsx(
                    parse_optional_bool(attributes.unqualified("top"), true, state),
                    parse_optional_bool(attributes.unqualified("percent"), false, state),
                    value,
                    parse_optional_numeric(attributes, "filterVal", state),
                ))
            }),
            _ => None,
        };
        if let Some(criteria) = criteria {
            pending.column.set_criteria(criteria);
        }
        return;
    }
    let Some(criteria) = pending.column.criteria_mut() else {
        state.invalidate(reason::INVALID_FILTER_CRITERIA);
        return;
    };
    match (criteria, local_name) {
        (TableFilterCriteria::Values(filters), FILTER) => {
            if filters
                .items()
                .iter()
                .any(|item| matches!(item, TableFilterItem::DateGroup(_)))
            {
                state.invalidate(reason::INVALID_FILTER_CRITERIA);
            } else {
                filters.push_item(TableFilterItem::Value(
                    attributes.unqualified("val").map(Arc::from),
                ));
            }
        }
        (TableFilterCriteria::Values(filters), DATE_GROUP_ITEM) => {
            if let Some(item) = parse_date_group_item(attributes, state) {
                filters.push_item(TableFilterItem::DateGroup(item));
            }
        }
        (TableFilterCriteria::Custom(filters), CUSTOM_FILTER) => {
            filters.push_filter(TableCustomFilter::from_xlsx(
                parse_optional_token(
                    attributes,
                    "operator",
                    TableCustomFilterOperator::from_xlsx,
                    state,
                ),
                attributes.unqualified("val").map(str::to_owned),
            ));
        }
        _ => state.invalidate(reason::INVALID_FILTER_CRITERIA),
    }
}

fn parse_date_group_item(
    attributes: &XmlAttributes,
    state: &mut TableParseState,
) -> Option<TableDateGroupItem> {
    let item = TableDateGroupItem::from_xlsx(
        parse_required_u16(attributes, "year", state)?,
        parse_optional_u16(attributes, "month", state),
        parse_optional_u16(attributes, "day", state),
        parse_optional_u16(attributes, "hour", state),
        parse_optional_u16(attributes, "minute", state),
        parse_optional_u16(attributes, "second", state),
        parse_required_token(
            attributes,
            "dateTimeGrouping",
            TableDateTimeGrouping::from_xlsx,
            state,
        )?,
    );
    if item.is_none() {
        state.invalidate(reason::INVALID_FILTER_CRITERIA);
    }
    item
}

fn parse_dynamic_filter(
    attributes: &XmlAttributes,
    state: &mut TableParseState,
) -> Option<TableDynamicFilter> {
    let kind = parse_required_token(attributes, "type", TableDynamicFilterType::from_xlsx, state)?;
    if state.strict_spreadsheet == Some(true) && attributes.unqualified("maxVal").is_some() {
        state.invalidate(reason::INVALID_FILTER_CRITERIA);
        return None;
    }
    let value = parse_optional_numeric(attributes, "val", state);
    let iso_value = parse_optional_date_time(attributes, "valIso", state);
    let max_value = parse_optional_numeric(attributes, "maxVal", state);
    let max_iso_value = parse_optional_date_time(attributes, "maxValIso", state);
    let has_any_value =
        value.is_some() || iso_value.is_some() || max_value.is_some() || max_iso_value.is_some();
    match kind {
        TableDynamicFilterType::AboveAverage | TableDynamicFilterType::BelowAverage => {
            if value.is_none()
                || iso_value.is_some()
                || max_value.is_some()
                || max_iso_value.is_some()
            {
                state.mark_opaque();
            }
        }
        TableDynamicFilterType::Tomorrow
        | TableDynamicFilterType::Today
        | TableDynamicFilterType::Yesterday
        | TableDynamicFilterType::NextWeek
        | TableDynamicFilterType::ThisWeek
        | TableDynamicFilterType::LastWeek
        | TableDynamicFilterType::NextMonth
        | TableDynamicFilterType::ThisMonth
        | TableDynamicFilterType::LastMonth
        | TableDynamicFilterType::NextQuarter
        | TableDynamicFilterType::ThisQuarter
        | TableDynamicFilterType::LastQuarter
        | TableDynamicFilterType::NextYear
        | TableDynamicFilterType::ThisYear
        | TableDynamicFilterType::LastYear
        | TableDynamicFilterType::YearToDate => {
            if state.strict_spreadsheet == Some(true) && value.is_some() {
                state.mark_opaque();
            }
        }
        TableDynamicFilterType::Quarter1
        | TableDynamicFilterType::Quarter2
        | TableDynamicFilterType::Quarter3
        | TableDynamicFilterType::Quarter4
        | TableDynamicFilterType::Month1
        | TableDynamicFilterType::Month2
        | TableDynamicFilterType::Month3
        | TableDynamicFilterType::Month4
        | TableDynamicFilterType::Month5
        | TableDynamicFilterType::Month6
        | TableDynamicFilterType::Month7
        | TableDynamicFilterType::Month8
        | TableDynamicFilterType::Month9
        | TableDynamicFilterType::Month10
        | TableDynamicFilterType::Month11
        | TableDynamicFilterType::Month12 => {
            if has_any_value {
                state.mark_opaque();
            }
        }
        TableDynamicFilterType::Null => {}
    }
    Some(TableDynamicFilter::from_xlsx(
        kind,
        value,
        iso_value,
        max_value,
        max_iso_value,
    ))
}

fn validate_fragment_sequence(
    capture: &mut FragmentCapture,
    local_name: &[u8],
    depth: u64,
    state: &mut TableParseState,
) {
    if depth != capture.root_depth + 1
        || capture.open_elements.last().map(Box::as_ref) != Some(AUTO_FILTER)
    {
        return;
    }
    let phase = match local_name {
        FILTER_COLUMN => 1,
        SORT_STATE => 2,
        EXTENSION_LIST => 3,
        _ => return,
    };
    if phase < capture.child_phase {
        state.invalidate(reason::INVALID_CHILD_ORDER);
    } else {
        capture.child_phase = phase;
    }
}

fn fragment_child_allowed(capture: &FragmentCapture, local_name: &[u8]) -> bool {
    if matches!(&capture.kind, FragmentKind::Invalid) {
        return false;
    }
    let Some(parent) = capture.open_elements.last().map(Box::as_ref) else {
        return false;
    };
    match parent {
        AUTO_FILTER => matches!(local_name, FILTER_COLUMN | SORT_STATE | EXTENSION_LIST),
        FILTER_COLUMN => matches!(
            local_name,
            FILTERS
                | CUSTOM_FILTERS
                | DYNAMIC_FILTER
                | COLOR_FILTER
                | ICON_FILTER
                | TOP_TEN
                | EXTENSION_LIST
        ),
        FILTERS => matches!(local_name, FILTER | DATE_GROUP_ITEM),
        CUSTOM_FILTERS => local_name == CUSTOM_FILTER,
        SORT_STATE => matches!(local_name, SORT_CONDITION | EXTENSION_LIST),
        _ => false,
    }
}

fn close_fragment_element(state: &mut TableParseState) {
    if let Some(capture) = &mut state.fragment_capture {
        capture.open_elements.pop();
    }
}

fn finish_filter_column(depth: u64, local_name: &[u8], state: &mut TableParseState) {
    if local_name != FILTER_COLUMN {
        return;
    }
    let Some(capture) = &mut state.fragment_capture else {
        return;
    };
    let FragmentKind::AutoFilter(pending) = &mut capture.kind else {
        return;
    };
    if pending
        .pending_filter_column
        .as_ref()
        .is_some_and(|column| column.depth == depth)
        && let Some(column) = pending.pending_filter_column.take()
    {
        pending.filter_columns.push(column.column);
    }
}

fn finish_nested_sort_state(depth: u64, local_name: &[u8], state: &mut TableParseState) {
    if local_name != SORT_STATE {
        return;
    }
    let Some(capture) = &mut state.fragment_capture else {
        return;
    };
    let FragmentKind::AutoFilter(pending) = &mut capture.kind else {
        return;
    };
    if pending
        .pending_sort
        .as_ref()
        .is_some_and(|sort| sort.depth == depth)
        && let Some(sort) = pending.pending_sort.take()
    {
        pending.sort_state = Some(sort.finish());
    }
}

fn finish_fragment(state: &mut TableParseState) {
    let Some(capture) = state.fragment_capture.take() else {
        return;
    };
    match capture.kind {
        FragmentKind::AutoFilter(pending) => {
            let PendingAutoFilter {
                range,
                range_is_explicit,
                filter_columns,
                seen_filter_column_ids: _,
                pending_filter_column,
                sort_state,
                pending_sort,
            } = *pending;
            let mut filter_columns = filter_columns;
            if let Some(pending) = pending_filter_column {
                filter_columns.push(pending.column);
            }
            let sort_state = sort_state.or_else(|| pending_sort.map(PendingSortState::finish));
            if let Some(range) = range {
                state.auto_filter = Some(TableAutoFilter::from_xlsx(
                    range,
                    range_is_explicit,
                    filter_columns,
                    sort_state,
                ));
            }
        }
        FragmentKind::SortState(pending) => {
            state.sort_state = Some(pending.finish());
        }
        FragmentKind::Invalid => {}
    }
}

fn parse_sort_state(
    depth: u64,
    attributes: &XmlAttributes,
    state: &mut TableParseState,
) -> Option<PendingSortState> {
    let Some(range) = parse_optional_range(attributes.unqualified("ref")) else {
        state.invalidate(reason::INVALID_SORT_STATE_REF);
        return None;
    };
    let case_sensitive = parse_optional_bool(attributes.unqualified("caseSensitive"), false, state);
    let column_sort = parse_optional_bool(attributes.unqualified("columnSort"), false, state);
    Some(PendingSortState {
        depth,
        range,
        case_sensitive,
        column_sort,
        sort_method: parse_optional_token(
            attributes,
            "sortMethod",
            TableSortMethod::from_xlsx,
            state,
        ),
        conditions: Vec::new(),
    })
}

fn push_sort_condition(
    pending: &mut PendingSortState,
    attributes: &XmlAttributes,
    state: &mut TableParseState,
) {
    let Some(range) = parse_optional_range(attributes.unqualified("ref")) else {
        state.invalidate(reason::INVALID_SORT_STATE_REF);
        return;
    };
    if pending.conditions.len() >= MAX_SORT_CONDITIONS {
        state.invalidate(reason::TOO_MANY_SORT_CONDITIONS);
        return;
    }
    let differential_format_id = match parse_optional_u32(attributes, "dxfId", state) {
        Some(value) => Some(value),
        None if attributes.unqualified("dxfId").is_none() => None,
        None => return,
    };
    let icon_id = match parse_optional_u32(attributes, "iconId", state) {
        Some(value) => Some(value),
        None if attributes.unqualified("iconId").is_none() => None,
        None => return,
    };
    pending.conditions.push(TableSortCondition::from_xlsx(
        range,
        parse_optional_bool(attributes.unqualified("descending"), false, state),
        parse_optional_token(attributes, "sortBy", TableSortBy::from_xlsx, state),
        attributes.unqualified("customList").map(str::to_owned),
        differential_format_id,
        parse_optional_token(attributes, "iconSet", TableIconSet::from_xlsx, state),
        icon_id,
    ));
}

fn finish_formula(
    depth: u64,
    local_name: &[u8],
    state: &mut TableParseState,
    total_formula_bytes: &mut u64,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let Some(capture) = state.formula_capture.take() else {
        return Ok(());
    };
    let expected = match capture.kind {
        FormulaKind::CalculatedColumn => CALCULATED_COLUMN_FORMULA,
        FormulaKind::TotalsRow => TOTALS_ROW_FORMULA,
    };
    if capture.depth != depth || local_name != expected {
        state.formula_capture = Some(capture);
        return Ok(());
    }
    *total_formula_bytes = total_formula_bytes.saturating_add(capture.bytes_seen);
    if *total_formula_bytes > budget.limits().max_total_formula_bytes() {
        return Err(budget.error(XlsxErrorCode::TotalFormulaBytesTooLarge));
    }
    let Some(column_index) = capture.column_index else {
        return Ok(());
    };
    let formula = match crate::FormulaText::from_xlsx(capture.text) {
        Ok(text) => TableFormula::new(text, capture.array),
        Err(error) => {
            state.invalidate(error.to_string());
            return Ok(());
        }
    };
    let Some(column) = state.columns.get_mut(column_index) else {
        state.invalidate(reason::MISSING_COLUMN_ID);
        return Ok(());
    };
    let target = match capture.kind {
        FormulaKind::CalculatedColumn => &mut column.calculated_column_formula,
        FormulaKind::TotalsRow => &mut column.totals_row_formula,
    };
    if target.replace(formula).is_some() {
        state.invalidate(reason::DUPLICATE_COLUMN_FORMULA);
    }
    Ok(())
}

fn charge_formula_text(
    capture: &mut FormulaCapture,
    value: &str,
    append: bool,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    capture.bytes_seen = capture.bytes_seen.saturating_add(value.len() as u64);
    if capture.bytes_seen > budget.limits().max_formula_bytes() {
        return Err(budget.error(XlsxErrorCode::FormulaTooLarge));
    }
    if append {
        capture.text.push_str(value);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharacterDataKind {
    Text,
    CData,
    Reference,
}

fn process_character_data(
    value: String,
    kind: CharacterDataKind,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if budget.current_depth() == 0 {
        return if kind == CharacterDataKind::Text && is_xml_whitespace(&value) {
            Ok(())
        } else {
            Err(budget.error(XlsxErrorCode::InvalidXml))
        };
    }
    if state.fragment_capture.is_none()
        && let Some(capture) = &mut state.formula_capture
        && budget.current_depth() >= capture.depth
    {
        return charge_formula_text(
            capture,
            &value,
            budget.current_depth() == capture.depth,
            budget,
        );
    }
    if is_xml_whitespace(&value) {
        return Ok(());
    }
    state.mark_opaque();
    Ok(())
}

fn is_xml_whitespace(value: &str) -> bool {
    value
        .chars()
        .all(|character| matches!(character, '\u{20}' | '\t' | '\r' | '\n'))
}

#[derive(Clone, Copy)]
struct ElementEvent<'a> {
    is_spreadsheet: bool,
    is_strict_spreadsheet: bool,
    local_name: &'a [u8],
    depth: u64,
    has_children: bool,
    attributes: &'a XmlAttributes,
}

fn validate_element_sequence(event: ElementEvent<'_>, state: &mut TableParseState) {
    if !event.is_spreadsheet {
        return;
    }
    if event.depth == 2 {
        let phase = match event.local_name {
            AUTO_FILTER => 1,
            SORT_STATE => 2,
            TABLE_COLUMNS => 3,
            TABLE_STYLE_INFO => 4,
            EXTENSION_LIST => 5,
            _ => return,
        };
        if phase < state.root_child_phase {
            state.invalidate(reason::INVALID_CHILD_ORDER);
        } else {
            state.root_child_phase = phase;
        }
        return;
    }
    let Some((column_depth, _)) = state.table_column_depth else {
        return;
    };
    if event.depth != column_depth + 1 {
        return;
    }
    let phase = match event.local_name {
        CALCULATED_COLUMN_FORMULA => 1,
        TOTALS_ROW_FORMULA => 2,
        XML_COLUMN_PROPERTIES => 3,
        EXTENSION_LIST => 4,
        _ => return,
    };
    if phase < state.table_column_child_phase {
        state.invalidate(reason::INVALID_CHILD_ORDER);
    } else {
        state.table_column_child_phase = phase;
    }
}

fn process_element(
    event: ElementEvent<'_>,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let ElementEvent {
        is_spreadsheet,
        is_strict_spreadsheet,
        local_name,
        depth,
        has_children,
        attributes,
    } = event;
    if depth == 1 {
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidXml));
        }
        state.saw_root = true;
        if !is_spreadsheet || local_name != TABLE {
            state.invalidate(reason::ROOT_NOT_TABLE);
            return Ok(());
        }
        state.strict_spreadsheet = Some(is_strict_spreadsheet);
        process_root_attributes(attributes, limits, state, budget)?;
        return Ok(());
    }
    if !is_spreadsheet {
        state.mark_opaque();
        return Ok(());
    }
    if depth == 2 && local_name == TABLE_COLUMNS {
        mark_unknown_attributes(attributes, &["count"], state);
        if state.saw_table_columns {
            state.invalidate(reason::DUPLICATE_TABLE_COLUMNS);
        } else {
            state.saw_table_columns = true;
            if let Some(value) = attributes.unqualified("count") {
                match value.parse::<u32>() {
                    Ok(count) => state.declared_column_count = Some(count),
                    Err(_) => state.invalidate(reason::INVALID_TABLE_COLUMNS_COUNT),
                }
            }
        }
        // A self-closing <tableColumns/> has no children, so it must not leave the depth
        // marker armed for unrelated elements that happen to sit at the same depth later.
        if has_children {
            state.table_columns_depth = Some(depth);
        }
    } else if depth == 2 && local_name == TABLE_STYLE_INFO {
        if state.saw_table_style {
            state.invalidate(reason::DUPLICATE_TABLE_STYLE);
        } else {
            state.saw_table_style = true;
            state.style_info = Some(parse_table_style(attributes, state));
        }
        if has_children {
            state.mark_opaque();
        }
    } else if depth == 2 && local_name == EXTENSION_LIST {
        state.mark_opaque();
    } else if state
        .table_columns_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == TABLE_COLUMN
    {
        process_column(attributes, limits, state, budget, depth, has_children)?;
    } else if let Some((column_depth, column_index)) = state.table_column_depth
        && depth == column_depth + 1
        && matches!(local_name, CALCULATED_COLUMN_FORMULA | TOTALS_ROW_FORMULA)
    {
        start_formula_capture(
            local_name,
            attributes,
            depth,
            column_index,
            has_children,
            state,
        );
    } else if state.table_column_depth.is_some()
        || state.table_columns_depth.is_some()
        || depth == 2
    {
        state.mark_opaque();
    }
    Ok(())
}

fn process_root_attributes(
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    match attributes
        .unqualified("id")
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(|value| TableId::new(value).ok())
    {
        Some(id) => state.id = Some(id),
        None => state.invalidate(reason::MISSING_TABLE_ID),
    }
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
    match attributes.unqualified("tableType") {
        None => state.table_type = TableType::Worksheet,
        Some(value) => match TableType::from_xlsx(value) {
            Some(table_type) => state.table_type = table_type,
            None => state.invalidate(reason::INVALID_TABLE_TYPE),
        },
    }
    state.totals_row_shown =
        parse_optional_bool(attributes.unqualified("totalsRowShown"), true, state);
    mark_unknown_attributes(
        attributes,
        &[
            "id",
            "name",
            "displayName",
            "ref",
            "tableType",
            "headerRowCount",
            "totalsRowCount",
            "totalsRowShown",
        ],
        state,
    );
    Ok(())
}

fn process_column(
    attributes: &XmlAttributes,
    limits: ReadLimits,
    state: &mut TableParseState,
    budget: &XmlBudget,
    depth: u64,
    has_children: bool,
) -> Result<(), XlsxReadError> {
    if has_children {
        state.table_column_child_phase = 0;
    }
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
        if has_children {
            state.table_column_depth = Some((depth, None));
        }
        return Ok(());
    }
    let Some(id) = attributes
        .unqualified("id")
        .and_then(|value| value.parse::<u32>().ok())
    else {
        state.invalidate(reason::MISSING_COLUMN_ID);
        if has_children {
            state.table_column_depth = Some((depth, None));
        }
        return Ok(());
    };
    let Some(name) = attributes.unqualified("name") else {
        state.invalidate(reason::MISSING_COLUMN_NAME);
        if has_children {
            state.table_column_depth = Some((depth, None));
        }
        return Ok(());
    };
    let totals_row_label = attributes.unqualified("totalsRowLabel").map(str::to_owned);
    let totals_row_function = match attributes.unqualified("totalsRowFunction") {
        None | Some("none") => None,
        Some(token) => match parse_totals_row_function(token) {
            Some(function) => Some(function),
            None => {
                state.invalidate(reason::INVALID_TOTALS_FUNCTION);
                if has_children {
                    state.table_column_depth = Some((depth, None));
                }
                return Ok(());
            }
        },
    };
    mark_unknown_attributes(
        attributes,
        &["id", "name", "totalsRowFunction", "totalsRowLabel"],
        state,
    );
    match TableColumn::from_xlsx(id, name, totals_row_function) {
        Ok(column) => {
            let column_index = state.columns.len();
            state.columns.push(ParsedColumn {
                column,
                totals_row_label,
                calculated_column_formula: None,
                totals_row_formula: None,
            });
            if has_children {
                state.table_column_depth = Some((depth, Some(column_index)));
            }
        }
        Err(error) => {
            state.invalidate(error.to_string());
            if has_children {
                state.table_column_depth = Some((depth, None));
            }
        }
    }
    Ok(())
}

fn start_formula_capture(
    local_name: &[u8],
    attributes: &XmlAttributes,
    depth: u64,
    column_index: Option<usize>,
    has_children: bool,
    state: &mut TableParseState,
) {
    if state.formula_capture.is_some() {
        state.invalidate(reason::DUPLICATE_COLUMN_FORMULA);
        return;
    }
    mark_unknown_attributes(attributes, &["array"], state);
    let array = parse_optional_bool(attributes.unqualified("array"), false, state);
    let kind = if local_name == CALCULATED_COLUMN_FORMULA {
        FormulaKind::CalculatedColumn
    } else {
        FormulaKind::TotalsRow
    };
    if !has_children {
        state.invalidate(crate::ValidationError::FormulaEmpty.to_string());
        return;
    }
    state.formula_capture = Some(FormulaCapture {
        depth,
        column_index,
        kind,
        array,
        text: String::new(),
        bytes_seen: 0,
    });
}

fn parse_table_style(attributes: &XmlAttributes, state: &mut TableParseState) -> TableStyleInfo {
    mark_unknown_attributes(
        attributes,
        &[
            "name",
            "showFirstColumn",
            "showLastColumn",
            "showRowStripes",
            "showColumnStripes",
        ],
        state,
    );
    TableStyleInfo::new(
        attributes.unqualified("name").map(str::to_owned),
        parse_optional_bool(attributes.unqualified("showFirstColumn"), false, state),
        parse_optional_bool(attributes.unqualified("showLastColumn"), false, state),
        parse_optional_bool(attributes.unqualified("showRowStripes"), false, state),
        parse_optional_bool(attributes.unqualified("showColumnStripes"), false, state),
    )
}

fn mark_unknown_attributes(
    attributes: &XmlAttributes,
    known_unqualified: &[&str],
    state: &mut TableParseState,
) {
    if attributes
        .iter()
        .any(|(name, namespace, _)| namespace.is_some() || !known_unqualified.contains(&name))
    {
        state.mark_opaque();
    }
}

fn parse_optional_bool(value: Option<&str>, default: bool, state: &mut TableParseState) -> bool {
    match value {
        None => default,
        Some("0" | "false") => false,
        Some("1" | "true") => true,
        Some(_) => {
            state.invalidate(reason::INVALID_BOOLEAN);
            default
        }
    }
}

fn parse_required_token<T>(
    attributes: &XmlAttributes,
    name: &str,
    parser: fn(&str) -> Option<T>,
    state: &mut TableParseState,
) -> Option<T> {
    match attributes.unqualified(name).and_then(parser) {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_token<T>(
    attributes: &XmlAttributes,
    name: &str,
    parser: fn(&str) -> Option<T>,
    state: &mut TableParseState,
) -> Option<T> {
    let value = attributes.unqualified(name)?;
    match parser(value) {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_required_numeric(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<TableNumericValue> {
    match attributes
        .unqualified(name)
        .map(str::to_owned)
        .and_then(TableNumericValue::from_xlsx)
    {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_numeric(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<TableNumericValue> {
    let value = attributes.unqualified(name)?.to_owned();
    match TableNumericValue::from_xlsx(value) {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_date_time(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<TableDateTimeValue> {
    let value = attributes.unqualified(name)?.to_owned();
    match TableDateTimeValue::from_xlsx(value) {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_required_u16(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<u16> {
    match attributes
        .unqualified(name)
        .and_then(|value| value.parse::<u16>().ok())
    {
        Some(value) => Some(value),
        None => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_u16(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<u16> {
    let value = attributes.unqualified(name)?;
    match value.parse::<u16>() {
        Ok(value) => Some(value),
        Err(_) => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_u32(
    attributes: &XmlAttributes,
    name: &str,
    state: &mut TableParseState,
) -> Option<u32> {
    let value = attributes.unqualified(name)?;
    match value.parse::<u32>() {
        Ok(value) => Some(value),
        Err(_) => {
            state.invalidate(reason::INVALID_FILTER_CRITERIA);
            None
        }
    }
}

fn parse_optional_range(value: Option<&str>) -> Option<CellRange> {
    parse_reference(value?)
}

fn fragment_attributes(local_name: &[u8]) -> Option<&'static [&'static str]> {
    match local_name {
        FILTER_COLUMN => Some(&["colId", "hiddenButton", "showButton"]),
        FILTERS => Some(&["blank", "calendarType"]),
        FILTER => Some(&["val"]),
        DATE_GROUP_ITEM => Some(&[
            "year",
            "month",
            "day",
            "hour",
            "minute",
            "second",
            "dateTimeGrouping",
        ]),
        CUSTOM_FILTERS => Some(&["and"]),
        CUSTOM_FILTER => Some(&["operator", "val"]),
        DYNAMIC_FILTER => Some(&["type", "val", "valIso", "maxVal", "maxValIso"]),
        COLOR_FILTER => Some(&["dxfId", "cellColor"]),
        ICON_FILTER => Some(&["iconSet", "iconId"]),
        TOP_TEN => Some(&["top", "percent", "val", "filterVal"]),
        SORT_STATE => Some(&["ref", "caseSensitive", "columnSort", "sortMethod"]),
        SORT_CONDITION => Some(&[
            "descending",
            "sortBy",
            "ref",
            "customList",
            "dxfId",
            "iconSet",
            "iconId",
        ]),
        EXTENSION_LIST => Some(&[]),
        _ => None,
    }
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

fn inherited_auto_filter_range(state: &TableParseState) -> Option<CellRange> {
    let table_range = state.reference?;
    let end_row = table_range
        .end()
        .row()
        .get()
        .checked_sub(state.totals_row_count)?;
    if end_row < table_range.start().row().get() {
        return None;
    }
    let end = CellAddress::from_indices(end_row, table_range.end().column().get()).ok()?;
    CellRange::new(table_range.start(), end).ok()
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
    let code = DiagnosticCode::new(code)
        .map_err(|error| budget.error(XlsxErrorCode::InvalidXml).with_cause(error))?;
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
