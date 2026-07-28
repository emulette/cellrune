use quick_xml::events::Event;

use super::super::error::compatibility;
use super::super::package::PartPath;
use super::super::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, XmlAttributes, XmlBudget,
    decode_cdata, decode_reference, decode_text, is_spreadsheet_element, read_attributes, reader,
    require_spreadsheet_element,
};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use super::PresentationCapture;
use super::formula_cell::SharedFormulaTable;
use super::merge::MergedRangeCollector;
use super::metadata::CellMetadata;
use super::phonetic::{PhoneticReadBudget, parse_bool, parse_properties};
use super::shared_strings::SharedStrings;
use super::styles::Styles;
use super::worksheet_cell::{CellBuilder, CellFinishContext};
use crate::{
    CellAddress, Column, ColumnPhoneticVisibility, Diagnostic, DiagnosticCode, DiagnosticSeverity,
    DocumentPresentation, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, FrozenPane, Row, Sheet, SheetId,
    SourceLocation,
};

const WORKSHEET: &[u8] = b"worksheet";
const COLUMNS: &[u8] = b"cols";
const COLUMN: &[u8] = b"col";
const SHEET_VIEWS: &[u8] = b"sheetViews";
const SHEET_VIEW: &[u8] = b"sheetView";
const PANE: &[u8] = b"pane";
const SHEET_DATA: &[u8] = b"sheetData";
const ROW: &[u8] = b"row";
const CELL: &[u8] = b"c";
const PHONETIC_PROPERTIES: &[u8] = b"phoneticPr";
const MERGE_CELLS: &[u8] = b"mergeCells";
const MERGE_CELL: &[u8] = b"mergeCell";
const TABLE_PARTS: &[u8] = b"tableParts";
const TABLE_PART: &[u8] = b"tablePart";
#[derive(Debug, Default)]
struct WorksheetParseState {
    saw_root: bool,
    saw_sheet_data: bool,
    sheet_views_depth: Option<u64>,
    sheet_view_depth: Option<u64>,
    default_view_active: bool,
    saw_default_view: bool,
    saw_default_pane: bool,
    columns_depth: Option<u64>,
    saw_worksheet_phonetic_properties: bool,
    sheet_data_depth: Option<u64>,
    row_depth: Option<u64>,
    row_number: Option<u32>,
    current_cell: Option<CellBuilder>,
    sheet_cells: u64,
    shared_formulas: SharedFormulaTable,
    merge_cells_depth: Option<u64>,
    saw_merge_cells: bool,
    merged_ranges: MergedRangeCollector,
    table_parts_depth: Option<u64>,
    saw_table_parts: bool,
}

#[derive(Clone, Copy)]
pub(super) struct WorksheetResources<'a> {
    pub(super) shared_strings: Option<&'a SharedStrings>,
    pub(super) styles: &'a Styles,
    pub(super) cell_metadata: Option<&'a CellMetadata>,
}

pub(super) struct WorksheetOutput<'a> {
    pub(super) sheet: &'a mut Sheet,
    pub(super) total_cells: &'a mut u64,
    pub(super) total_formula_bytes: &'a mut u64,
    pub(super) total_merged_ranges: &'a mut u64,
    pub(super) total_tables: &'a mut u64,
    pub(super) presentation: &'a mut DocumentPresentation,
    pub(super) phonetic_budget: &'a mut PhoneticReadBudget,
    pub(super) diagnostics: &'a mut Vec<Diagnostic>,
    pub(super) table_relationship_ids: &'a mut Vec<Box<str>>,
}

struct WorksheetStartContext<'budget, 'state> {
    budget: &'budget XmlBudget,
    state: &'state mut WorksheetParseState,
    total_cells: &'state mut u64,
    total_merged_ranges: &'state mut u64,
    total_tables: &'state mut u64,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &'state mut DocumentPresentation,
    diagnostics: &'state mut Vec<Diagnostic>,
    table_relationship_ids: &'state mut Vec<Box<str>>,
    font_count: u32,
}

