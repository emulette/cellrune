//! Moves a sheet's cells and grid-bound structures through a structural edit.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{CellStore, Sheet};
use crate::grid::{GridAxis, GridEdit, RangeRule};
use crate::{
    Cell, CellAddress, CellContent, CellRange, FormulaCell, FormulaMetadata, FormulaText,
    SharedFormulaRole,
};

impl Sheet {
    /// Moves every cell through `edit`, dropping the cells it deletes or overwrites, and gives
    /// each formula listed in `rewritten` (by its address before the edit) its rewritten text.
    /// Merged ranges, the auto-filter, hidden rows, hyperlinks, comments, and the styles new
    /// cells receive move with the cells; lines an insertion adds take the styles of the line
    /// before them. Tables and data validations are left to the caller.
    ///
    /// Returns the addresses, after the edit, of the formulas whose text or metadata changed.
    pub(crate) fn apply_grid_edit(
        &mut self,
        edit: GridEdit,
        rewritten: &BTreeMap<CellAddress, FormulaText>,
    ) -> BTreeSet<CellAddress> {
        let inherited = self.inherited_styles(edit);
        let previous = std::mem::take(&mut self.cells);
        self.formula_addresses = Arc::new(BTreeSet::new());
        self.array_formula_ranges = Arc::new(BTreeMap::new());
        self.column_max_rows = Arc::new(BTreeMap::new());
        self.min_row = None;
        self.min_column = None;
        self.max_row = None;
        self.max_column = None;
        self.bounds_dirty = false;
        let mut cells = BTreeMap::new();
        let mut changed = BTreeSet::new();
        for cell in previous.values() {
            let Some(address) = edit.map_address(cell.address()) else {
                continue;
            };
            let moved = match cell.content() {
                CellContent::Formula(formula) => {
                    let text = rewritten
                        .get(&cell.address())
                        .cloned()
                        .or_else(|| formula.text().cloned());
                    let updated = FormulaCell::from_xlsx_parts(
                        formula.dialect(),
                        text,
                        formula.saved_result().clone(),
                        grid_metadata(formula.metadata(), edit, address),
                        formula.recalculate_always(),
                    );
                    if updated == *formula {
                        cell.moved_to(address)
                    } else {
                        changed.insert(address);
                        Cell::with_content_and_number_format(
                            address,
                            CellContent::Formula(updated),
                            cell.number_format().clone(),
                        )
                    }
                }
                CellContent::Literal(_) => cell.moved_to(address),
            };
            self.track_formula_metadata(address, moved.content());
            self.update_bounds(address);
            self.update_column_extent(address);
            cells.insert(CellStore::key(address), moved);
        }
        self.cells = CellStore::from_sorted(cells);

        let mut merged_ranges = self
            .merged_ranges
            .iter()
            .filter(|range| !edit.overwrites(**range))
            .filter_map(|range| edit.map_range(*range, RangeRule::Reference))
            .filter(|range| range.height() > 1 || range.width() > 1)
            .collect::<Vec<_>>();
        merged_ranges.sort_by_key(|range| (range.start(), range.end()));
        self.merged_ranges = Arc::new(merged_ranges);
        self.auto_filter = self
            .auto_filter
            .as_ref()
            .and_then(|filter| filter.grid_mapped(edit));
        if edit.line_axis() == Some(GridAxis::Rows) {
            let mut hidden_rows = self
                .hidden_rows
                .iter()
                .filter_map(|row| crate::Row::new(edit.map_line(row.get())?).ok())
                .collect::<Vec<_>>();
            hidden_rows.sort_unstable();
            self.hidden_rows = Arc::new(hidden_rows);
        }
        self.hyperlinks = Arc::new(
            self.hyperlinks
                .iter()
                .filter(|hyperlink| !edit.overwrites(hyperlink.range()))
                .filter_map(|hyperlink| {
                    Some(
                        hyperlink
                            .with_range(edit.map_range(hyperlink.range(), RangeRule::Reference)?),
                    )
                })
                .collect(),
        );
        self.comments = Arc::new(
            self.comments
                .iter()
                .filter_map(|comment| Some(comment.moved_to(edit.map_address(comment.address())?)))
                .collect(),
        );
        Arc::make_mut(&mut self.new_cell_styles).apply_grid_edit(edit, inherited);
        self.semantic_fingerprint = std::sync::OnceLock::new();
        changed
    }

