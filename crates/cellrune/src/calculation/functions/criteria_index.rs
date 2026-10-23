//! Equality indexes shared by conditional scans over one criteria range within a calculation pass.
//!
//! SUMIFS, COUNTIFS, and their siblings read their criteria ranges row by row and stop each row at
//! its first unmatched criterion. Once calls have scanned the same ranges twice, one read of the
//! populated rows of each builds an index of the cells holding each value, and later calls whose
//! criteria are exact values find their matching rows by intersecting those lists. An indexed call
//! is charged the cell reads and work units its scan would have charged. Calls whose criteria an
//! index cannot answer, or whose charges could reach a limit, keep scanning without an index, so
//! results and limit failures do not change. An index is dropped when a result inside its range
//! changes.

use std::cell::RefMut;
use std::collections::HashMap;
use std::sync::Arc;

use super::super::criteria::{CompiledCriteria, ExactCriterion};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::{CellId, Rect};
use super::super::value::{ErrorKind, Value};
use super::lookup_index::{EqualityKey, LookupIndexCache, lowercase, number_key};
use crate::CalculationLimitKind;

/// Ranges with fewer cells than this are scanned; an index would not pay for itself.
const MIN_INDEXED_CELLS: u64 = 32;
/// Calls that scan a range before it is indexed.
const SCANS_BEFORE_INDEXING: u32 = 2;
/// Most recently used indexes kept at once.
const MAX_INDEXES: usize = 16;
/// Cells covered by all kept indexes together.
const MAX_INDEXED_CELLS: u64 = 2_000_000;
const CANCELLATION_POLL_INTERVAL: u32 = 4_096;

/// The first `rows` rows of a criteria range `width` columns wide, read row by row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::calculation) struct CriteriaArea {
    sheet: usize,
    row: u32,
    column: u32,
    rows: u32,
    width: u32,
}

impl CriteriaArea {
    fn new(rect: &Rect, rows: u64, width: u64) -> Option<Self> {
        Some(Self {
            sheet: rect.sheet,
            row: rect.row_start,
            column: rect.col_start,
            rows: u32::try_from(rows).ok()?,
            width: u32::try_from(width).ok()?,
        })
    }

    fn cells(self) -> u64 {
        u64::from(self.rows) * u64::from(self.width)
    }

    fn cell(self, offset: u32) -> CellId {
        (
            self.sheet,
            self.row + offset / self.width,
            self.column + offset % self.width,
        )
    }

    fn last_row(self) -> u32 {
        self.row + self.rows.saturating_sub(1)
    }

    fn last_column(self) -> u32 {
        self.column + self.width.saturating_sub(1)
    }

    fn rect(self) -> Rect {
        Rect {
            sheet: self.sheet,
            row_start: self.row,
            col_start: self.column,
            row_end: self.last_row(),
            col_end: self.last_column(),
            whole_rows: false,
        }
    }

    fn contains(self, cell: CellId) -> bool {
        cell.0 == self.sheet
            && (self.row..=self.last_row()).contains(&cell.1)
            && (self.column..=self.last_column()).contains(&cell.2)
    }

    fn intersects(self, rect: &Rect) -> bool {
        rect.sheet == self.sheet
            && rect.row_start <= self.last_row()
            && self.row <= rect.row_end
            && rect.col_start <= self.last_column()
            && self.column <= rect.col_end
    }
}

/// Criteria indexes built during one calculation, most recently used last.
#[derive(Debug, Default)]
pub(in crate::calculation) struct CriteriaIndexCache {
    /// Calls that scanned each range since it was last indexed or changed.
    scans: HashMap<CriteriaArea, u32>,
    indexes: Vec<(CriteriaArea, Arc<CriteriaIndex>)>,
}

impl CriteriaIndexCache {
    pub(in crate::calculation) fn clear(&mut self) {
        self.scans.clear();
        self.indexes.clear();
    }

    pub(in crate::calculation) fn is_empty(&self) -> bool {
        self.indexes.is_empty()
    }