pub(super) fn parse(
    bytes: &[u8],
    source: &PartPath,
    limits: ReadLimits,
    resources: WorksheetResources<'_>,
    capture: PresentationCapture,
    output: WorksheetOutput<'_>,
) -> Result<(), XlsxReadError> {
    let WorksheetOutput {
        sheet,
        total_cells,
        total_formula_bytes,
        total_merged_ranges,
        total_tables,
        presentation,
        phonetic_budget,
        diagnostics,
        table_relationship_ids,
    } = output;
    let sheet_id = sheet.id();
    let mut xml = reader(bytes);
    let mut buffer = Vec::new();
    let mut budget = XmlBudget::new(limits, source.source_id(), XlsxErrorCode::InvalidWorksheet);
    let mut state = WorksheetParseState::default();

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidWorksheet)
                .with_cause(error)
        })?;
        match event {
            Event::Start(element) => {
                let depth = budget.start()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                process_start(
                    is_spreadsheet,
                    &local_name,
                    depth,
                    attributes,
                    WorksheetStartContext {
                        budget: &budget,
                        state: &mut state,
                        total_cells: &mut *total_cells,
                        total_merged_ranges: &mut *total_merged_ranges,
                        total_tables: &mut *total_tables,
                        capture,
                        sheet_id,
                        presentation: &mut *presentation,
                        diagnostics: &mut *diagnostics,
                        table_relationship_ids: &mut *table_relationship_ids,
                        font_count: resources.styles.font_count(),
                    },
                )?;
            }
            Event::Empty(element) => {
                let depth = budget.empty()?;
                let local_name = element.local_name().as_ref().to_vec();
                let attributes = read_attributes(&element, &xml, &budget)?;
                let is_spreadsheet = is_spreadsheet_element(&xml, element.name(), &budget)?;
                if depth == 1 {
                    require_spreadsheet_element(is_spreadsheet, &local_name, WORKSHEET, &budget)?;
                    if state.saw_root {
                        return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
                    }
                    state.saw_root = true;
                } else if is_spreadsheet {
                    if state
                        .sheet_views_depth
                        .is_some_and(|parent| depth == parent + 1)
                        && local_name == SHEET_VIEW
                    {
                        process_sheet_view(
                            &attributes,
                            capture,
                            sheet_id,
                            presentation,
                            &budget,
                            &mut state,
                        )?;
                        state.sheet_view_depth = None;
                        state.default_view_active = false;
                    } else if state.default_view_active
                        && state
                            .sheet_view_depth
                            .is_some_and(|parent| depth == parent + 1)
                        && local_name == PANE
                    {
                        process_pane(
                            &attributes,
                            capture,
                            sheet_id,
                            presentation,
                            &budget,
                            &mut state,
                        )?;
                    } else if depth == 2 && local_name == PHONETIC_PROPERTIES {
                        process_worksheet_phonetic_properties(
                            &attributes,
                            capture,
                            sheet_id,
                            presentation,
                            resources.styles.font_count(),
                            &budget,
                            &mut state,
                        )?;
                    } else if state
                        .columns_depth
                        .is_some_and(|parent| depth == parent + 1)
                        && local_name == COLUMN
                    {
                        process_column_visibility(
                            &attributes,
                            capture,
                            sheet_id,
                            presentation,
                            &budget,
                        )?;
                    } else if depth == 2 && local_name == SHEET_DATA {
                        // An empty sheet is serialized by Excel and every mainstream producer
                        // as a self-closing <sheetData/>, which arrives as an Empty event, not
                        // Start; it satisfies the required-sheetData rule all the same.
                        if state.saw_sheet_data {
                            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
                        }
                        state.saw_sheet_data = true;
                    } else if depth == 2 && local_name == MERGE_CELLS {
                        if state.saw_merge_cells {
                            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
                        }
                        state.saw_merge_cells = true;
                    } else if state
                        .merge_cells_depth
                        .is_some_and(|parent| depth == parent + 1)
                        && local_name == MERGE_CELL
                    {
                        state.merged_ranges.record(
                            attributes.unqualified("ref"),
                            total_merged_ranges,
                            sheet_id,
                            diagnostics,
                            &budget,
                        )?;
                    } else if depth == 2 && local_name == TABLE_PARTS {
                        if state.saw_table_parts {
                            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
                        }
                        state.saw_table_parts = true;
                    } else if state
                        .table_parts_depth
                        .is_some_and(|parent| depth == parent + 1)
                        && local_name == TABLE_PART
                    {
                        record_table_part(
                            &attributes,
                            table_relationship_ids,
                            total_tables,
                            sheet_id,
                            diagnostics,
                            &budget,
                        )?;
                    } else if state
                        .sheet_data_depth
                        .is_some_and(|parent| depth == parent + 1)
                        && local_name == ROW
                    {
                        process_empty_row_visibility(
                            &attributes,
                            capture,
                            sheet.id(),
                            presentation,
                            &budget,
                        )?;
                    } else if local_name == CELL
                        && state.row_depth.is_some_and(|row| depth == row + 1)
                    {
                        increment_cell_counts(
                            &mut state.sheet_cells,
                            total_cells,
                            limits,
                            &budget,
                        )?;
                        let builder = CellBuilder::begin(
                            attributes,
                            depth,
                            state.row_number,
                            capture,
                            resources.styles.font_count(),
                            &budget,
                        )?;
                        builder.finish(CellFinishContext {
                            resources,
                            shared_formulas: &mut state.shared_formulas,
                            total_formula_bytes: &mut *total_formula_bytes,
                            sheet: &mut *sheet,
                            presentation: &mut *presentation,
                            phonetic_budget: &mut *phonetic_budget,
                            budget: &budget,
                        })?;
                    } else if let Some(cell) = &mut state.current_cell {
                        cell.process_empty(&local_name, depth, attributes, &budget)?;
                    }
                }
            }
            Event::Text(text) => {
                if let Some(cell) = &mut state.current_cell {
                    cell.append(decode_text(&text, &budget)?, &budget)?;
                }
            }
            Event::CData(text) => {
                if let Some(cell) = &mut state.current_cell {
                    cell.append(decode_cdata(&text, &budget)?, &budget)?;
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(cell) = &mut state.current_cell {
                    cell.append(decode_reference(&reference, &budget)?, &budget)?;
                }
            }
            Event::End(element) => {
                let depth = budget.end()?;
                let local_name = element.local_name().as_ref().to_vec();
                if let Some(cell) = &mut state.current_cell {
                    cell.process_end(&local_name, depth, &budget)?;
                }
                if state
                    .current_cell
                    .as_ref()
                    .is_some_and(|cell| cell.depth() == depth && local_name == CELL)
                {
                    let builder = state
                        .current_cell
                        .take()
                        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidWorksheet))?;
                    builder.finish(CellFinishContext {
                        resources,
                        shared_formulas: &mut state.shared_formulas,
                        total_formula_bytes: &mut *total_formula_bytes,
                        sheet: &mut *sheet,
                        presentation: &mut *presentation,
                        phonetic_budget: &mut *phonetic_budget,
                        budget: &budget,
                    })?;
                } else if state.row_depth == Some(depth) && local_name == ROW {
                    state.row_depth = None;
                    state.row_number = None;
                } else if state.sheet_data_depth == Some(depth) && local_name == SHEET_DATA {
                    state.sheet_data_depth = None;
                } else if state.merge_cells_depth == Some(depth) && local_name == MERGE_CELLS {
                    state.merge_cells_depth = None;
                } else if state.table_parts_depth == Some(depth) && local_name == TABLE_PARTS {
                    state.table_parts_depth = None;
                } else if state.sheet_view_depth == Some(depth) && local_name == SHEET_VIEW {
                    state.sheet_view_depth = None;
                    state.default_view_active = false;
                } else if state.sheet_views_depth == Some(depth) && local_name == SHEET_VIEWS {
                    state.sheet_views_depth = None;
                } else if state.columns_depth == Some(depth) && local_name == COLUMNS {
                    state.columns_depth = None;
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
    if !state.saw_sheet_data
        || state.current_cell.is_some()
        || state.row_depth.is_some()
        || state.sheet_data_depth.is_some()
        || state.merge_cells_depth.is_some()
        || state.table_parts_depth.is_some()
        || state.sheet_view_depth.is_some()
        || state.sheet_views_depth.is_some()
        || state.columns_depth.is_some()
    {
        return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
    }
    sheet.set_merged_ranges(state.merged_ranges.finish(sheet_id, diagnostics, &budget)?);
    Ok(())
}

fn process_start(
    is_spreadsheet: bool,
    local_name: &[u8],
    depth: u64,
    attributes: XmlAttributes,
    context: WorksheetStartContext<'_, '_>,
) -> Result<(), XlsxReadError> {
    let WorksheetStartContext {
        budget,
        state,
        total_cells,
        total_merged_ranges,
        total_tables,
        capture,
        sheet_id,
        presentation,
        diagnostics,
        table_relationship_ids,
        font_count,
    } = context;
    if depth == 1 {
        require_spreadsheet_element(is_spreadsheet, local_name, WORKSHEET, budget)?;
        if state.saw_root {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.saw_root = true;
        return Ok(());
    }
    if !is_spreadsheet {
        return Ok(());
    }
    if depth == 2 && local_name == COLUMNS {
        if state.columns_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
    } else if state
        .columns_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == COLUMN
    {
        process_column_visibility(&attributes, capture, sheet_id, presentation, budget)?;
    } else if depth == 2 && local_name == PHONETIC_PROPERTIES {
        process_worksheet_phonetic_properties(
            &attributes,
            capture,
            sheet_id,
            presentation,
            font_count,
            budget,
            state,
        )?;
    } else if depth == 2 && local_name == SHEET_VIEWS {
        if state.sheet_views_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
        }
    } else if state
        .sheet_views_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == SHEET_VIEW
    {
        if state.sheet_view_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
        }
        process_sheet_view(&attributes, capture, sheet_id, presentation, budget, state)?;
    } else if state.default_view_active
        && state
            .sheet_view_depth
            .is_some_and(|parent| depth == parent + 1)
        && local_name == PANE
    {
        process_pane(&attributes, capture, sheet_id, presentation, budget, state)?;
    } else if depth == 2 && local_name == SHEET_DATA {
        if state.saw_sheet_data || state.sheet_data_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.saw_sheet_data = true;
    } else if depth == 2 && local_name == MERGE_CELLS {
        if state.saw_merge_cells || state.merge_cells_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.saw_merge_cells = true;
    } else if state
        .merge_cells_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == MERGE_CELL
    {
        state.merged_ranges.record(
            attributes.unqualified("ref"),
            total_merged_ranges,
            sheet_id,
            diagnostics,
            budget,
        )?;
    } else if depth == 2 && local_name == TABLE_PARTS {
        if state.saw_table_parts || state.table_parts_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.saw_table_parts = true;
    } else if state
        .table_parts_depth
        .is_some_and(|parent| depth == parent + 1)
        && local_name == TABLE_PART
    {
        record_table_part(
            &attributes,
            table_relationship_ids,
            total_tables,
            sheet_id,
            diagnostics,
            budget,
        )?;
    } else if state
        .sheet_data_depth
        .is_some_and(|sheet_data| depth == sheet_data + 1)
        && local_name == ROW
    {
        if state.row_depth.replace(depth).is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.row_number = attributes
            .unqualified("r")
            .map(|value| {
                value.parse::<u32>().map_err(|error| {
                    budget
                        .error(XlsxErrorCode::InvalidWorksheet)
                        .with_cause(error)
                })
            })
            .transpose()?;
        if state
            .row_number
            .is_some_and(|row| !(1..=EXCEL_MAX_ROWS).contains(&row))
        {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        if capture == PresentationCapture::Document
            && let Some(value) = attributes.unqualified("ph")
        {
            let row = state
                .row_number
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?;
            presentation.source_row_phonetic_visibility(
                sheet_id,
                Row::new(row).map_err(|error| {
                    budget
                        .error(XlsxErrorCode::InvalidPhoneticMetadata)
                        .with_cause(error)
                })?,
                parse_bool(value, budget)?,
            );
        }
    } else if state.row_depth.is_some_and(|row| depth == row + 1) && local_name == CELL {
        increment_cell_counts(&mut state.sheet_cells, total_cells, budget.limits(), budget)?;
        if state.current_cell.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidWorksheet));
        }
        state.current_cell = Some(CellBuilder::begin(
            attributes,
            depth,
            state.row_number,
            capture,
            font_count,
            budget,
        )?);
    } else if let Some(cell) = &mut state.current_cell {
        cell.process_start(local_name, depth, attributes, budget)?;
    }
    Ok(())
}

