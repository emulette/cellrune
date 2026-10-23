use std::sync::Arc;

use super::{SheetId, SheetName, SheetVisibility};

/// A workbook tab that is not a worksheet: a chartsheet, dialogsheet, or macrosheet.
///
/// Such a tab holds no cells and is excluded from calculation, but it keeps its place in the
/// workbook tab order, which Excel counts in `SHEET`, `SHEETS`, and sheet-scoped defined names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NonWorksheetTab {
    position: usize,
    id: SheetId,
    name: SheetName,
    visibility: SheetVisibility,
}

impl NonWorksheetTab {
    /// Creates a tab at its zero-based position in the complete `<sheets>` list.
    pub(crate) const fn new(
        position: usize,
        id: SheetId,
        name: SheetName,
        visibility: SheetVisibility,
    ) -> Self {
        Self {
            position,
            id,
            name,
            visibility,
        }
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

    pub(crate) const fn visibility(&self) -> SheetVisibility {
        self.visibility
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

    /// Merges one item per worksheet, in worksheet order, with one per tab that is not a worksheet
    /// into tab order.
    pub(crate) fn in_tab_order<'a, T>(
        &'a self,
        worksheets: impl IntoIterator<Item = T>,
        tab_item: impl Fn(&'a NonWorksheetTab) -> T,
    ) -> Vec<T> {
        let mut worksheets = worksheets.into_iter();
        let mut tabs = self.0.iter().peekable();
        let mut items = Vec::new();
        loop {
            let item = match tabs.next_if(|tab| tab.position <= items.len()) {
                Some(tab) => tab_item(tab),
                None => match worksheets.next() {
                    Some(item) => item,
                    None => match tabs.next() {
                        Some(tab) => tab_item(tab),
                        None => return items,
                    },
                },
            };
            items.push(item);
        }
    }

    /// Returns these tabs once the worksheet at tab `position` is deleted: every later tab moves
    /// one position toward the front.
    pub(crate) fn without_worksheet_at(&self, position: usize) -> Self {
        Self(
            self.0
                .iter()
                .map(|tab| NonWorksheetTab {
                    position: if tab.position > position {
                        tab.position - 1
                    } else {
                        tab.position
                    },
                    ..tab.clone()
                })
                .collect(),
        )
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
    use crate::{SheetId, SheetName, SheetVisibility};

    fn tab(position: usize, id: u32) -> NonWorksheetTab {
        NonWorksheetTab::new(
            position,
            SheetId::new(id).expect("sheet id"),
            SheetName::new(format!("Chart{id}")).expect("sheet name"),
            SheetVisibility::Visible,
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

    #[test]
    fn deleting_a_worksheet_moves_only_later_tabs_forward() {
        // Tab order: Chart1, Sheet(0), Chart2, Chart3, Sheet(1); Sheet(0) is deleted.
        let tabs = NonWorksheetTabs::new(vec![tab(0, 1), tab(2, 2), tab(3, 3)]);
        let remaining = tabs.without_worksheet_at(1);
        let positions = remaining
            .iter()
            .map(NonWorksheetTab::position)
            .collect::<Vec<_>>();
        assert_eq!(positions, [0, 1, 2]);
        assert_eq!(remaining.worksheet_position(0), 3);
        let names = |tabs: &NonWorksheetTabs, worksheets: &[&'static str]| {
            tabs.in_tab_order(worksheets.iter().copied(), |tab| tab.name().as_str())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&tabs, &["First", "Second"]),
            ["Chart1", "First", "Chart2", "Chart3", "Second"]
        );
        assert_eq!(
            names(&remaining, &["Second"]),
            ["Chart1", "Chart2", "Chart3", "Second"]
        );
    }
}