    /// Drops indexes whose range holds `cell`, whose result is about to change.
    pub(in crate::calculation) fn invalidate_cell(&mut self, cell: CellId) {
        let scans = &mut self.scans;
        self.indexes.retain(|(area, _)| {
            let keep = !area.contains(cell);
            if !keep {
                scans.insert(*area, 0);
            }
            keep
        });
    }

    /// Drops indexes whose range meets `rect`, whose cells are about to read a different owner.
    pub(in crate::calculation) fn invalidate_rect(&mut self, rect: &Rect) {
        let scans = &mut self.scans;
        self.indexes.retain(|(area, _)| {
            let keep = !area.intersects(rect);
            if !keep {
                scans.insert(*area, 0);
            }
            keep
        });
    }

    fn get(&mut self, area: CriteriaArea) -> Option<Arc<CriteriaIndex>> {
        let position = self
            .indexes
            .iter()
            .position(|(candidate, _)| *candidate == area)?;
        let entry = self.indexes.remove(position);
        let index = Arc::clone(&entry.1);
        self.indexes.push(entry);
        Some(index)
    }

    /// Records one more call that scans `area`, and reports whether enough have to index it.
    fn record_scan(&mut self, area: CriteriaArea) -> bool {
        let scans = self.scans.entry(area).or_insert(0);
        *scans = scans.saturating_add(1);
        *scans > SCANS_BEFORE_INDEXING
    }

    fn insert(&mut self, area: CriteriaArea, index: Arc<CriteriaIndex>) {
        let mut cells = area.cells();
        for (kept, _) in &self.indexes {
            cells += kept.cells();
        }
        while !self.indexes.is_empty()
            && (self.indexes.len() >= MAX_INDEXES || cells > MAX_INDEXED_CELLS)
        {
            let (evicted, _) = self.indexes.remove(0);
            cells -= evicted.cells();
            self.scans.insert(evicted, 0);
        }
        self.indexes.push((area, index));
    }
}

/// What an index groups a cell under: its value, or for a text cell also the number its trimmed
/// text parses as.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IndexKey {
    Value(EqualityKey),
    NumericText(u64),
}

/// The cells of one criteria range grouped by value, as offsets in reading order.
#[derive(Debug)]
pub(in crate::calculation) struct CriteriaIndex {
    keys: HashMap<IndexKey, u32>,
    /// `offsets[starts[key]..starts[key + 1]]` are the ascending offsets grouped under `key`.
    starts: Vec<u32>,
    offsets: Vec<u32>,
    texts: Bits,
    nonempty_texts: Bits,
}

impl CriteriaIndex {
    fn build(engine: &Engine<'_>, context: EvalContext<'_>, area: CriteriaArea) -> Option<Self> {
        // Every cell below the last populated row of the area's columns is blank, which no exact
        // criterion matches, so the index reads only the rows above it.
        let populated_rows = engine
            .populated_row_end(&area.rect())
            .saturating_add(1)
            .saturating_sub(area.row)
            .min(area.rows);
        let cells = u32::try_from(u64::from(populated_rows) * u64::from(area.width)).ok()?;
        let mut keys = HashMap::new();
        let mut entries = Vec::with_capacity(cells as usize);
        let mut texts = Bits::new(cells);
        let mut nonempty_texts = Bits::new(cells);
        let mut key_id = |key: IndexKey| {
            let next = keys.len() as u32;
            *keys.entry(key).or_insert(next)
        };
        for offset in 0..cells {
            if offset % CANCELLATION_POLL_INTERVAL == 0 && context.is_cancelled() {
                return None;
            }
            let key = match engine.cell_value(area.cell(offset)) {
                Value::Blank | Value::Error(_) => continue,
                Value::Number(number) => match number_key(number) {
                    Some(key) => EqualityKey::Number(key),
                    None => continue,
                },
                Value::Logical(logical) => EqualityKey::Logical(logical),
                Value::Text(text) => {
                    texts.set(offset);
                    if !text.is_empty() {
                        nonempty_texts.set(offset);
                    }
                    if let Some(key) = text.trim().parse::<f64>().ok().and_then(number_key) {
                        entries.push((key_id(IndexKey::NumericText(key)), offset));
                    }
                    EqualityKey::Text(lowercase(&text))
                }
            };
            entries.push((key_id(IndexKey::Value(key)), offset));
        }
        let mut starts = vec![0_u32; keys.len() + 1];
        for (key, _) in &entries {
            starts[*key as usize + 1] += 1;
        }
        for key in 0..keys.len() {
            starts[key + 1] += starts[key];
        }
        let mut next = starts.clone();
        let mut offsets = vec![0_u32; entries.len()];
        for (key, offset) in entries {
            let slot = &mut next[key as usize];
            offsets[*slot as usize] = offset;
            *slot += 1;
        }
        Some(Self {
            keys,
            starts,
            offsets,
            texts,
            nonempty_texts,
        })
    }