fn process_sheet_view(
    attributes: &XmlAttributes,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &mut DocumentPresentation,
    budget: &XmlBudget,
    state: &mut WorksheetParseState,
) -> Result<(), XlsxReadError> {
    if capture == PresentationCapture::None {
        state.default_view_active = false;
        return Ok(());
    }
    let view_id = attributes
        .unqualified("workbookViewId")
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidFrozenPane))?
        .parse::<u32>()
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidFrozenPane)
                .with_cause(error)
        })?;
    state.default_view_active = view_id == 0;
    if !state.default_view_active {
        return Ok(());
    }
    if std::mem::replace(&mut state.saw_default_view, true) {
        return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
    }
    let right_to_left = optional_bool(attributes.unqualified("rightToLeft"), budget)?;
    presentation.source_right_to_left(sheet_id, right_to_left.unwrap_or(false));
    Ok(())
}

fn process_worksheet_phonetic_properties(
    attributes: &XmlAttributes,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &mut DocumentPresentation,
    font_count: u32,
    budget: &XmlBudget,
    state: &mut WorksheetParseState,
) -> Result<(), XlsxReadError> {
    if capture == PresentationCapture::None {
        return Ok(());
    }
    if std::mem::replace(&mut state.saw_worksheet_phonetic_properties, true) {
        return Err(budget.error(XlsxErrorCode::InvalidPhoneticMetadata));
    }
    presentation.source_worksheet_phonetic_properties(
        sheet_id,
        parse_properties(attributes, font_count, budget)?,
    );
    Ok(())
}

