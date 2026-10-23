//! The areas a dependency graph builds for the multi-cell ranges formulas read.
//!
//! A range is split along its longer axis into aligned blocks of 2^k rows (or columns), the way a
//! segment tree splits an interval, and a block holding many cells is split again into its two
//! halves. Every range over the same columns (or rows) shares these blocks, so a running total
//! `SUM($B$2:Bn)` adds a few edges per row instead of one per formula above it.

use std::collections::HashMap;

use super::Engine;
use crate::calculation::graph::{DependencyGraph, DependencyNode};
use crate::calculation::runtime::{CellId, Rect};

/// A block holding at most this many cells and array anchors keeps them directly.
const LEAF_MEMBERS: usize = 16;

/// The axis a range is split along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Axis {
    Rows,
    Columns,
}

/// The cells of one sheet between `start` and `end` across the split axis, which every block of
/// a range shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Band {
    sheet: usize,
    axis: Axis,
    start: u32,
    end: u32,
}

impl Band {
    fn of(rect: &Rect) -> (Self, u32, u32) {
        if rect.height() >= rect.width() {
            (
                Self {
                    sheet: rect.sheet,
                    axis: Axis::Rows,
                    start: rect.col_start,
                    end: rect.col_end,
                },
                rect.row_start,
                rect.row_end,
            )
        } else {
            (
                Self {
                    sheet: rect.sheet,
                    axis: Axis::Columns,
                    start: rect.row_start,
                    end: rect.row_end,
                },
                rect.col_start,
                rect.col_end,
            )
        }
    }

    fn rect(self, first: u32, last: u32) -> Rect {
        match self.axis {
            Axis::Rows => Rect {
                sheet: self.sheet,
                row_start: first,
                col_start: self.start,
                row_end: last,
                col_end: self.end,
                whole_rows: false,
            },
            Axis::Columns => Rect {
                sheet: self.sheet,
                row_start: self.start,
                col_start: first,
                row_end: self.end,
                col_end: last,
                whole_rows: false,
            },
        }
    }

    const fn position(self, row: u32, column: u32) -> u32 {
        match self.axis {
            Axis::Rows => row,
            Axis::Columns => column,
        }
    }
}

/// The aligned block of `2^level` positions of a band starting at `index << level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Block {
    band: Band,
    level: u32,
    index: u32,
}

impl Block {
    const fn first(self) -> u32 {
        self.index << self.level
    }

    const fn last(self) -> u32 {
        self.first() + ((1 << self.level) - 1)
    }

    /// The first position of the upper half.
    const fn middle(self) -> u32 {
        self.first() + (1 << (self.level - 1))
    }

    const fn half(self, upper: bool) -> Self {
        Self {
            band: self.band,
            level: self.level - 1,
            index: self.index * 2 + upper as u32,
        }
    }
}

/// A formula cell at `position` along a band.
#[derive(Debug, Clone, Copy)]
struct Member {
    position: u32,
    cell: CellId,
}

/// An array formula whose result spans `first..=last` along a band; reading any of those cells
/// waits for its anchor.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    first: u32,
    last: u32,
    cell: CellId,
}

/// The aligned blocks covering `first..=last`, largest first where alignment allows.
fn blocks(band: Band, first: u32, last: u32) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut position = u64::from(first);
    let last = u64::from(last);
    while position <= last {
        let mut level = position.trailing_zeros().min(31);
        while position + (1 << level) - 1 > last {
            level -= 1;
        }
        blocks.push(Block {
            band,
            level,
            index: (position >> level) as u32,
        });
        position += 1 << level;
    }
    blocks
}

/// The nodes built so far for ranges and blocks, so later ranges reuse them.
#[derive(Debug, Default)]
pub(super) struct AreaNodes {
    ranges: HashMap<(usize, u32, u32, u32, u32), Option<DependencyNode>>,
    blocks: HashMap<Block, Option<DependencyNode>>,
}

