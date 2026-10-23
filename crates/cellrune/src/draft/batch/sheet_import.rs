//! Importing a worksheet from another workbook, as Excel 16.114's Move or Copy into another open
//! workbook does.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::BatchExecutionError;
use super::sheet_copy::{rewrite_copy, rewrite_text};
use super::tab_order::{TabOrder, place_tabs};
use crate::calculation::formula_references::formula_references;
use crate::calculation::formula_rewrite::{FormulaRewriteBudget, FormulaRewriteRequest};
use crate::calculation::sheet_changes::TabEntry;
use crate::presentation::{CellFormat, ColumnWidth, SheetPresentation};
use crate::workbook::{NewCellStyles, NonWorksheetTabs};
use crate::{
    CellAddress, CellContent, DateSystem, DefinedName, DefinedNameScope, FormulaText, NumberFormat,
    Row, Sheet, SheetId, SheetName, TableId, TableName, ValidationError, WorkbookDraft,
    case_insensitive_eq,
};

/// A worksheet read from one workbook for [`crate::WorkbookChange::import_sheet`] into another.
///
/// It holds the sheet's cells, number and cell formats, column widths, row heights, frozen pane,
/// merged ranges, hidden rows, tables, and auto-filter, the names scoped to it, and the workbook
/// names that read it. Two values are equal only when one is a clone of the other.
#[derive(Debug, Clone)]
pub struct ImportedSheet(Arc<ImportedSheetData>);

#[derive(Debug)]
struct ImportedSheetData {
    /// The sheet with its source ID and name; its formats are in `cell_formats`.
    sheet: Sheet,
    date_system: DateSystem,
    cell_formats: Vec<(CellAddress, CellFormat)>,
    column_widths: Vec<ColumnWidth>,
    row_heights: Vec<(Row, f64)>,
    presentation: Option<SheetPresentation>,
    /// The names scoped to the sheet, then the workbook names that read it.
    names: Vec<DefinedName>,
}

