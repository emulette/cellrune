//! Sheet renames, deletions, moves, and copies as a formula rewrite sees them: each sheet a formula
//! names resolves to its ID, single-sheet references follow that ID to its new name, and a 3-D
//! reference's ends follow the deletions and moves in the order they were made.

use std::sync::Arc;

use crate::{SheetId, case_insensitive_eq};

/// One tab of a tab order a deletion or move works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TabEntry {
    pub(crate) id: SheetId,
    /// Whether the tab is a worksheet; a moving 3-D end passes over the others, as Excel does.
    pub(crate) worksheet: bool,
}

/// A change to the tab order, which decides the tabs a 3-D reference spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TabStep {
    /// `deleted` leaves `tabs`, the tab order before the deletion.
    Delete {
        tabs: Arc<[TabEntry]>,
        deleted: SheetId,
    },
    /// The tab at `from` in `tabs`, the tab order before the move, moves to position `to` of the
    /// order after it.
    Move {
        tabs: Arc<[TabEntry]>,
        from: usize,
        to: usize,
    },
}

impl TabStep {
    fn mentions(&self, id: SheetId) -> bool {
        match self {
            Self::Delete { deleted, .. } => *deleted == id,
            Self::Move { tabs, from, .. } => tabs[*from].id == id,
        }
    }
}

/// Sheet changes made together or in sequence, with every name as formulas spell it before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SheetChanges<'a> {
    /// Each tab, worksheets and others, by its name before the changes.
    pub(crate) before: &'a [(&'a str, SheetId)],
    /// Each remaining tab's name after the changes; a tab missing here was deleted.
    pub(crate) after: &'a [(SheetId, &'a str)],
    /// The deletions and moves, in the order they were made.
    pub(crate) steps: &'a [TabStep],
    /// A sheet whose single-sheet references name another sheet instead, as a copied sheet's
    /// references to its source name the copy. 3-D references keep their tabs.
    pub(crate) retarget: Option<(SheetId, SheetId)>,
    /// Display names of the tables on deleted sheets.
    pub(crate) deleted_tables: &'a [&'a str],
}

/// What a single-sheet qualifier becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SheetOutcome<'a> {
    Unchanged,
    Renamed(&'a str),
    Deleted,
}

/// What the qualifier of a 3-D reference becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpanOutcome<'a> {
    Unchanged,
    /// The reference becomes `#REF!`.
    Deleted,
    /// Either end is spelled anew; `None` keeps that end's text.
    Ends {
        start: Option<&'a str>,
        end: Option<&'a str>,
    },
    /// The ends met, and the reference names this one sheet.
    Collapsed(&'a str),
}

impl<'a> SheetChanges<'a> {
    fn id(&self, name: &str) -> Option<SheetId> {
        self.before
            .iter()
            .find(|(before, _)| case_insensitive_eq(name, before))
            .map(|(_, id)| *id)
    }