    /// Returns the cell styles of the line before an insertion at the positions of the inserted
    /// lines.
    fn inherited_styles(&self, edit: GridEdit) -> Vec<(CellAddress, usize, crate::NumberFormat)> {
        let GridEdit::Insert { axis, at, count } = edit else {
            return Vec::new();
        };
        if at == 1 {
            return Vec::new();
        }
        let styles = self.new_cell_styles.line_styles(axis, at - 1, |address| {
            self.cell(address).map(|cell| cell.number_format().clone())
        });
        let mut inherited = Vec::new();
        for (address, index, number_format) in styles {
            for line in at..at + count {
                let target = match axis {
                    GridAxis::Rows => crate::Row::new(line)
                        .ok()
                        .map(|row| CellAddress::new(row, address.column())),
                    GridAxis::Columns => crate::Column::new(line)
                        .ok()
                        .map(|column| CellAddress::new(address.row(), column)),
                };
                if let Some(target) = target {
                    inherited.push((target, index, number_format.clone()));
                }
            }
        }
        inherited
    }

    /// Turns every shared formula group with a member in `touched` that no longer reads back as
    /// a group into ordinary formulas, and returns the addresses whose metadata changed.
    ///
    /// A group reads back only when its anchor is the first of its cells, its range holds every
    /// member, and shifting the anchor's text to each follower gives that follower's text.
    pub(crate) fn normalize_shared_formula_groups(
        &mut self,
        touched: &BTreeSet<CellAddress>,
    ) -> BTreeSet<CellAddress> {
        let mut anchors =
            BTreeMap::<(u32, CellAddress), (Option<CellRange>, Option<String>)>::new();
        let mut followers =
            BTreeMap::<(u32, CellAddress), Vec<(CellAddress, Option<String>)>>::new();
        for address in self.formula_addresses.iter() {
            let Some(CellContent::Formula(formula)) = self.cell(*address).map(Cell::content) else {
                continue;
            };
            let text = formula.text().map(|text| text.as_str().to_owned());
            match formula.metadata() {
                FormulaMetadata::Shared {
                    group_index,
                    role: SharedFormulaRole::Anchor,
                    range,
                } => {
                    anchors.insert((*group_index, *address), (*range, text));
                }
                FormulaMetadata::Shared {
                    group_index,
                    role: SharedFormulaRole::Follower { anchor },
                    ..
                } => followers
                    .entry((*group_index, *anchor))
                    .or_default()
                    .push((*address, text)),
                _ => {}
            }
        }
        let mut detached = BTreeSet::new();
        let keys = anchors
            .keys()
            .chain(followers.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        for key in keys {
            let members = followers.get(&key).map(Vec::as_slice).unwrap_or_default();
            let (group_index, anchor) = key;
            let anchor_entry = anchors.get(&key);
            // A group whose master the edit deleted keeps followers that name it, whose own
            // text the edit may have left as it was.
            let is_touched = anchor_entry.is_none()
                || touched.contains(&anchor)
                || members.iter().any(|(address, _)| touched.contains(address));
            if !is_touched {
                continue;
            }
            let consistent = anchor_entry.is_some_and(|(range, text)| {
                range.is_some_and(|range| {
                    range.contains(anchor)
                        && text.as_deref().is_some_and(|anchor_text| {
                            members.iter().all(|(address, follower_text)| {
                                range.contains(*address)
                                    && (address.row(), address.column())
                                        > (anchor.row(), anchor.column())
                                    && crate::xlsx::shift_shared_formula(
                                        anchor_text,
                                        anchor,
                                        *address,
                                    )
                                    .ok()
                                    .as_deref()
                                        == follower_text.as_deref()
                            })
                        })
                })
            });
            if consistent {
                continue;
            }
            let addresses = anchor_entry
                .map(|_| anchor)
                .into_iter()
                .chain(members.iter().map(|(address, _)| *address));
            for address in addresses {
                let Some(cell) = self.cell(address).cloned() else {
                    continue;
                };
                let CellContent::Formula(formula) = cell.content() else {
                    continue;
                };
                if !matches!(
                    formula.metadata(),
                    FormulaMetadata::Shared { group_index: index, .. } if *index == group_index
                ) {
                    continue;
                }
                let ordinary = FormulaCell::from_xlsx_parts(
                    formula.dialect(),
                    formula.text().cloned(),
                    formula.saved_result().clone(),
                    FormulaMetadata::Normal,
                    formula.recalculate_always(),
                );
                self.upsert_cell_deferred(
                    address,
                    CellContent::Formula(ordinary),
                    cell.number_format().clone(),
                );
                detached.insert(address);
            }
        }
        detached
    }
}

/// Moves a formula's container metadata, now at `address`, through a structural edit.
pub(crate) fn grid_metadata(
    metadata: &FormulaMetadata,
    edit: GridEdit,
    address: CellAddress,
) -> FormulaMetadata {
    let range_at = |range: CellRange| {
        edit.map_range(range, RangeRule::Reference)
            .filter(|mapped| mapped.start() == address)
            .or_else(|| translated(range, address))
    };
    match metadata {
        FormulaMetadata::Normal => FormulaMetadata::Normal,
        FormulaMetadata::Shared {
            group_index,
            role: SharedFormulaRole::Anchor,
            range,
        } => FormulaMetadata::Shared {
            group_index: *group_index,
            role: SharedFormulaRole::Anchor,
            range: range.and_then(|range| edit.map_range(range, RangeRule::Reference)),
        },
        FormulaMetadata::Shared {
            group_index,
            role: SharedFormulaRole::Follower { anchor },
            range,
        } => FormulaMetadata::Shared {
            group_index: *group_index,
            role: SharedFormulaRole::Follower {
                anchor: edit.map_address(*anchor).unwrap_or(*anchor),
            },
            range: *range,
        },
        FormulaMetadata::Array {
            range,
            always_calculate,
        } => FormulaMetadata::Array {
            range: range_at(*range).unwrap_or(*range),
            always_calculate: *always_calculate,
        },
        FormulaMetadata::DynamicArray {
            range,
            always_calculate,
        } => FormulaMetadata::DynamicArray {
            range: range.and_then(range_at),
            always_calculate: *always_calculate,
        },
        FormulaMetadata::DataTable {
            range,
            input_cell_1,
            input_cell_2,
            two_dimensional,
            row_oriented,
            input_cell_1_deleted,
            input_cell_2_deleted,
        } => {
            let input = |cell: Option<CellAddress>, deleted: bool| match cell {
                Some(cell) => match edit.map_address(cell) {
                    Some(moved) => (Some(moved), deleted),
                    None => (Some(cell), true),
                },
                None => (None, deleted),
            };
            let (input_cell_1, input_cell_1_deleted) = input(*input_cell_1, *input_cell_1_deleted);
            let (input_cell_2, input_cell_2_deleted) = input(*input_cell_2, *input_cell_2_deleted);
            FormulaMetadata::DataTable {
                range: range_at(*range).unwrap_or(*range),
                input_cell_1,
                input_cell_2,
                two_dimensional: *two_dimensional,
                row_oriented: *row_oriented,
                input_cell_1_deleted,
                input_cell_2_deleted,
            }
        }
    }
}

/// Returns `range` moved so that it starts at `start`, when it still fits on the sheet.
fn translated(range: CellRange, start: CellAddress) -> Option<CellRange> {
    let end = CellAddress::from_indices(
        start.row().get() + range.height() - 1,
        start.column().get() + range.width() - 1,
    )
    .ok()?;
    Some(CellRange::from_ordered(start, end))
}