fn process_column_visibility(
    attributes: &XmlAttributes,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &mut DocumentPresentation,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if capture == PresentationCapture::None {
        return Ok(());
    }
    let Some(value) = attributes.unqualified("phonetic") else {
        return Ok(());
    };
    let first = required_column(attributes.unqualified("min"), budget)?;
    let last = required_column(attributes.unqualified("max"), budget)?;
    let visibility = ColumnPhoneticVisibility::new(first, last, parse_bool(value, budget)?)
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?;
    presentation.source_column_phonetic_visibility(sheet_id, visibility);
    Ok(())
}

fn process_empty_row_visibility(
    attributes: &XmlAttributes,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &mut DocumentPresentation,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if capture == PresentationCapture::None {
        return Ok(());
    }
    let Some(value) = attributes.unqualified("ph") else {
        return Ok(());
    };
    let row = attributes
        .unqualified("r")
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
        .parse::<u32>()
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?;
    presentation.source_row_phonetic_visibility(
        sheet_id,
        Row::new(row).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?,
        parse_bool(value, budget)?,
    );
    Ok(())
}

fn process_pane(
    attributes: &XmlAttributes,
    capture: PresentationCapture,
    sheet_id: SheetId,
    presentation: &mut DocumentPresentation,
    budget: &XmlBudget,
    state: &mut WorksheetParseState,
) -> Result<(), XlsxReadError> {
    if capture == PresentationCapture::None {
        return Ok(());
    }
    if std::mem::replace(&mut state.saw_default_pane, true) {
        return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
    }
    if attributes.unqualified("state").unwrap_or("split") != "frozen" {
        push_presentation_diagnostic(
            presentation,
            sheet_id,
            budget,
            super::super::error::compatibility::PRESERVED_PANE_CODE,
            super::super::error::compatibility::PRESERVED_PANE_MESSAGE,
        )?;
        return Ok(());
    }
    let columns = frozen_count(attributes.unqualified("xSplit"), budget)?;
    let rows = frozen_count(attributes.unqualified("ySplit"), budget)?;
    if rows == 0 && columns == 0 {
        return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
    }
    let pane = FrozenPane::new(rows, columns).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFrozenPane)
            .with_cause(error)
    })?;
    if let Some(top_left) = attributes.unqualified("topLeftCell") {
        let top_left = CellAddress::from_a1(top_left).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidFrozenPane)
                .with_cause(error)
        })?;
        if top_left.row().get() <= rows || top_left.column().get() <= columns {
            return Err(budget
                .error(XlsxErrorCode::InvalidFrozenPane)
                .with_detail(top_left.to_string()));
        }
    }
    let expected_active = match (rows > 0, columns > 0) {
        (true, true) => "bottomRight",
        (true, false) => "bottomLeft",
        (false, true) => "topRight",
        (false, false) => unreachable!("zero pane rejected"),
    };
    if attributes
        .unqualified("activePane")
        .is_some_and(|value| value != expected_active)
    {
        return Err(budget.error(XlsxErrorCode::InvalidFrozenPane));
    }
    presentation.source_frozen_pane(sheet_id, pane);
    Ok(())
}

