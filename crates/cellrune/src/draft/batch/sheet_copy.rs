use super::BatchExecutionError;
use super::formula_edit::WorkbookFormulaRename;
use super::formula_edit::rewrite_sheet_metadata_formulas;
use super::tab_order::{TabOrder, place_tabs};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteRequest, SheetChanges, rewrite_formula,
};
use crate::calculation::sheet_changes::TabEntry;
use crate::workbook::NonWorksheetTabs;
use crate::{
    CellContent, DefinedName, DefinedNameScope, FormulaText, Sheet, SheetId, TableId, TableName,
    ValidationError, WorkbookSnapshot, case_insensitive_eq,
};

/// The tables a sheet copy gave new identities, as `(source, copy)` IDs.
pub(super) type CopiedTables = Vec<(TableId, TableId)>;

/// One worksheet copy: the source sheet, the zero-based tab position the copy takes, and its ID.
pub(super) struct SheetCopy {
    pub(super) sheet_id: SheetId,
    pub(super) position: usize,
    pub(super) copy_id: SheetId,
}

/// Copies the worksheet `sheet_id` to zero-based tab `position` as `copy_id`, as Excel's Move or
/// Copy does within a workbook.
///
/// The copy is named as [`SheetName::copy_name`] names it. In its formulas, table formulas, and
/// data validations, a single-sheet reference to the source sheet names the copy, while other
/// sheets' and 3-D references keep their text. Its tables take the next free IDs and, as Excel
/// 16.114 names them, their names followed by the workbook's table count plus two plus their
/// index among the copied tables, the next free number when taken; structured references to them
/// inside the copy follow. The source's scoped names are copied to the copy, and each workbook
/// name that reads the source sheet becomes a name of the copy too. Returns the copied tables.
pub(super) fn copy_sheet(
    sheets: &mut Vec<Sheet>,
    defined_names: &mut Vec<DefinedName>,
    non_worksheet_tabs: &mut NonWorksheetTabs,
    source: Option<&WorkbookSnapshot>,
    budget: &mut FormulaRewriteBudget<'_>,
    SheetCopy {
        sheet_id,
        position,
        copy_id,
    }: SheetCopy,
) -> Result<CopiedTables, BatchExecutionError> {
    let index = sheets
        .iter()
        .position(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })?;
    let order = TabOrder::of(sheets, non_worksheet_tabs);
    if position > order.entries.len() {
        return Err(ValidationError::SheetPositionOutOfRange {
            position,
            tab_count: order.entries.len() + 1,
        }
        .into());
    }
    let names = order.names();
    let copy_name = sheets[index].name().copy_name(|candidate| {
        names
            .iter()
            .any(|(name, _)| case_insensitive_eq(name, candidate))
    });

    // Tables keep unique names across the workbook and its defined names.
    let table_count = sheets
        .iter()
        .map(|sheet| sheet.tables().len())
        .sum::<usize>();
    let mut next_table_id = sheets
        .iter()
        .chain(source.into_iter().flat_map(WorkbookSnapshot::sheets))
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
    let mut copy = sheets[index].clone();
    copy.set_id(copy_id);
    copy.rename(copy_name.clone());
    let mut copied_tables = Vec::new();
    let mut table_renames = Vec::new();
    for (table_index, table) in copy.tables_mut().iter_mut().enumerate() {
        let mut number = table_count + 2 + table_index;
        let name = loop {
            let candidate = format!("{}{number}", table.display_name().as_str());
            if !taken_table_names
                .iter()
                .any(|taken| case_insensitive_eq(taken, &candidate))
            {
                break TableName::new(candidate)?;
            }
            number += 1;
        };
        next_table_id = next_table_id
            .checked_add(1)
            .ok_or(ValidationError::UnsupportedTableAuthoringMetadata { table_id: u32::MAX })?;
        let copied_id = TableId::new(next_table_id)?;
        table_renames.push((
            table.display_name().as_str().to_owned(),
            name.as_str().to_owned(),
        ));
        copied_tables.push((table.id(), copied_id));
        taken_table_names.push(name.as_str().to_owned());
        table.set_id(copied_id);
        table.rename(name);
    }

    // Self-references name the copy; references to the copied tables name the copies.
    let mut after = order.unchanged_names();
    after.push((copy_id, copy_name.as_str()));
    let retarget = FormulaRewriteRequest::Sheets(SheetChanges {
        before: &names,
        after: &after,
        steps: &[],
        retarget: Some((sheet_id, copy_id)),
        deleted_tables: &[],
    });
    let mut requests = vec![retarget.clone()];
    requests.extend(
        table_renames
            .iter()
            .map(|(old_name, new_name)| FormulaRewriteRequest::Table { old_name, new_name }),
    );
    rewrite_copy(&mut copy, &requests, budget)?;
    if let FormulaRewriteRequest::Sheets(changes) = &retarget {
        rewrite_sheet_metadata_formulas(
            std::slice::from_mut(&mut copy),
            &WorkbookFormulaRename::SheetChanges(*changes),
            false,
            budget,
        )?;
    }

    let mut copied_names = Vec::new();
    for defined_name in defined_names.iter() {
        let scoped = defined_name.scope() == DefinedNameScope::Sheet(sheet_id);
        let rewritten = rewrite_text(defined_name.formula().as_str(), &requests, budget)?;
        let workbook_name_reads_source = defined_name.scope() == DefinedNameScope::Workbook
            && rewrite_formula(defined_name.formula().as_str(), &retarget, budget)?.is_some();
        if scoped || workbook_name_reads_source {
            copied_names.push((
                scoped,
                DefinedName::new(
                    defined_name.name(),
                    DefinedNameScope::Sheet(copy_id),
                    match rewritten {
                        Some(text) => FormulaText::from_xlsx(text)?,
                        None => defined_name.formula().clone(),
                    },
                    defined_name.hidden(),
                )?,
            ));
        }
    }
    // A copied sheet name wins over a workbook name of the same spelling.
    for (scoped, name) in &copied_names {
        if *scoped
            || !copied_names.iter().any(|(other_scoped, other)| {
                *other_scoped && other.lookup_key() == name.lookup_key()
            })
        {
            defined_names.push(name.clone());
        }
    }

    let mut placed = order.entries.to_vec();
    placed.insert(
        position,
        TabEntry {
            id: copy_id,
            worksheet: true,
        },
    );
    sheets.push(copy);
    place_tabs(sheets, non_worksheet_tabs, &placed)?;
    Ok(copied_tables)
}

