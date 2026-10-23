//! Structural edits of a worksheet's cell grid and how they move coordinates.
//!
//! Every edit is stated in the coordinates before it. Lines are rows or columns; a line edit
//! inserts, deletes, or moves whole lines along one axis, and a range move cuts a rectangle and
//! pastes it elsewhere on the same sheet.

use crate::{CellAddress, CellRange, Column, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, Row};

/// Rows or columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GridAxis {
    Rows,
    Columns,
}

impl GridAxis {
    /// Returns the last line of the sheet along this axis.
    pub(crate) const fn max(self) -> u32 {
        match self {
            Self::Rows => EXCEL_MAX_ROWS,
            Self::Columns => EXCEL_MAX_COLUMNS,
        }
    }

    /// Returns the coordinate of `address` along this axis.
    pub(crate) const fn of(self, address: CellAddress) -> u32 {
        match self {
            Self::Rows => address.row().get(),
            Self::Columns => address.column().get(),
        }
    }

    /// Returns `address` with its coordinate along this axis replaced.
    fn with(self, address: CellAddress, value: u32) -> Option<CellAddress> {
        match self {
            Self::Rows => Some(CellAddress::new(Row::new(value).ok()?, address.column())),
            Self::Columns => Some(CellAddress::new(address.row(), Column::new(value).ok()?)),
        }
    }
}

/// How a range treats lines inserted right after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeRule {
    /// Formula references, merges, tables, hyperlinks, and filters grow only when lines are
    /// inserted strictly inside them.
    Reference,
    /// Conditional formats and data validations also grow over lines inserted right after their
    /// last line, which inherit the formatting of the line before them.
    Format,
}

/// One structural edit of a worksheet's grid, in the coordinates before the edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GridEdit {
    /// Inserts `count` empty lines before line `at`; later lines move by `count`.
    Insert { axis: GridAxis, at: u32, count: u32 },
    /// Deletes lines `first..=last`; later lines move back.
    Delete {
        axis: GridAxis,
        first: u32,
        last: u32,
    },
    /// Moves lines `first..=last` so they start at `destination` afterwards, as cutting them and
    /// inserting the cut lines does.
    Move {
        axis: GridAxis,
        first: u32,
        last: u32,
        destination: u32,
    },
    /// Moves the cells of `source` so its top-left cell lands at `destination`, overwriting the
    /// cells there.
    MoveRange {
        source: CellRange,
        destination: CellAddress,
    },
}

impl GridEdit {
    /// Returns the axis a line edit moves along, or `None` for a range move.
    pub(crate) const fn line_axis(self) -> Option<GridAxis> {
        match self {
            Self::Insert { axis, .. } | Self::Delete { axis, .. } | Self::Move { axis, .. } => {
                Some(axis)
            }
            Self::MoveRange { .. } => None,
        }
    }

    /// Returns the rectangle a range move pastes into.
    pub(crate) fn destination_range(self) -> Option<CellRange> {
        let Self::MoveRange {
            source,
            destination,
        } = self
        else {
            return None;
        };
        let end = CellAddress::from_indices(
            destination.row().get() + source.height() - 1,
            destination.column().get() + source.width() - 1,
        )
        .ok()?;
        Some(CellRange::from_ordered(destination, end))
    }

    /// Returns the edit that undoes this one.
    pub(crate) fn inverse(self) -> Self {
        match self {
            Self::Insert { axis, at, count } => Self::Delete {
                axis,
                first: at,
                last: at + count - 1,
            },
            Self::Delete { axis, first, last } => Self::Insert {
                axis,
                at: first,
                count: last - first + 1,
            },
            Self::Move {
                axis,
                first,
                last,
                destination,
            } => Self::Move {
                axis,
                first: destination,
                last: destination + (last - first),
                destination: first,
            },
            Self::MoveRange { source, .. } => Self::MoveRange {
                source: self
                    .destination_range()
                    .expect("a validated range move pastes inside the sheet"),
                destination: source.start(),
            },
        }
    }