fn record_table_part(
    attributes: &XmlAttributes,
    table_relationship_ids: &mut Vec<Box<str>>,
    total_tables: &mut u64,
    sheet_id: SheetId,
    diagnostics: &mut Vec<Diagnostic>,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    *total_tables = total_tables.saturating_add(1);
    if *total_tables > budget.limits().max_tables() {
        return Err(budget.error(XlsxErrorCode::TooManyTables));
    }
    let relationship_id = attributes
        .namespaced(DOCUMENT_RELATIONSHIPS_TRANSITIONAL, "id")
        .or_else(|| attributes.namespaced(DOCUMENT_RELATIONSHIPS_STRICT, "id"));
    let Some(relationship_id) = relationship_id else {
        return super::table::push_invalid_diagnostic(
            diagnostics,
            compatibility::TABLE_MISSING_RELATIONSHIP_ID,
            sheet_id,
            budget,
        );
    };
    table_relationship_ids.push(Box::from(relationship_id));
    Ok(())
}

fn required_column(value: Option<&str>, budget: &XmlBudget) -> Result<Column, XlsxReadError> {
    let value = value
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
        .parse::<u32>()
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?;
    if value == 0 || value > EXCEL_MAX_COLUMNS {
        return Err(budget
            .error(XlsxErrorCode::InvalidPhoneticMetadata)
            .with_detail(value.to_string()));
    }
    Column::new(value).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidPhoneticMetadata)
            .with_cause(error)
    })
}