impl PartialEq for ImportedSheet {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ImportedSheet {
    /// Reads the worksheet `sheet_id` of `source` for an import.
    ///
    /// The names scoped to the sheet come with it, and so does each workbook name that reads the
    /// sheet and no other sheet, as Excel 16.114 copies them.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] for a missing sheet, or
    /// [`ValidationError::SheetImportUnsupported`] naming the first content the import cannot
    /// carry: a formula that does not parse or that reads another sheet, another workbook, a
    /// table on another sheet, or a name that does; data validations, hyperlinks, comments,
    /// auto-filter criteria, row or column styles, a right-to-left view, or phonetic settings,
    /// which the model does not write for a new sheet; and worksheet content only the source
    /// package carries, such as conditional formats, drawings, or pivot tables. Print and view
    /// settings, such as margins, page setup, headers and footers, and the default row height,
    /// are not carried.
    pub fn from_draft(source: &WorkbookDraft, sheet_id: SheetId) -> Result<Self, ValidationError> {
        let workbook = source.workbook();
        let original = workbook
            .sheet_by_id(sheet_id)
            .ok_or(ValidationError::UnknownSheetId {
                value: sheet_id.get(),
            })?;
        let presentation = source.presentation().sheet(sheet_id).cloned();
        let unwritten = [
            (!original.data_validations().is_empty(), "data validations"),
            (!original.hyperlinks().is_empty(), "hyperlinks"),
            (!original.comments().is_empty(), "comments"),
            (
                original
                    .auto_filter()
                    .is_some_and(|filter| !filter.filter_column_ids().is_empty()),
                "auto-filter criteria",
            ),
            (
                !original
                    .new_cell_styles()
                    .same_line_styles(&NewCellStyles::default()),
                "row or column styles",
            ),
            (
                presentation
                    .as_ref()
                    .is_some_and(|sheet| sheet.right_to_left),
                "a right-to-left view",
            ),
            (
                presentation.as_ref().is_some_and(|sheet| {
                    sheet.worksheet_phonetic_properties.is_some()
                        || !sheet.row_phonetic_visibility.is_empty()
                        || !sheet.column_phonetic_visibility.is_empty()
                }),
                "phonetic settings",
            ),
        ];
        if let Some((_, content)) = unwritten.into_iter().find(|(present, _)| *present) {
            return Err(unsupported(content));
        }
        if let Some(document) = source.source_document()
            && !source.added_sheets().contains(&sheet_id)
        {
            let origin = source
                .copied_sheets()
                .get(&sheet_id)
                .copied()
                .unwrap_or(sheet_id);
            let content = document
                .package_only_sheet_content(origin)
                .map_err(|error| unsupported(&error.to_string()))?;
            if let Some(content) = content.first() {
                return Err(unsupported(content));
            }
        }

        let own_tables = original
            .tables()
            .iter()
            .map(|table| table.display_name().as_str().to_owned())
            .collect::<Vec<_>>();
        let mut used_names = BTreeSet::new();
        let mut formulas = Vec::new();
        for cell in original.cells() {
            if let CellContent::Formula(formula) = cell.content() {
                match formula.text() {
                    Some(text) => formulas.push(text.as_str()),
                    None => return Err(unsupported(&format!("the formula at {}", cell.address()))),
                }
            }
        }
        for table in original.tables() {
            for column in table.columns() {
                formulas.extend(
                    column
                        .calculated_column_formula()
                        .map(|formula| formula.text().as_str()),
                );
                formulas.extend(
                    column
                        .totals_row_formula()
                        .map(|formula| formula.text().as_str()),
                );
            }
        }
        for text in formulas {
            check_reads(text, original.name(), &own_tables, &mut used_names)?;
        }

        // The sheet's scoped names come with it, and so does each workbook name that reads only
        // the sheet. A name a formula uses must come, and so must the names it uses in turn.
        let defined_names = workbook.defined_names();
        let mut names = Vec::new();
        let mut taken = BTreeSet::new();
        for name in defined_names {
            if name.scope() == DefinedNameScope::Sheet(sheet_id) {
                check_reads(
                    name.formula().as_str(),
                    original.name(),
                    &own_tables,
                    &mut used_names,
                )?;
                taken.insert(name.lookup_key().to_owned());
                names.push(name.clone());
            }
        }
        for name in defined_names {
            if name.scope() != DefinedNameScope::Workbook {
                continue;
            }
            let Some(references) = formula_references(name.formula().as_str()) else {
                continue;
            };
            let reads_sheet = references
                .sheets
                .iter()
                .any(|sheet| case_insensitive_eq(sheet, original.name().as_str()));
            let reads_elsewhere = references.external
                || references.three_d
                || references
                    .sheets
                    .iter()
                    .any(|sheet| !case_insensitive_eq(sheet, original.name().as_str()));
            if reads_sheet && !reads_elsewhere && !taken.contains(name.lookup_key()) {
                check_reads(
                    name.formula().as_str(),
                    original.name(),
                    &own_tables,
                    &mut used_names,
                )?;
                taken.insert(name.lookup_key().to_owned());
                names.push(name.clone());
            }
        }
        let mut pending = used_names.into_iter().collect::<Vec<_>>();
        while let Some(used) = pending.pop() {
            if taken.contains(&used) {
                continue;
            }
            let Some(name) = defined_names.iter().find(|name| {
                name.scope() == DefinedNameScope::Workbook && name.lookup_key() == used
            }) else {
                continue;
            };
            let mut further = BTreeSet::new();
            check_reads(
                name.formula().as_str(),
                original.name(),
                &own_tables,
                &mut further,
            )
            .map_err(|_| {
                unsupported(&format!(
                    "the name {} that reads another sheet",
                    name.name()
                ))
            })?;
            taken.insert(used);
            names.push(name.clone());
            pending.extend(further);
        }

        // Formats become explicit: a style index means a cell format of the source workbook.
        let default_format = source.presentation().style_format(0);
        let positions = original
            .cells()
            .map(crate::Cell::address)
            .chain(original.new_cell_styles().styled_positions())
            .chain(
                source
                    .presentation()
                    .edited_cell_formats(sheet_id)
                    .map(|(address, _)| address),
            )
            .collect::<BTreeSet<_>>();
        let mut cell_formats = Vec::new();
        for address in positions {
            let format = source.cell_format(sheet_id, address)?;
            if format != default_format {
                cell_formats.push((address, format));
            }
        }
        let mut sheet = original.clone();
        sheet.clear_new_cell_styles();
        Ok(Self(Arc::new(ImportedSheetData {
            sheet,
            date_system: workbook.date_system(),
            cell_formats,
            column_widths: source.presentation().column_widths(sheet_id).to_vec(),
            row_heights: source.presentation().row_heights(sheet_id).collect(),
            presentation,
            names,
        })))
    }