    /// Maps one line of a line edit's axis; `None` when the edit deletes it or pushes it off the
    /// sheet.
    pub(crate) fn map_line(self, value: u32) -> Option<u32> {
        match self {
            Self::Insert { axis, at, count } => {
                if value < at {
                    Some(value)
                } else {
                    let moved = value.checked_add(count)?;
                    (moved <= axis.max()).then_some(moved)
                }
            }
            Self::Delete { first, last, .. } => {
                if value < first {
                    Some(value)
                } else if value <= last {
                    None
                } else {
                    Some(value - (last - first + 1))
                }
            }
            Self::Move {
                first,
                last,
                destination,
                ..
            } => {
                let count = last - first + 1;
                Some(if (first..=last).contains(&value) {
                    destination + (value - first)
                } else if destination < first && (destination..first).contains(&value) {
                    value + count
                } else if destination > first && (last + 1..destination + count).contains(&value) {
                    value - count
                } else {
                    value
                })
            }
            Self::MoveRange { .. } => Some(value),
        }
    }

    /// Maps one line back to the line it came from; `None` for a line the edit inserted or a
    /// line at the end of the sheet a deletion vacated.
    pub(crate) fn unmap_line(self, value: u32) -> Option<u32> {
        match self {
            Self::Insert { at, count, .. } => {
                if value < at {
                    Some(value)
                } else if value < at + count {
                    None
                } else {
                    Some(value - count)
                }
            }
            Self::Delete { axis, first, last } => {
                if value < first {
                    Some(value)
                } else {
                    let source = value.checked_add(last - first + 1)?;
                    (source <= axis.max()).then_some(source)
                }
            }
            Self::Move { .. } | Self::MoveRange { .. } => self.inverse().map_line(value),
        }
    }

    /// Maps a cell; `None` when the edit deletes or overwrites it.
    pub(crate) fn map_address(self, address: CellAddress) -> Option<CellAddress> {
        match self {
            Self::MoveRange { source, .. } => {
                let destination = self.destination_range()?;
                if source.contains(address) {
                    offset_address(address, self.range_offset())
                } else if destination.contains(address) {
                    None
                } else {
                    Some(address)
                }
            }
            Self::Insert { axis, .. } | Self::Delete { axis, .. } | Self::Move { axis, .. } => {
                axis.with(address, self.map_line(axis.of(address))?)
            }
        }
    }

    /// Maps a cell back to the cell it came from; `None` for a position the edit emptied or
    /// inserted.
    pub(crate) fn unmap_address(self, address: CellAddress) -> Option<CellAddress> {
        match self {
            Self::MoveRange { source, .. } => {
                let destination = self.destination_range()?;
                if destination.contains(address) {
                    let (rows, columns) = self.range_offset();
                    offset_address(address, (-rows, -columns))
                } else if source.contains(address) {
                    None
                } else {
                    Some(address)
                }
            }
            Self::Insert { axis, .. } | Self::Delete { axis, .. } | Self::Move { axis, .. } => {
                axis.with(address, self.unmap_line(axis.of(address))?)
            }
        }
    }

    /// Returns the row and column offset a range move shifts its cells by.
    pub(crate) fn range_offset(self) -> (i64, i64) {
        match self {
            Self::MoveRange {
                source,
                destination,
            } => (
                i64::from(destination.row().get()) - i64::from(source.start().row().get()),
                i64::from(destination.column().get()) - i64::from(source.start().column().get()),
            ),
            Self::Insert { .. } | Self::Delete { .. } | Self::Move { .. } => (0, 0),
        }
    }