impl AreaNodes {
    /// Returns the node a formula reading the multi-cell `rect` waits for, or `None` when no
    /// formula cell or array result lies in it.
    ///
    /// The node covers every formula cell inside `rect` and the anchor of every array result
    /// that intersects it.
    pub(super) fn range(
        &mut self,
        engine: &Engine<'_>,
        graph: &mut DependencyGraph,
        rect: &Rect,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<DependencyNode>, ()> {
        let key = (
            rect.sheet,
            rect.row_start,
            rect.col_start,
            rect.row_end,
            rect.col_end,
        );
        if let Some(node) = self.ranges.get(&key) {
            return Ok(*node);
        }
        let (band, first, last) = Band::of(rect);
        let mut parts = Vec::new();
        for block in blocks(band, first, last) {
            if cancelled() {
                return Err(());
            }
            let node = match self.blocks.get(&block) {
                Some(node) => *node,
                None => self.build_block(engine, graph, block, cancelled)?,
            };
            parts.extend(node);
        }
        parts.sort_unstable();
        parts.dedup();
        let node = match parts.len() {
            0 => None,
            1 => Some(parts[0]),
            _ => Some(graph.add_area(parts)),
        };
        self.ranges.insert(key, node);
        Ok(node)
    }

    fn build_block(
        &mut self,
        engine: &Engine<'_>,
        graph: &mut DependencyGraph,
        block: Block,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<DependencyNode>, ()> {
        let rect = block.band.rect(block.first(), block.last());
        let mut members = Vec::new();
        for (row, column) in engine.formula_cells_within(&rect) {
            if cancelled() {
                return Err(());
            }
            members.push(Member {
                position: block.band.position(row, column),
                cell: (rect.sheet, row, column),
            });
        }
        members.sort_unstable_by_key(|member| (member.position, member.cell));
        let mut anchors = Vec::new();
        for region in &engine.array_regions {
            if cancelled() {
                return Err(());
            }
            if super::dependency::rects_intersect(&region.rect, &rect) {
                let (first, last) = match block.band.axis {
                    Axis::Rows => (region.rect.row_start, region.rect.row_end),
                    Axis::Columns => (region.rect.col_start, region.rect.col_end),
                };
                anchors.push(Anchor {
                    first,
                    last,
                    cell: region.anchor,
                });
            }
        }
        self.build(graph, block, &members, &anchors, cancelled)
    }

    fn build(
        &mut self,
        graph: &mut DependencyGraph,
        block: Block,
        members: &[Member],
        anchors: &[Anchor],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<DependencyNode>, ()> {
        if let Some(node) = self.blocks.get(&block) {
            return Ok(*node);
        }
        if cancelled() {
            return Err(());
        }
        let node = if block.level == 0 || members.len() + anchors.len() <= LEAF_MEMBERS {
            let mut cells = members
                .iter()
                .map(|member| member.cell)
                .chain(anchors.iter().map(|anchor| anchor.cell))
                .map(DependencyNode::Cell)
                .collect::<Vec<_>>();
            cells.sort_unstable();
            cells.dedup();
            match cells.len() {
                0 => None,
                1 => Some(cells[0]),
                _ => Some(graph.add_area(cells)),
            }
        } else {
            let middle = block.middle();
            let split = members.partition_point(|member| member.position < middle);
            let lower_anchors = anchors
                .iter()
                .filter(|anchor| anchor.first < middle)
                .copied()
                .collect::<Vec<_>>();
            let upper_anchors = anchors
                .iter()
                .filter(|anchor| anchor.last >= middle)
                .copied()
                .collect::<Vec<_>>();
            let lower = self.build(
                graph,
                block.half(false),
                &members[..split],
                &lower_anchors,
                cancelled,
            )?;
            let upper = self.build(
                graph,
                block.half(true),
                &members[split..],
                &upper_anchors,
                cancelled,
            )?;
            match (lower, upper) {
                (Some(lower), Some(upper)) if lower != upper => {
                    Some(graph.add_area(vec![lower, upper]))
                }
                (Some(node), _) | (None, Some(node)) => Some(node),
                (None, None) => None,
            }
        };
        self.blocks.insert(block, node);
        Ok(node)
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis, Band, blocks};

    #[test]
    fn aligned_blocks_cover_a_range_exactly_once() {
        let band = Band {
            sheet: 0,
            axis: Axis::Rows,
            start: 2,
            end: 2,
        };
        for (first, last) in [
            (1, 1),
            (2, 1_000),
            (7, 8),
            (1, 1_048_576),
            (1_048_576, 1_048_576),
        ] {
            let covered = blocks(band, first, last)
                .into_iter()
                .flat_map(|block| {
                    assert_eq!(block.first() % (1 << block.level), 0);
                    block.first()..=block.last()
                })
                .collect::<Vec<_>>();
            assert_eq!(covered, (first..=last).collect::<Vec<_>>());
        }
    }
}