    fn group(&self, key: &IndexKey) -> &[u32] {
        self.keys.get(key).map_or(&[], |key| {
            &self.offsets
                [self.starts[*key as usize] as usize..self.starts[*key as usize + 1] as usize]
        })
    }

    /// The ascending offsets `criterion` matches, and the cells among them that cost one work
    /// unit to test.
    fn matching(&self, criterion: &ExactCriterion) -> (Vec<u32>, Option<&Bits>) {
        match criterion {
            ExactCriterion::Number(number) => {
                let Some(key) = number_key(*number) else {
                    return (Vec::new(), Some(&self.texts));
                };
                (
                    merge(
                        self.group(&IndexKey::Value(EqualityKey::Number(key))),
                        self.group(&IndexKey::NumericText(key)),
                    ),
                    Some(&self.texts),
                )
            }
            ExactCriterion::Text(text) => (
                self.group(&IndexKey::Value(EqualityKey::Text(text.as_str().into())))
                    .to_vec(),
                Some(&self.nonempty_texts),
            ),
            ExactCriterion::Logical(logical) => (
                self.group(&IndexKey::Value(EqualityKey::Logical(*logical)))
                    .to_vec(),
                None,
            ),
        }
    }
}

/// One bit per cell offset; offsets past the bits are unset.
#[derive(Debug)]
struct Bits {
    words: Vec<u64>,
    count: u64,
}

impl Bits {
    fn new(cells: u32) -> Self {
        Self {
            words: vec![0; (cells as usize).div_ceil(64)],
            count: 0,
        }
    }

    fn set(&mut self, offset: u32) {
        self.words[offset as usize / 64] |= 1 << (offset % 64);
        self.count += 1;
    }

    fn contains(&self, offset: u32) -> bool {
        self.words
            .get(offset as usize / 64)
            .is_some_and(|word| word & (1 << (offset % 64)) != 0)
    }
}

/// The offsets every criterion of a conditional call matches, found through indexes, with what
/// its scan of the criteria ranges would have charged.
#[derive(Debug)]
pub(super) struct IndexedMatches {
    /// Ascending offsets, row by row, of the cells every criterion matches.
    pub(super) offsets: Vec<u32>,
    reads: u64,
    work: u64,
}

impl IndexedMatches {
    /// Charges what the scan would have: its criteria reads and work, and `value_reads` more cell
    /// reads. Returns `None` when that could reach a limit, so that the caller scans and reports
    /// the limit it reaches first.
    pub(super) fn charge(
        &self,
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        value_reads: u64,
    ) -> Option<Result<(), ErrorKind>> {
        let reads = self.reads.checked_add(value_reads)?;
        if engine.remaining_reference_cells(context) < reads
            || engine.remaining_function_iterations(context) < self.work
        {
            return None;
        }
        if context.is_cancelled() {
            return Some(Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            )));
        }
        Some(
            engine
                .charge_reference_cells(context, reads)
                .and_then(|()| engine.charge_function_iterations(context, self.work)),
        )
    }
}