    fn before_name(&self, id: SheetId) -> Option<&'a str> {
        self.before
            .iter()
            .find(|(_, before)| *before == id)
            .map(|(name, _)| *name)
    }

    fn after_name(&self, id: SheetId) -> Option<&'a str> {
        self.after
            .iter()
            .find(|(after, _)| *after == id)
            .map(|(_, name)| *name)
    }

    /// Returns the new spelling of a tab that kept its ID, when it was renamed.
    fn renamed(&self, id: SheetId) -> Option<&'a str> {
        let after = self.after_name(id)?;
        (self.before_name(id) != Some(after)).then_some(after)
    }

    pub(crate) fn is_deleted_table(&self, name: &str) -> bool {
        self.deleted_tables
            .iter()
            .any(|table| case_insensitive_eq(name, table))
    }

    /// Resolves a single-sheet qualifier.
    pub(crate) fn single(&self, name: &str) -> SheetOutcome<'a> {
        let Some(id) = self.id(name) else {
            return SheetOutcome::Unchanged;
        };
        let target = match self.retarget {
            Some((source, copy)) if source == id => copy,
            _ => id,
        };
        match self.after_name(target) {
            None => SheetOutcome::Deleted,
            Some(after) if target != id || self.before_name(id) != Some(after) => {
                SheetOutcome::Renamed(after)
            }
            Some(_) => SheetOutcome::Unchanged,
        }
    }

    /// Resolves the qualifier of a 3-D reference from `start` through `end`.
    pub(crate) fn span(&self, start: &str, end: &str) -> SpanOutcome<'a> {
        let (Some(first), Some(last)) = (self.id(start), self.id(end)) else {
            // A span the tab order cannot place keeps its text, its known end renamed, unless
            // that end is gone.
            let known = self.id(start).or(self.id(end));
            if known.is_some_and(|id| self.after_name(id).is_none()) {
                return SpanOutcome::Deleted;
            }
            let start = self.id(start).and_then(|id| self.renamed(id));
            let end = self.id(end).and_then(|id| self.renamed(id));
            return if start.is_none() && end.is_none() {
                SpanOutcome::Unchanged
            } else {
                SpanOutcome::Ends { start, end }
            };
        };
        let Some((new_first, new_last)) = self.span_after(first, last) else {
            return SpanOutcome::Deleted;
        };
        let (Some(start_name), Some(end_name)) =
            (self.after_name(new_first), self.after_name(new_last))
        else {
            return SpanOutcome::Deleted;
        };
        if new_first == new_last && first != last {
            return SpanOutcome::Collapsed(start_name);
        }
        let start = (new_first != first || self.renamed(first).is_some()).then_some(start_name);
        let end = (new_last != last || self.renamed(last).is_some()).then_some(end_name);
        if start.is_none() && end.is_none() {
            SpanOutcome::Unchanged
        } else {
            SpanOutcome::Ends { start, end }
        }
    }

    /// Reports whether a change can touch a reference naming `name`, for refusing a formula that
    /// does not parse rather than leaving it stale.
    pub(crate) fn targets(&self, name: &str) -> bool {
        self.id(name).is_some_and(|id| {
            !matches!(self.single(name), SheetOutcome::Unchanged)
                || self.steps.iter().any(|step| step.mentions(id))
        })
    }

    /// Follows a 3-D reference's ends through the deletions and moves. A deleted end moves to the
    /// nearest remaining worksheet toward the other end; a move that puts the ends out of their
    /// order makes the moved end its neighbour inside the old span, the start the next tab and the
    /// end the previous one, as Excel 16.114 does. `None` means no worksheet remains.
    fn span_after(&self, mut start: SheetId, mut end: SheetId) -> Option<(SheetId, SheetId)> {
        for step in self.steps {
            match step {
                TabStep::Delete { tabs, deleted } => {
                    if start != *deleted && end != *deleted {
                        continue;
                    }
                    let first = position(tabs, start)?;
                    let last = position(tabs, end)?;
                    if start == *deleted {
                        start = nearest_worksheet(tabs, first, last, *deleted)?;
                    }
                    if end == *deleted {
                        end = nearest_worksheet(tabs, last, first, *deleted)?;
                    }
                }
                TabStep::Move { tabs, from, to } => {
                    let moved = tabs[*from].id;
                    if start == end || (moved != start && moved != end) {
                        continue;
                    }
                    let first = position(tabs, start)?;
                    let last = position(tabs, end)?;
                    let placed = |position| moved_position(position, *from, *to);
                    if (first < last) == (placed(first) < placed(last)) {
                        continue;
                    }
                    if moved == start {
                        start = neighbour(tabs, first, last);
                    } else {
                        end = neighbour(tabs, last, first);
                    }
                }
            }
        }
        Some((start, end))
    }
}

fn position(tabs: &[TabEntry], id: SheetId) -> Option<usize> {
    tabs.iter().position(|tab| tab.id == id)
}

/// Returns where the tab at `position` lands once the tab at `from` moves to `to`.
const fn moved_position(position: usize, from: usize, to: usize) -> usize {
    if position == from {
        return to;
    }
    let without = if position > from {
        position - 1
    } else {
        position
    };
    if without >= to { without + 1 } else { without }
}

const fn step_toward(position: usize, toward: usize) -> usize {
    if position < toward {
        position + 1
    } else {
        position - 1
    }
}

