use super::table_edit::ranges_overlap;
use super::widen_edit::{FILTER_DATABASE_NAME, absolute_a1, align_filter_database_name};
use crate::calculation::formula_rewrite::sheet_name_needs_quotes;
use crate::{
    CellRange, DefinedName, DefinedNameScope, FormulaText, Sheet, SheetAutoFilter, ValidationError,
    case_insensitive_eq,
};

/// What a sheet autoFilter edit changed.
pub(super) struct AutoFilterEdit {
    pub(super) filter_changed: bool,
    pub(super) names_changed: bool,
}

/// Creates, changes, or, for `None`, removes the sheet-level auto-filter, keeping the hidden
/// sheet-scoped `_xlnm._FilterDatabase` name Excel keeps equal to its range: a new filter gains
/// one, a changed filter moves it, and a removed filter takes it away.
///
/// A changed filter keeps the settings of the filter columns whose worksheet columns it still
/// covers. Rows a filter hid stay hidden.
pub(super) fn set_sheet_auto_filter(
    sheet: &mut Sheet,
    defined_names: &mut Vec<DefinedName>,
    range: Option<CellRange>,
) -> Result<AutoFilterEdit, ValidationError> {
    let filter_database = |name: &DefinedName, sheet: &Sheet| {
        name.scope() == DefinedNameScope::Sheet(sheet.id())
            && case_insensitive_eq(name.name(), FILTER_DATABASE_NAME)
    };
    let Some(range) = range else {
        if sheet.auto_filter().is_none() {
            return Ok(AutoFilterEdit {
                filter_changed: false,
                names_changed: false,
            });
        }
        sheet.set_auto_filter(None);
        let before = defined_names.len();
        defined_names.retain(|name| !filter_database(name, sheet));
        return Ok(AutoFilterEdit {
            filter_changed: true,
            names_changed: defined_names.len() != before,
        });
    };
    if sheet
        .auto_filter()
        .is_some_and(|filter| filter.range() == range)
    {
        return Ok(AutoFilterEdit {
            filter_changed: false,
            names_changed: false,
        });
    }
    if let Some(table) = sheet
        .tables()
        .iter()
        .find(|table| ranges_overlap(range, table.range()))
    {
        return Err(ValidationError::SheetAutoFilterOverlapsTable {
            sheet_id: sheet.id().get(),
            table_id: table.id().get(),
        });
    }
    let filter = sheet.auto_filter().map_or_else(
        || SheetAutoFilter::from_xlsx(range, Vec::new(), Vec::new()),
        |filter| filter.respanned(range),
    );
    sheet.set_auto_filter(Some(filter));
    if defined_names
        .iter()
        .any(|name| filter_database(name, sheet))
    {
        let names_changed = align_filter_database_name(sheet, defined_names)?;
        return Ok(AutoFilterEdit {
            filter_changed: true,
            names_changed,
        });
    }
    let sheet_name = sheet.name().as_str();
    let qualifier = if sheet_name_needs_quotes(sheet_name) {
        format!("'{}'", sheet_name.replace('\'', "''"))
    } else {
        sheet_name.to_owned()
    };
    defined_names.push(DefinedName::new(
        FILTER_DATABASE_NAME,
        DefinedNameScope::Sheet(sheet.id()),
        FormulaText::from_xlsx(format!(
            "{qualifier}!{}:{}",
            absolute_a1(range.start()),
            absolute_a1(range.end())
        ))?,
        true,
    )?);
    Ok(AutoFilterEdit {
        filter_changed: true,
        names_changed: true,
    })
}

#[cfg(test)]
#[path = "auto_filter_edit_tests.rs"]
mod tests;