    /// Maps the inclusive interval `start..=end` of a line edit's axis by the range rule: lines
    /// inserted strictly inside widen it, a partial deletion narrows it, and each end of a moved
    /// interval follows its line. Returns `None` when the edit deletes the whole interval. An
    /// interval spanning the whole axis, as `A:A` spans the rows, is never changed.
    pub(crate) fn map_interval(self, start: u32, end: u32, rule: RangeRule) -> Option<(u32, u32)> {
        let Some(axis) = self.line_axis() else {
            return Some((start, end));
        };
        if start == 1 && end == axis.max() {
            return Some((start, end));
        }
        match self {
            Self::Insert { at, count, .. } => {
                let grows = end >= at || (rule == RangeRule::Format && end + 1 == at);
                let start = if start >= at { start + count } else { start };
                if start > axis.max() {
                    return None;
                }
                let end = if grows { end + count } else { end };
                Some((start, end.min(axis.max())))
            }
            Self::Delete { first, last, .. } => {
                if start >= first && end <= last {
                    return None;
                }
                let count = last - first + 1;
                let start = if start < first {
                    start
                } else if start <= last {
                    first
                } else {
                    start - count
                };
                let end = if end < first {
                    end
                } else if end <= last {
                    first - 1
                } else {
                    end - count
                };
                Some((start, end))
            }
            Self::Move {
                axis,
                first,
                last,
                destination,
            } => {
                let mapped_start = self.map_line(start)?;
                let mapped_end = self.map_line(end)?;
                if mapped_start <= mapped_end {
                    return Some((mapped_start, mapped_end));
                }
                // An interval whose ends cross shrinks as if the moved lines were deleted, then
                // makes room for them at their destination.
                let (start, end) =
                    Self::Delete { axis, first, last }.map_interval(start, end, rule)?;
                Self::Insert {
                    axis,
                    at: destination,
                    count: last - first + 1,
                }
                .map_interval(start, end, RangeRule::Reference)
            }
            Self::MoveRange { .. } => Some((start, end)),
        }
    }

    /// Maps a range by the range rule. A range move shifts a range that lies wholly inside the
    /// moved cells and leaves every other range where it is.
    pub(crate) fn map_range(self, range: CellRange, rule: RangeRule) -> Option<CellRange> {
        match self {
            Self::MoveRange { source, .. } => {
                if range_contains(source, range) {
                    offset_range(range, self.range_offset())
                } else {
                    Some(range)
                }
            }
            Self::Insert { axis, .. } | Self::Delete { axis, .. } | Self::Move { axis, .. } => {
                let (start, end) =
                    self.map_interval(axis.of(range.start()), axis.of(range.end()), rule)?;
                Some(CellRange::from_ordered(
                    axis.with(range.start(), start)?,
                    axis.with(range.end(), end)?,
                ))
            }
        }
    }

    /// Maps the ranges of a conditional format, data validation, or similar list of cells.
    ///
    /// A line edit maps each range and drops the ones it deletes. A range move splits each range
    /// into the cells that stay and the cells that move with the source, and drops the cells it
    /// overwrites.
    pub(crate) fn map_ranges(self, ranges: &[CellRange], rule: RangeRule) -> Vec<CellRange> {
        match self {
            Self::MoveRange { source, .. } => {
                let Some(destination) = self.destination_range() else {
                    return ranges.to_vec();
                };
                let mut kept = Vec::new();
                let mut moved = Vec::new();
                for range in ranges {
                    for piece in subtract(*range, source) {
                        kept.extend(subtract(piece, destination));
                    }
                    if let Some(inside) = intersect(*range, source)
                        && let Some(shifted) = offset_range(inside, self.range_offset())
                    {
                        moved.push(shifted);
                    }
                }
                kept.extend(moved);
                kept
            }
            Self::Insert { .. } | Self::Delete { .. } | Self::Move { .. } => ranges
                .iter()
                .filter_map(|range| self.map_range(*range, rule))
                .collect(),
        }
    }

    /// Reports whether the edit changes `range` only in part: lines inserted or a moved block
    /// landing strictly inside it, part of it deleted, part of it moved, or part of it cut or
    /// overwritten by a range move. Excel refuses such an edit of a legacy array formula.
    pub(crate) fn splits(self, range: CellRange) -> bool {
        match self {
            Self::MoveRange { source, .. } => {
                let destination = self.destination_range().unwrap_or(source);
                partially_overlaps(range, source)
                    || (!range_contains(source, range) && partially_overlaps(range, destination))
            }
            Self::Insert { axis, at, .. } => {
                let (start, end) = (axis.of(range.start()), axis.of(range.end()));
                start < at && at <= end
            }
            Self::Delete { axis, first, last } => {
                let (start, end) = (axis.of(range.start()), axis.of(range.end()));
                start <= last && first <= end && !(first <= start && end <= last)
            }
            Self::Move {
                axis,
                first,
                last,
                destination,
            } => {
                let (start, end) = (axis.of(range.start()), axis.of(range.end()));
                let block_overlaps = start <= last && first <= end;
                let block_contains = first <= start && end <= last;
                // Where the cut lines are inserted, in the coordinates without them.
                let insertion = if destination > first {
                    destination + (last - first + 1)
                } else {
                    destination
                };
                (block_overlaps && !block_contains)
                    || (!block_contains && start < insertion && insertion <= end)
            }
        }
    }