/// Walks from the deleted tab at `from` toward `to`, inclusive, to the first worksheet.
fn nearest_worksheet(
    tabs: &[TabEntry],
    from: usize,
    to: usize,
    deleted: SheetId,
) -> Option<SheetId> {
    let mut position = from;
    loop {
        let tab = tabs[position];
        if tab.id != deleted && tab.worksheet {
            return Some(tab.id);
        }
        if position == to {
            return None;
        }
        position = step_toward(position, to);
    }
}

/// Returns the next worksheet from `from` toward `to`, or the tab at `to` once the walk reaches it.
fn neighbour(tabs: &[TabEntry], from: usize, to: usize) -> SheetId {
    let mut position = step_toward(from, to);
    while position != to && !tabs[position].worksheet {
        position = step_toward(position, to);
    }
    tabs[position].id
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{SheetChanges, SpanOutcome, TabEntry, TabStep};
    use crate::SheetId;

    fn id(value: u32) -> SheetId {
        SheetId::new(value).expect("sheet ID")
    }

    /// Tabs `S1` through `S6` with IDs 1 through 6.
    fn tabs() -> Arc<[TabEntry]> {
        (1..=6)
            .map(|value| TabEntry {
                id: id(value),
                worksheet: true,
            })
            .collect()
    }

    const NAMES: [&str; 6] = ["S1", "S2", "S3", "S4", "S5", "S6"];

    fn span_after_move(from: usize, to: usize, start: &str, end: &str) -> SpanOutcome<'static> {
        let before = NAMES
            .iter()
            .zip(1..)
            .map(|(name, value)| (*name, id(value)))
            .collect::<Vec<_>>();
        let after = before
            .iter()
            .map(|(name, id)| (*id, *name))
            .collect::<Vec<_>>();
        let steps = [TabStep::Move {
            tabs: tabs(),
            from,
            to,
        }];
        let changes = SheetChanges {
            before: &before,
            after: &after,
            steps: &steps,
            retarget: None,
            deleted_tables: &[],
        };
        match changes.span(start, end) {
            SpanOutcome::Unchanged => SpanOutcome::Unchanged,
            SpanOutcome::Deleted => SpanOutcome::Deleted,
            SpanOutcome::Collapsed(name) => {
                SpanOutcome::Collapsed(NAMES.into_iter().find(|n| *n == name).expect("name"))
            }
            SpanOutcome::Ends { start, end } => SpanOutcome::Ends {
                start: start.map(|name| NAMES.into_iter().find(|n| *n == name).expect("name")),
                end: end.map(|name| NAMES.into_iter().find(|n| *n == name).expect("name")),
            },
        }
    }

    #[test]
    fn moves_keep_3d_text_unless_they_reverse_the_span_as_excel_measures() {
        // Excel 16.114, tabs S1 to S6.
        // S3 after S6: `'S2:S5'` keeps its text and loses S3; `'S3:S4'` becomes `'S4'`.
        assert_eq!(span_after_move(2, 5, "S2", "S5"), SpanOutcome::Unchanged);
        assert_eq!(
            span_after_move(2, 5, "S3", "S4"),
            SpanOutcome::Collapsed("S4")
        );
        // S6 before S4 and S5 after S6 keep `'S2:S5'`, which takes S6 in.
        assert_eq!(span_after_move(5, 3, "S2", "S5"), SpanOutcome::Unchanged);
        assert_eq!(span_after_move(4, 5, "S2", "S5"), SpanOutcome::Unchanged);
        // S5 before S2 gives `'S2:S4'`; S2 after S5 gives `'S3:S5'` and `'S2:S3'` becomes `'S3'`.
        assert_eq!(
            span_after_move(4, 1, "S2", "S5"),
            SpanOutcome::Ends {
                start: None,
                end: Some("S4")
            }
        );
        assert_eq!(
            span_after_move(1, 4, "S2", "S5"),
            SpanOutcome::Ends {
                start: Some("S3"),
                end: None
            }
        );
        assert_eq!(
            span_after_move(1, 4, "S2", "S3"),
            SpanOutcome::Collapsed("S3")
        );
        // S2 after S3 keeps `'S2:S5'`, which loses S3.
        assert_eq!(span_after_move(1, 2, "S2", "S5"), SpanOutcome::Unchanged);
    }
}