/// Finds the cells a conditional call over `rows` by `width` cells of each criteria range matches,
/// or returns `None` when the call must scan: during targeted calculation, whose cell reads record
/// pending dependencies, for small ranges, for ranges larger than the reference cells the call may
/// still read, for a criterion other than an exact value, and for ranges calls have not yet
/// scanned often enough to index.
pub(super) fn indexed_matches<'criteria>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    criteria: impl IntoIterator<Item = (&'criteria Rect, &'criteria CompiledCriteria)>,
    rows: u64,
    width: u64,
) -> Option<IndexedMatches> {
    let cells = rows.checked_mul(width)?;
    // An indexed call is charged every cell of its first criteria range, so a call whose ranges
    // exceed the remaining budget scans and reports the limit; an index would read them for nothing.
    if engine.is_targeted()
        || cells < MIN_INDEXED_CELLS
        || engine.remaining_reference_cells(context) < cells
    {
        return None;
    }
    let criteria = criteria
        .into_iter()
        .map(|(rect, criterion)| {
            Some((
                CriteriaArea::new(rect, rows, width)?,
                criterion.exact_value()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let indexes = criteria_indexes(engine, context, criteria.iter().map(|(area, _)| *area))?;
    let mut matched = None::<Vec<u32>>;
    let mut reads = 0_u64;
    let mut work = 0_u64;
    for ((_, criterion), index) in criteria.iter().zip(&indexes) {
        let (matching, costly) = index.matching(criterion);
        match &matched {
            None => {
                reads += cells;
                work += costly.map_or(0, |bits| bits.count);
                matched = Some(matching);
            }
            Some(scanned) => {
                reads += scanned.len() as u64;
                work += costly.map_or(0, |bits| {
                    scanned
                        .iter()
                        .filter(|offset| bits.contains(**offset))
                        .count() as u64
                });
                matched = Some(intersect(scanned, &matching));
            }
        }
    }
    Some(IndexedMatches {
        offsets: matched?,
        reads,
        work,
    })
}

/// Returns the index of every area, building those not yet kept once calls have scanned the first
/// area often enough, or `None` when the call must scan.
fn criteria_indexes(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    areas: impl Iterator<Item = CriteriaArea>,
) -> Option<Vec<Arc<CriteriaIndex>>> {
    let areas = areas.collect::<Vec<_>>();
    let mut cache = criteria_cache(engine);
    let mut due = true;
    for area in &areas {
        due &= cache.record_scan(*area);
    }
    let mut indexes = Vec::with_capacity(areas.len());
    for area in areas {
        if let Some(index) = cache.get(area) {
            indexes.push(index);
            continue;
        }
        if !due {
            return None;
        }
        drop(cache);
        let index = Arc::new(CriteriaIndex::build(engine, context, area)?);
        cache = criteria_cache(engine);
        cache.insert(area, Arc::clone(&index));
        indexes.push(index);
    }
    Some(indexes)
}

fn criteria_cache<'engine>(engine: &'engine Engine<'_>) -> RefMut<'engine, CriteriaIndexCache> {
    RefMut::map(
        engine.lookup_index_cache().borrow_mut(),
        LookupIndexCache::criteria_mut,
    )
}

/// Merges two ascending offset lists with no offset in common.
fn merge(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut merged = Vec::with_capacity(left.len() + right.len());
    let (mut left, mut right) = (left.iter().peekable(), right.iter().peekable());
    loop {
        let next = match (left.peek(), right.peek()) {
            (Some(l), Some(r)) if l <= r => left.next(),
            (Some(_), Some(_)) => right.next(),
            (Some(_), None) => left.next(),
            (None, Some(_)) => right.next(),
            (None, None) => break,
        };
        merged.extend(next.copied());
    }
    merged
}

/// The offsets of `scanned` also in `matching`, both ascending.
fn intersect(scanned: &[u32], matching: &[u32]) -> Vec<u32> {
    if scanned.len().saturating_mul(16) < matching.len() {
        return scanned
            .iter()
            .filter(|offset| matching.binary_search(offset).is_ok())
            .copied()
            .collect();
    }
    let mut both = Vec::new();
    let mut matching = matching.iter().peekable();
    for offset in scanned {
        while matching.next_if(|candidate| *candidate < offset).is_some() {}
        if matching.peek() == Some(&offset) {
            both.push(*offset);
        }
    }
    both
}

#[cfg(test)]
#[path = "criteria_index_tests.rs"]
mod tests;
