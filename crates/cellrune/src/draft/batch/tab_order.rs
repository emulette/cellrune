use std::sync::Arc;

use crate::calculation::sheet_changes::TabEntry;
use crate::workbook::{NonWorksheetTab, NonWorksheetTabs};
use crate::{Sheet, SheetId, SheetName, ValidationError};

/// The staged workbook's tabs in tab order, worksheets and others, as a formula rewrite names
/// them and as a deletion or move records them.
pub(super) struct TabOrder {
    names: Vec<(SheetName, SheetId)>,
    pub(super) entries: Arc<[TabEntry]>,
}

impl TabOrder {
    pub(super) fn of(sheets: &[Sheet], non_worksheet_tabs: &NonWorksheetTabs) -> Self {
        let tabs = non_worksheet_tabs.in_tab_order(
            sheets
                .iter()
                .map(|sheet| (sheet.name().clone(), sheet.id(), true)),
            |tab| (tab.name().clone(), tab.id(), false),
        );
        Self {
            entries: tabs
                .iter()
                .map(|(_, id, worksheet)| TabEntry {
                    id: *id,
                    worksheet: *worksheet,
                })
                .collect(),
            names: tabs.into_iter().map(|(name, id, _)| (name, id)).collect(),
        }
    }

    /// Returns each tab by its current name, as a rewrite resolves the names formulas spell.
    pub(super) fn names(&self) -> Vec<(&str, SheetId)> {
        self.names
            .iter()
            .map(|(name, id)| (name.as_str(), *id))
            .collect()
    }

    /// Returns each tab's ID and current name, which a change that renames nothing leaves as they
    /// are.
    pub(super) fn unchanged_names(&self) -> Vec<(SheetId, &str)> {
        self.names
            .iter()
            .map(|(name, id)| (*id, name.as_str()))
            .collect()
    }
}

/// Puts the worksheets and the other tabs in `order`: worksheets in their order, and each other
/// tab at its position.
pub(super) fn place_tabs(
    sheets: &mut Vec<Sheet>,
    non_worksheet_tabs: &mut NonWorksheetTabs,
    order: &[TabEntry],
) -> Result<(), ValidationError> {
    let mut taken = std::mem::take(sheets)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let mut tabs = Vec::new();
    for (index, entry) in order.iter().enumerate() {
        let unknown = ValidationError::UnknownSheetId {
            value: entry.id.get(),
        };
        if entry.worksheet {
            let sheet = taken
                .iter_mut()
                .find(|sheet| sheet.as_ref().is_some_and(|sheet| sheet.id() == entry.id))
                .and_then(Option::take)
                .ok_or(unknown)?;
            sheets.push(sheet);
        } else {
            let tab = non_worksheet_tabs.by_id(entry.id).ok_or(unknown)?;
            tabs.push(NonWorksheetTab::new(
                index,
                tab.id(),
                tab.name().clone(),
                tab.visibility(),
            ));
        }
    }
    *non_worksheet_tabs = NonWorksheetTabs::new(tabs);
    Ok(())
}
