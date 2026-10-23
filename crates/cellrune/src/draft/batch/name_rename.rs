use super::super::case_insensitive_key;
use super::BatchExecutionError;
use super::formula_edit::{
    WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_sheet_metadata_formulas,
    rewrite_workbook_formulas,
};
use crate::calculation::formula_rewrite::NameRename;
use crate::workbook::NonWorksheetTabs;
use crate::{DefinedName, DefinedNameScope, Sheet, SheetId, SheetName, ValidationError};

/// A defined-name rename as the writer replays it on the formulas of the parts it preserves:
/// by sheet ID, since a later edit may rename the sheets involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinedNameRename {
    pub(crate) scope: DefinedNameScope,
    pub(crate) old_name: Box<str>,
    pub(crate) new_name: Box<str>,
    /// Sheets with a name of their own spelled `old_name` when a workbook name was renamed.
    pub(crate) shadowing_sheets: Vec<SheetId>,
}

/// Renames the defined name `name` in `scope` and rewrites the references that resolve to it in
/// cell formulas, defined names, table formulas, and data validation formulas. Returns the rename
/// for the writer, or `None` when the name is unchanged.
///
/// A use resolves to a sheet's own name on that sheet or qualified with it, else to the workbook
/// name, so a rename that would let a name of the other scope take over references fails.
pub(super) fn rename_defined_name(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    non_worksheet_tabs: &NonWorksheetTabs,
    edit: WorkbookFormulaEdit<'_, '_>,
    scope: DefinedNameScope,
    name: &str,
    new_name: &str,
) -> Result<Option<DefinedNameRename>, BatchExecutionError> {
    let key = case_insensitive_key(name);
    let index = defined_names
        .iter()
        .position(|candidate| candidate.scope() == scope && candidate.lookup_key() == key)
        .ok_or_else(|| ValidationError::UnknownDefinedName {
            name: name.to_owned(),
        })?;
    let current = &defined_names[index];
    if current.name() == new_name {
        return Ok(None);
    }
    let renamed = DefinedName::new(new_name, scope, current.formula().clone(), current.hidden())?;
    let new_key = renamed.lookup_key();
    if defined_names.iter().enumerate().any(|(other, candidate)| {
        other != index && candidate.scope() == scope && candidate.lookup_key() == new_key
    }) {
        return Err(ValidationError::DuplicateDefinedName {
            name: new_name.to_owned(),
        }
        .into());
    }
    // A sheet's own name wins over a workbook name of the same spelling, so the new name must
    // not exist in the other scope.
    let other_scope = defined_names.iter().any(|candidate| {
        candidate.lookup_key() == new_key
            && match scope {
                DefinedNameScope::Workbook => candidate.scope() != DefinedNameScope::Workbook,
                DefinedNameScope::Sheet(_) => candidate.scope() == DefinedNameScope::Workbook,
            }
    });
    if other_scope {
        return Err(ValidationError::DefinedNameRenameShadowed {
            name: new_name.to_owned(),
        }
        .into());
    }

    let tab_name = |sheet_id: SheetId| -> Option<SheetName> {
        sheets
            .iter()
            .find(|sheet| sheet.id() == sheet_id)
            .map(|sheet| sheet.name().clone())
            .or_else(|| {
                non_worksheet_tabs
                    .by_id(sheet_id)
                    .map(|tab| tab.name().clone())
            })
    };
    let scope_sheet = match scope {
        DefinedNameScope::Workbook => None,
        DefinedNameScope::Sheet(sheet_id) => Some(tab_name(sheet_id).ok_or(
            ValidationError::DefinedNameUnknownSheet {
                sheet_id: sheet_id.get(),
            },
        )?),
    };
    let shadowing = match scope {
        DefinedNameScope::Workbook => defined_names
            .iter()
            .filter(|candidate| candidate.lookup_key() == key)
            .filter_map(|candidate| match candidate.scope() {
                DefinedNameScope::Sheet(sheet_id) => Some(sheet_id),
                DefinedNameScope::Workbook => None,
            })
            .collect::<Vec<_>>(),
        DefinedNameScope::Sheet(_) => Vec::new(),
    };
    let shadowing_names = shadowing
        .iter()
        .filter_map(|sheet_id| tab_name(*sheet_id))
        .collect::<Vec<_>>();
    let shadowing_sheets = shadowing_names
        .iter()
        .map(SheetName::as_str)
        .collect::<Vec<_>>();
    let rename = NameRename {
        old_name: name,
        new_name,
        scope_sheet: scope_sheet.as_ref().map(SheetName::as_str),
        shadowing_sheets: &shadowing_sheets,
        owner_sheet: None,
    };
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        table_formula_locations,
        budget,
    } = edit;
    rewrite_sheet_metadata_formulas(
        sheets,
        &WorkbookFormulaRename::DefinedName(rename),
        false,
        &mut *budget,
    )?;
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state,
            changed_table_ids,
            table_formula_locations,
            budget,
        },
        WorkbookFormulaRename::DefinedName(rename),
    )?;
    let old_name = defined_names[index].name().into();
    defined_names[index] = renamed;
    Ok(Some(DefinedNameRename {
        scope,
        old_name,
        new_name: new_name.into(),
        shadowing_sheets: shadowing,
    }))
}

#[cfg(test)]
#[path = "name_rename_tests.rs"]
mod tests;