    /// Returns the sheet's name in its own workbook.
    pub fn name(&self) -> &SheetName {
        self.0.sheet.name()
    }
}

/// Refuses a formula that does not parse or that reads another sheet, workbook, or table, and
/// collects the names it may use.
fn check_reads(
    text: &str,
    sheet_name: &SheetName,
    own_tables: &[String],
    names: &mut BTreeSet<String>,
) -> Result<(), ValidationError> {
    let references =
        formula_references(text).ok_or_else(|| unsupported(&format!("the formula {text}")))?;
    if references.external {
        return Err(unsupported("a reference to another workbook"));
    }
    if references.three_d {
        return Err(unsupported("a 3-D reference"));
    }
    if let Some(sheet) = references
        .sheets
        .iter()
        .find(|sheet| !case_insensitive_eq(sheet, sheet_name.as_str()))
    {
        return Err(unsupported(&format!("a reference to sheet '{sheet}'")));
    }
    if let Some(table) = references
        .tables
        .iter()
        .find(|table| !own_tables.iter().any(|own| case_insensitive_eq(own, table)))
    {
        return Err(unsupported(&format!("a reference to table {table}")));
    }
    names.extend(references.names.iter().map(|name| {
        name.chars()
            .flat_map(char::to_lowercase)
            .collect::<String>()
    }));
    Ok(())
}

fn unsupported(content: &str) -> ValidationError {
    ValidationError::SheetImportUnsupported {
        content: content.to_owned(),
    }
}

/// What an import adds to the target workbook.
pub(super) struct SheetImport<'a> {
    pub(super) imported: &'a ImportedSheet,
    pub(super) position: usize,
    pub(super) sheet_id: SheetId,
    /// The custom number formats the target uses, by ID; the import adds its own.
    pub(super) number_formats: &'a mut BTreeMap<u32, Box<str>>,
}

/// Adds the imported sheet at zero-based tab `position` as `sheet_id`, as Excel 16.114 copies a
/// sheet into another workbook.
///
/// The sheet keeps its name, or takes the name [`SheetName::copy_name`] gives when the target
/// has it, and its self-references follow. Each table keeps its name, or takes its name followed
/// by the target's table count plus two plus its index among the sheet's tables when the name is
/// taken. Scoped names come scoped to the new sheet, and a workbook name that reads the sheet
/// comes as a workbook name, or scoped to the new sheet when the target has that workbook name.
/// Returns the new table IDs.
pub(super) fn import_sheet(
    sheets: &mut Vec<Sheet>,
    defined_names: &mut Vec<DefinedName>,
    non_worksheet_tabs: &mut NonWorksheetTabs,
    date_system: DateSystem,
    budget: &mut FormulaRewriteBudget<'_>,
    SheetImport {
        imported,
        position,
        sheet_id,
        number_formats,
    }: SheetImport<'_>,
) -> Result<Vec<TableId>, BatchExecutionError> {
    let data = &imported.0;
    if data.date_system != date_system {
        return Err(unsupported("a different date system").into());
    }
    let order = TabOrder::of(sheets, non_worksheet_tabs);
    if position > order.entries.len() {
        return Err(ValidationError::SheetPositionOutOfRange {
            position,
            tab_count: order.entries.len() + 1,
        }
        .into());
    }
    let names = order.names();
    let is_taken = |candidate: &str| {
        names
            .iter()
            .any(|(name, _)| case_insensitive_eq(name, candidate))
    };
    let source_name = data.sheet.name().clone();
    let name = if is_taken(source_name.as_str()) {
        source_name.copy_name(is_taken)
    } else {
        source_name.clone()
    };

    let table_count = sheets
        .iter()
        .map(|sheet| sheet.tables().len())
        .sum::<usize>();
    let mut next_table_id = sheets
        .iter()
        .flat_map(Sheet::tables)
        .map(|table| table.id().get())
        .max()
        .unwrap_or(0);
    let mut taken_table_names = sheets
        .iter()
        .flat_map(Sheet::tables)
        .map(|table| table.display_name().as_str().to_owned())
        .chain(defined_names.iter().map(|name| name.name().to_owned()))
        .collect::<Vec<_>>();
    let mut sheet = data.sheet.clone();
    sheet.set_id(sheet_id);
    sheet.rename(name.clone());
    let mut table_ids = Vec::new();
    let mut table_renames = Vec::new();
    for (table_index, table) in sheet.tables_mut().iter_mut().enumerate() {
        let original = table.display_name().as_str().to_owned();
        let taken = |candidate: &str| {
            taken_table_names
                .iter()
                .any(|taken| case_insensitive_eq(taken, candidate))
        };
        let table_name = if taken(&original) {
            let mut number = table_count + 2 + table_index;
            loop {
                let candidate = format!("{original}{number}");
                if !taken(&candidate) {
                    break TableName::new(candidate)?;
                }
                number += 1;
            }
        } else {
            table.display_name().clone()
        };
        next_table_id = next_table_id
            .checked_add(1)
            .ok_or(ValidationError::UnsupportedTableAuthoringMetadata { table_id: u32::MAX })?;
        let id = TableId::new(next_table_id)?;
        if table_name.as_str() != original {
            table_renames.push((original, table_name.as_str().to_owned()));
        }
        taken_table_names.push(table_name.as_str().to_owned());
        table.set_id(id);
        table.rename(table_name);
        table_ids.push(id);
    }

    let mut requests = Vec::new();
    if name != source_name {
        requests.push(FormulaRewriteRequest::Sheet {
            old_name: source_name.as_str(),
            new_name: name.as_str(),
        });
    }
    requests.extend(
        table_renames
            .iter()
            .map(|(old_name, new_name)| FormulaRewriteRequest::Table { old_name, new_name }),
    );
    rewrite_copy(&mut sheet, &requests, budget)?;
    remap_number_formats(&mut sheet, number_formats);

    let source_sheet_id = data.sheet.id();
    for name in &data.names {
        let scope = match name.scope() {
            DefinedNameScope::Sheet(id) if id == source_sheet_id => {
                DefinedNameScope::Sheet(sheet_id)
            }
            _ if defined_names.iter().any(|existing| {
                existing.scope() == DefinedNameScope::Workbook
                    && existing.lookup_key() == name.lookup_key()
            }) =>
            {
                DefinedNameScope::Sheet(sheet_id)
            }
            _ => DefinedNameScope::Workbook,
        };
        let formula = match rewrite_text(name.formula().as_str(), &requests, budget)? {
            Some(text) => FormulaText::from_xlsx(text)?,
            None => name.formula().clone(),
        };
        defined_names.push(DefinedName::new(
            name.name(),
            scope,
            formula,
            name.hidden(),
        )?);
    }

    let mut placed = order.entries.to_vec();
    placed.insert(
        position,
        TabEntry {
            id: sheet_id,
            worksheet: true,
        },
    );
    sheets.push(sheet);
    place_tabs(sheets, non_worksheet_tabs, &placed)?;
    Ok(table_ids)
}

