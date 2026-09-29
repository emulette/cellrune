use std::sync::Arc;

use super::{SheetId, SheetName};

/// A workbook tab that is not a worksheet: a chartsheet, dialogsheet, or macrosheet.
///
/// Such a tab holds no cells and is excluded from calculation, but it keeps its place in the
/// workbook tab order, which Excel counts in `SHEET`, `SHEETS`, and sheet-scoped defined names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NonWorksheetTab {
    position: usize,
    id: SheetId,
    name: SheetName,
}

impl NonWorksheetTab {
    /// Creates a tab at its zero-based position in the complete `<sheets>` list.
    pub(crate) const fn new(position: usize, id: SheetId, name: SheetName) -> Self {
        Self { position, id, name }
    }

    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    pub(crate) const fn id(&self) -> SheetId {
        self.id
    }

    pub(crate) const fn name(&self) -> &SheetName {
        &self.name
    }
}

/// Non-worksheet tabs in ascending tab position.
///
/// Worksheets fill the remaining positions in worksheet order, so a worksheet's tab position
/// follows from its worksheet index. Worksheets added later are appended after every existing
/// tab and leave these positions unchanged.
#[derive(Debug, Clone, Default)]
pub(crate) struct NonWorksheetTabs(Arc<[NonWorksheetTab]>);

impl NonWorksheetTabs {
    /// Wraps tabs listed in ascending position, as the workbook part declares them.
    pub(crate) fn new(tabs: Vec<NonWorksheetTab>) -> Self {
        Self(tabs.into())
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &NonWorksheetTab> {
        self.0.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the tab position of the worksheet at `worksheet_index`.
    pub(crate) fn worksheet_position(&self, worksheet_index: usize) -> usize {
        let mut position = worksheet_index;
        for tab in self.0.iter() {
            if tab.position > position {
                break;
            }
            position += 1;
        }
        position
    }

    pub(crate) fn contains_position(&self, position: usize) -> bool {
        self.0
            .binary_search_by_key(&position, NonWorksheetTab::position)
            .is_ok()
    }

    pub(crate) fn by_id(&self, id: SheetId) -> Option<&NonWorksheetTab> {
        self.0.iter().find(|tab| tab.id == id)
    }

    pub(crate) fn by_lookup_key(&self, lookup_key: &str) -> Option<&NonWorksheetTab> {
        self.0
            .iter()
            .find(|tab| tab.name.lookup_key() == lookup_key)
    }
}

#[cfg(test)]
mod tests {
    use super::{NonWorksheetTab, NonWorksheetTabs};
    use crate::{SheetId, SheetName};

    fn tab(position: usize, id: u32) -> NonWorksheetTab {
        NonWorksheetTab::new(
            position,
            SheetId::new(id).expect("sheet id"),
            SheetName::new(format!("Chart{id}")).expect("sheet name"),
        )
    }

    #[test]
    fn worksheets_fill_the_positions_around_non_worksheet_tabs() {
        // Tab order: Chart1, Sheet(0), Chart2, Chart3, Sheet(1), Sheet(2).
        let tabs = NonWorksheetTabs::new(vec![tab(0, 1), tab(2, 2), tab(3, 3)]);
        let positions = (0..4)
            .map(|index| tabs.worksheet_position(index))
            .collect::<Vec<_>>();
        assert_eq!(positions, [1, 4, 5, 6]);
        assert!(tabs.contains_position(3));
        assert!(!tabs.contains_position(4));
        assert_eq!(NonWorksheetTabs::default().worksheet_position(2), 2);
    }
}