fn push_presentation_diagnostic(
    presentation: &mut DocumentPresentation,
    sheet_id: SheetId,
    budget: &XmlBudget,
    code: &'static str,
    message: &'static str,
) -> Result<(), XlsxReadError> {
    let code = DiagnosticCode::new(code).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorksheet)
            .with_cause(error)
    })?;
    let diagnostic = Diagnostic::new(
        code,
        DiagnosticSeverity::Warning,
        message,
        Some(SourceLocation::sheet(budget.source_id().clone(), sheet_id)),
    )
    .map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorksheet)
            .with_cause(error)
    })?;
    presentation.push_diagnostic(diagnostic);
    Ok(())
}

fn frozen_count(value: Option<&str>, budget: &XmlBudget) -> Result<u32, XlsxReadError> {
    let Some(value) = value else {
        return Ok(0);
    };
    let number = value.parse::<f64>().map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFrozenPane)
            .with_cause(error)
    })?;
    if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 || number > u32::MAX as f64 {
        return Err(budget
            .error(XlsxErrorCode::InvalidFrozenPane)
            .with_detail(value.to_owned()));
    }
    Ok(number as u32)
}

fn optional_bool(value: Option<&str>, budget: &XmlBudget) -> Result<Option<bool>, XlsxReadError> {
    value
        .map(|value| match value {
            "0" | "false" => Ok(false),
            "1" | "true" => Ok(true),
            _ => Err(budget
                .error(XlsxErrorCode::InvalidFrozenPane)
                .with_detail(value.to_owned())),
        })
        .transpose()
}

fn increment_cell_counts(
    sheet_cells: &mut u64,
    total_cells: &mut u64,
    limits: ReadLimits,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    *sheet_cells = sheet_cells.saturating_add(1);
    if *sheet_cells > limits.max_cells_per_sheet() {
        return Err(budget.error(XlsxErrorCode::TooManyCellsInSheet));
    }
    *total_cells = total_cells.saturating_add(1);
    if *total_cells > limits.max_total_cells() {
        return Err(budget.error(XlsxErrorCode::TooManyCells));
    }
    Ok(())
}