/// Applies `requests` in turn to every cell formula and table formula of the copy.
fn rewrite_copy(
    copy: &mut Sheet,
    requests: &[FormulaRewriteRequest<'_>],
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), BatchExecutionError> {
    let mut previous_address = None;
    loop {
        budget.check_cancelled()?;
        let Some(cell) = copy.next_formula_cell_after(previous_address) else {
            break;
        };
        let address = cell.address();
        previous_address = Some(address);
        let CellContent::Formula(formula) = cell.content() else {
            unreachable!("formula traversal returned a non-formula cell");
        };
        let Some(text) = formula.text() else {
            continue;
        };
        if let Some(rewritten) = rewrite_text(text.as_str(), requests, budget)? {
            let content = CellContent::Formula(
                formula
                    .clone()
                    .with_text(FormulaText::from_xlsx(rewritten)?),
            );
            let number_format = cell.number_format().clone();
            copy.upsert_cell_deferred(address, content, number_format);
        }
    }
    copy.finish_deferred_cell_edits();
    for table in copy.tables_mut() {
        for column in table.columns_mut() {
            let calculated = column
                .calculated_column_formula()
                .map(|formula| rewrite_text(formula.text().as_str(), requests, budget))
                .transpose()?
                .flatten()
                .map(FormulaText::from_xlsx)
                .transpose()?;
            let totals = column
                .totals_row_formula()
                .map(|formula| rewrite_text(formula.text().as_str(), requests, budget))
                .transpose()?
                .flatten()
                .map(FormulaText::from_xlsx)
                .transpose()?;
            if calculated.is_some() || totals.is_some() {
                column.rewrite_formulas(calculated, totals);
            }
        }
    }
    Ok(())
}

/// Applies `requests` in turn and returns the result when any changed the text.
fn rewrite_text(
    text: &str,
    requests: &[FormulaRewriteRequest<'_>],
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Option<String>, BatchExecutionError> {
    let mut current = None::<String>;
    for request in requests {
        if let Some(rewritten) =
            rewrite_formula(current.as_deref().unwrap_or(text), request, budget)?
        {
            current = Some(rewritten);
        }
    }
    Ok(current)
}

#[cfg(test)]
#[path = "sheet_copy_tests.rs"]
mod tests;