    /// Reports whether a range move overwrites any cell of `range` it does not also move.
    pub(crate) fn overwrites(self, range: CellRange) -> bool {
        let Self::MoveRange { source, .. } = self else {
            return false;
        };
        let Some(destination) = self.destination_range() else {
            return false;
        };
        intersect(range, destination).is_some_and(|overlap| !subtract(overlap, source).is_empty())
    }
}

/// The structural edits a draft applied to one source worksheet, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GridHistory {
    edits: Vec<GridEdit>,
}

impl GridHistory {
    pub(crate) fn push(&mut self, edit: GridEdit) {
        self.edits.push(edit);
    }

    pub(crate) fn edits(&self) -> &[GridEdit] {
        &self.edits
    }

    /// Maps a current cell back to the source cell it came from; `None` for a position the edits
    /// inserted or emptied.
    pub(crate) fn to_source(&self, address: CellAddress) -> Option<CellAddress> {
        self.edits
            .iter()
            .rev()
            .try_fold(address, |address, edit| edit.unmap_address(address))
    }
}

/// Returns the overlap of two ranges.
pub(crate) fn intersect(left: CellRange, right: CellRange) -> Option<CellRange> {
    let start = CellAddress::from_indices(
        left.start().row().get().max(right.start().row().get()),
        left.start()
            .column()
            .get()
            .max(right.start().column().get()),
    )
    .ok()?;
    let end = CellAddress::from_indices(
        left.end().row().get().min(right.end().row().get()),
        left.end().column().get().min(right.end().column().get()),
    )
    .ok()?;
    CellRange::new(start, end).ok()
}

/// Returns the cells of `range` outside `removed`, as up to four disjoint ranges.
pub(crate) fn subtract(range: CellRange, removed: CellRange) -> Vec<CellRange> {
    let Some(overlap) = intersect(range, removed) else {
        return vec![range];
    };
    let (top, bottom) = (range.start().row().get(), range.end().row().get());
    let (left, right) = (range.start().column().get(), range.end().column().get());
    let (overlap_top, overlap_bottom) = (overlap.start().row().get(), overlap.end().row().get());
    let (overlap_left, overlap_right) =
        (overlap.start().column().get(), overlap.end().column().get());
    let piece = |first_row: u32, last_row: u32, first_column: u32, last_column: u32| {
        let start = CellAddress::from_indices(first_row, first_column).ok()?;
        let end = CellAddress::from_indices(last_row, last_column).ok()?;
        CellRange::new(start, end).ok()
    };
    [
        (top < overlap_top).then(|| piece(top, overlap_top - 1, left, right)),
        (left < overlap_left).then(|| piece(overlap_top, overlap_bottom, left, overlap_left - 1)),
        (overlap_right < right)
            .then(|| piece(overlap_top, overlap_bottom, overlap_right + 1, right)),
        (overlap_bottom < bottom).then(|| piece(overlap_bottom + 1, bottom, left, right)),
    ]
    .into_iter()
    .flatten()
    .flatten()
    .collect()
}

/// Reports whether `outer` contains every cell of `inner`.
pub(crate) fn range_contains(outer: CellRange, inner: CellRange) -> bool {
    outer.contains(inner.start()) && outer.contains(inner.end())
}

/// Reports whether two ranges share cells without the first lying wholly inside the second.
pub(crate) fn partially_overlaps(range: CellRange, other: CellRange) -> bool {
    intersect(range, other).is_some() && !range_contains(other, range)
}

fn offset_address(address: CellAddress, (rows, columns): (i64, i64)) -> Option<CellAddress> {
    let row = u32::try_from(i64::from(address.row().get()) + rows).ok()?;
    let column = u32::try_from(i64::from(address.column().get()) + columns).ok()?;
    CellAddress::from_indices(row, column).ok()
}

fn offset_range(range: CellRange, offset: (i64, i64)) -> Option<CellRange> {
    Some(CellRange::from_ordered(
        offset_address(range.start(), offset)?,
        offset_address(range.end(), offset)?,
    ))
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