/// Returns the custom number formats the target's styles part declares and its cells use.
pub(super) fn used_number_formats(
    document: Option<&crate::XlsxDocument>,
    sheets: &[Sheet],
) -> Result<BTreeMap<u32, Box<str>>, ValidationError> {
    let mut used = match document {
        Some(document) => document
            .custom_number_formats()
            .map_err(|error| unsupported(&error.to_string()))?
            .into_iter()
            .map(|(id, code)| (id, code.into_boxed_str()))
            .collect(),
        None => BTreeMap::new(),
    };
    for cell in sheets.iter().flat_map(Sheet::cells) {
        let format = cell.number_format();
        if let Some(code) = format.code().filter(|_| format.id() >= 164) {
            used.entry(format.id()).or_insert_with(|| code.into());
        }
    }
    Ok(used)
}

/// Gives each custom number format of `sheet` an ID the target does not use for another code:
/// its own, the target's ID for the same code, or the next free one.
fn remap_number_formats(sheet: &mut Sheet, used: &mut BTreeMap<u32, Box<str>>) {
    let mut remapped = BTreeMap::<u32, u32>::new();
    let mut edits = Vec::new();
    for cell in sheet.cells() {
        let format = cell.number_format();
        let Some(code) = format.code().filter(|_| format.id() >= 164) else {
            continue;
        };
        let id = *remapped.entry(format.id()).or_insert_with(|| {
            if used
                .get(&format.id())
                .is_none_or(|existing| **existing == *code)
            {
                used.insert(format.id(), code.into());
                return format.id();
            }
            if let Some((id, _)) = used.iter().find(|(_, existing)| ***existing == *code) {
                return *id;
            }
            let id = used
                .keys()
                .next_back()
                .map_or(164, |last| (*last + 1).max(164));
            used.insert(id, code.into());
            id
        });
        if id != format.id() {
            edits.push((
                cell.address(),
                cell.content().clone(),
                NumberFormat::new(id, Some(code.into()), format.kind()),
            ));
        }
    }
    for (address, content, format) in edits {
        sheet.upsert_cell_deferred(address, content, format);
    }
    sheet.finish_deferred_cell_edits();
}

/// Gives the imported sheet `sheet_id` its formats, column widths, row heights, and panes.
pub(super) fn import_presentation(
    presentation: &mut crate::DocumentPresentation,
    imported: &ImportedSheet,
    sheet_id: SheetId,
) -> Result<(), ValidationError> {
    let data = &imported.0;
    for (address, format) in &data.cell_formats {
        presentation.set_cell_format(sheet_id, *address, format.clone())?;
    }
    for span in &data.column_widths {
        presentation.set_column_width(sheet_id, span.first(), span.last(), Some(span.width()))?;
    }
    for (row, height) in &data.row_heights {
        presentation.set_row_height(sheet_id, *row, Some(*height))?;
    }
    if let Some(sheet) = &data.presentation {
        presentation.import_sheet(sheet_id, sheet.clone());
    }
    Ok(())
}

#[cfg(test)]
#[path = "sheet_import_tests.rs"]
mod tests;
