//! Exact-match indexes shared by repeated lookups over one range within a calculation pass.
//!
//! A linear exact search reads a range from one end until the first match. When the same range is
//! searched again, one full read builds an index that answers each later search with the offset
//! the linear search would stop at. Each indexed search is charged exactly the cell reads and work
//! units the linear search would have charged, so results and limit failures do not change. An
//! index is dropped when a result inside its range changes.

use std::collections::HashMap;
use std::sync::Arc;

use super::super::eval::{Engine, EvalContext};
use super::super::runtime::{CellId, Rect};
use super::super::value::{ErrorKind, Value};
use crate::CalculationLimitKind;

/// Ranges shorter than this are scanned linearly; an index would not pay for itself.
const MIN_INDEXED_LENGTH: u32 = 32;
/// A range is indexed once it has been searched this many times in one pass.
const SEARCHES_BEFORE_INDEXING: u32 = 2;
/// Most recently used indexes kept at once.
const MAX_INDEXES: usize = 16;
/// Cells covered by all kept indexes together.
const MAX_INDEXED_CELLS: u64 = 2_000_000;
const CANCELLATION_POLL_INTERVAL: u32 = 4_096;

/// The cells one lookup searches: `length` cells down a column or along a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::calculation) struct LookupArea {
    sheet: usize,
    row: u32,
    column: u32,
    length: u32,
    horizontal: bool,
}

impl LookupArea {
    /// The area starting at `rect`'s top-left cell, or `None` when it cannot be addressed.
    pub(super) fn new(rect: Rect, length: u64, horizontal: bool) -> Option<Self> {
        Some(Self {
            sheet: rect.sheet,
            row: rect.row_start,
            column: rect.col_start,
            length: u32::try_from(length).ok()?,
            horizontal,
        })
    }

    fn cell(self, offset: u32) -> CellId {
        if self.horizontal {
            (self.sheet, self.row, self.column + offset)
        } else {
            (self.sheet, self.row + offset, self.column)
        }
    }

    fn last_row(self) -> u32 {
        if self.horizontal {
            self.row
        } else {
            self.row + self.length.saturating_sub(1)
        }
    }

    fn last_column(self) -> u32 {
        if self.horizontal {
            self.column + self.length.saturating_sub(1)
        } else {
            self.column
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

/// Indexes built during one calculation, most recently used last.
#[derive(Debug, Default)]
pub(in crate::calculation) struct LookupIndexCache {
    searches: HashMap<LookupArea, u32>,
    indexes: Vec<(LookupArea, Arc<ExactLookupIndex>)>,
}

impl LookupIndexCache {
    pub(in crate::calculation) fn clear(&mut self) {
        self.searches.clear();
        self.indexes.clear();
    }

    /// Drops indexes whose range holds `cell`, whose result is about to change.
    pub(in crate::calculation) fn invalidate_cell(&mut self, cell: CellId) {
        if self.indexes.is_empty() {
            return;
        }
        let searches = &mut self.searches;
        self.indexes.retain(|(area, _)| {
            let keep = !area.contains(cell);
            if !keep {
                searches.insert(*area, 0);
            }
            keep
        });
    }

    /// Drops indexes whose range meets `rect`, whose cells are about to read a different owner.
    pub(in crate::calculation) fn invalidate_rect(&mut self, rect: &Rect) {
        let searches = &mut self.searches;
        self.indexes.retain(|(area, _)| {
            let keep = !area.intersects(rect);
            if !keep {
                searches.insert(*area, 0);
            }
            keep
        });
    }

    fn get(&mut self, area: LookupArea) -> Option<Arc<ExactLookupIndex>> {
        let position = self
            .indexes
            .iter()
            .position(|(candidate, _)| *candidate == area)?;
        let entry = self.indexes.remove(position);
        let index = Arc::clone(&entry.1);
        self.indexes.push(entry);
        Some(index)
    }

    /// Counts one search of `area` and returns whether it is now due for an index.
    fn record_search(&mut self, area: LookupArea) -> bool {
        let searches = self.searches.entry(area).or_insert(0);
        *searches = searches.saturating_add(1);
        *searches >= SEARCHES_BEFORE_INDEXING
    }

    fn insert(&mut self, area: LookupArea, index: Arc<ExactLookupIndex>) {
        let mut cells = u64::from(area.length);
        for (kept, _) in &self.indexes {
            cells += u64::from(kept.length);
        }
        while !self.indexes.is_empty()
            && (self.indexes.len() >= MAX_INDEXES || cells > MAX_INDEXED_CELLS)
        {
            let (evicted, _) = self.indexes.remove(0);
            cells -= u64::from(evicted.length);
        }
        self.indexes.push((area, index));
    }
}

/// The value classes an exact search can match, each with its first and last offset.
#[derive(Debug)]
pub(in crate::calculation) struct ExactLookupIndex {
    values: HashMap<EqualityKey, Span>,
    numeric_texts: HashMap<u64, Span>,
    blanks: Option<Span>,
    contains_nan: bool,
    texts_before: Vec<u32>,
    nonempty_texts_before: Vec<u32>,
    blanks_before: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum EqualityKey {
    Number(u64),
    Text(Box<str>),
    Logical(bool),
}

#[derive(Debug, Clone, Copy)]
struct Span {
    first: u32,
    last: u32,
}

impl Span {
    /// Extends `span` to `offset`, which is past every offset recorded before it.
    fn extend(span: &mut Option<Self>, offset: u32) {
        match span {
            Some(span) => span.last = offset,
            None => {
                *span = Some(Self {
                    first: offset,
                    last: offset,
                });
            }
        }
    }

    fn record<K: std::hash::Hash + Eq>(spans: &mut HashMap<K, Self>, key: K, offset: u32) {
        spans
            .entry(key)
            .and_modify(|span| span.last = offset)
            .or_insert(Self {
                first: offset,
                last: offset,
            });
    }
}

/// Which end a search starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchDirection {
    Forward,
    Reverse,
}

impl ExactLookupIndex {
    fn build(engine: &Engine<'_>, context: EvalContext<'_>, area: LookupArea) -> Option<Self> {
        let length = area.length as usize;
        let mut index = Self {
            values: HashMap::new(),
            numeric_texts: HashMap::new(),
            blanks: None,
            contains_nan: false,
            texts_before: Vec::with_capacity(length + 1),
            nonempty_texts_before: Vec::with_capacity(length + 1),
            blanks_before: Vec::with_capacity(length + 1),
        };
        let (mut texts, mut nonempty_texts, mut blanks) = (0_u32, 0_u32, 0_u32);
        for offset in 0..area.length {
            if offset % CANCELLATION_POLL_INTERVAL == 0 && context.is_cancelled() {
                return None;
            }
            index.texts_before.push(texts);
            index.nonempty_texts_before.push(nonempty_texts);
            index.blanks_before.push(blanks);
            let key = match engine.cell_value(area.cell(offset)) {
                Value::Blank => {
                    blanks += 1;
                    Span::extend(&mut index.blanks, offset);
                    continue;
                }
                Value::Error(_) => continue,
                Value::Number(number) => match number_key(number) {
                    Some(key) => EqualityKey::Number(key),
                    None => {
                        index.contains_nan = true;
                        continue;
                    }
                },
                Value::Logical(logical) => EqualityKey::Logical(logical),
                Value::Text(text) => {
                    texts += 1;
                    if !text.is_empty() {
                        nonempty_texts += 1;
                    }
                    if let Some(key) = text.trim().parse::<f64>().ok().and_then(number_key) {
                        Span::record(&mut index.numeric_texts, key, offset);
                    }
                    EqualityKey::Text(lowercase(&text))
                }
            };
            Span::record(&mut index.values, key, offset);
        }
        index.texts_before.push(texts);
        index.nonempty_texts_before.push(nonempty_texts);
        index.blanks_before.push(blanks);
        Some(index)
    }

    /// The span of cells `coerce::compare` finds equal to `lookup`, which compares a blank cell as
    /// the lookup type's empty value, or `None` when the index cannot answer.
    fn compare_equal(&self, lookup: &Value) -> Option<Option<Span>> {
        let (key, blank_matches) = match lookup {
            Value::Number(number) => {
                if self.contains_nan {
                    return None;
                }
                (EqualityKey::Number(number_key(*number)?), *number == 0.0)
            }
            Value::Text(text) => (EqualityKey::Text(lowercase(text)), text.is_empty()),
            Value::Logical(logical) => (EqualityKey::Logical(*logical), !logical),
            Value::Blank | Value::Error(_) => return None,
        };
        let blanks = if blank_matches { self.blanks } else { None };
        Some(union(self.values.get(&key).copied(), blanks))
    }

    /// The span of cells an exact-equality criterion built from `lookup` matches, or `None` when
    /// the index cannot answer.
    fn criteria_equal(&self, lookup: &Value) -> Option<Option<Span>> {
        match lookup {
            Value::Number(number) => {
                let key = number_key(*number)?;
                Some(union(
                    self.values.get(&EqualityKey::Number(key)).copied(),
                    self.numeric_texts.get(&key).copied(),
                ))
            }
            Value::Text(text) if !text.is_empty() && !has_wildcard_syntax(text) => Some(
                self.values
                    .get(&EqualityKey::Text(lowercase(text)))
                    .copied(),
            ),
            Value::Logical(logical) => {
                Some(self.values.get(&EqualityKey::Logical(*logical)).copied())
            }
            Value::Text(_) | Value::Blank | Value::Error(_) => None,
        }
    }

    fn texts_in(&self, start: u32, end: u32) -> u64 {
        u64::from(self.texts_before[end as usize] - self.texts_before[start as usize])
    }

    fn nonempty_texts_in(&self, start: u32, end: u32) -> u64 {
        u64::from(
            self.nonempty_texts_before[end as usize] - self.nonempty_texts_before[start as usize],
        )
    }

    fn blanks_in(&self, start: u32, end: u32) -> u64 {
        u64::from(self.blanks_before[end as usize] - self.blanks_before[start as usize])
    }
}

/// The offset of a VLOOKUP or HLOOKUP exact search, or `None` when the caller must scan.
pub(super) fn indexed_lookup_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    area: LookupArea,
    lookup: &Value,
) -> Option<Result<u32, ErrorKind>> {
    // A text lookup containing wildcard syntax is a pattern, which the index does not hold.
    if matches!(lookup, Value::Text(text) if has_wildcard_syntax(text)) {
        return None;
    }
    let index = exact_lookup_index(engine, context, area)?;
    let span = index.compare_equal(lookup)?;
    let found = span.map(|span| span.first);
    let reads = found.map_or(area.length, |offset| offset + 1);
    charge_indexed_search(engine, context, area, reads, 0, 0)
        .map(|charged| charged.and_then(|()| found.ok_or(ErrorKind::NA)))
}

/// The offset of an exact MATCH, whose criterion compares numbers with numeric text and text
/// case-insensitively, or `None` when the caller must scan.
pub(super) fn indexed_match_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    area: LookupArea,
    lookup: &Value,
) -> Option<Result<u32, ErrorKind>> {
    let index = exact_lookup_index(engine, context, area)?;
    let span = index.criteria_equal(lookup)?;
    let found = span.map(|span| span.first);
    let reads = found.map_or(area.length, |offset| offset + 1);
    // A number criterion parses every text cell it meets, and a text criterion compares every
    // nonempty text cell; each costs one work unit.
    let (work, full_work) = match lookup {
        Value::Number(_) => (index.texts_in(0, reads), index.texts_in(0, area.length)),
        Value::Text(_) => (
            index.nonempty_texts_in(0, reads),
            index.nonempty_texts_in(0, area.length),
        ),
        _ => (0, 0),
    };
    charge_indexed_search(engine, context, area, reads, work, full_work)
        .map(|charged| charged.and_then(|()| found.ok_or(ErrorKind::NA)))
}

/// The offset of an exact XLOOKUP search from either end, or `None` when the caller must scan.
pub(super) fn indexed_xlookup_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    area: LookupArea,
    lookup: &Value,
    direction: SearchDirection,
) -> Option<Result<u32, ErrorKind>> {
    let index = exact_lookup_index(engine, context, area)?;
    let span = index.compare_equal(lookup)?;
    let (found, start, end) = match (direction, span) {
        (SearchDirection::Forward, Some(span)) => (Some(span.first), 0, span.first + 1),
        (SearchDirection::Reverse, Some(span)) => (Some(span.last), span.last, area.length),
        (_, None) => (None, 0, area.length),
    };
    // Each step costs one work unit, and comparing a text lookup with a text or blank cell one
    // more.
    let comparisons = |start, end| match lookup {
        Value::Text(_) => index.texts_in(start, end) + index.blanks_in(start, end),
        _ => 0,
    };
    let reads = end - start;
    let work = u64::from(reads) + comparisons(start, end);
    let full_work = u64::from(area.length) + comparisons(0, area.length);
    charge_indexed_search(engine, context, area, reads, work, full_work)
        .map(|charged| charged.and_then(|()| found.ok_or(ErrorKind::NA)))
}

/// Returns the index for `area`, building it once the area is due, or `None` when the search must
/// scan: during targeted calculation, whose cell reads record pending dependencies, for short
/// ranges, and for ranges not yet searched often enough.
fn exact_lookup_index(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    area: LookupArea,
) -> Option<Arc<ExactLookupIndex>> {
    if engine.is_targeted() || area.length < MIN_INDEXED_LENGTH {
        return None;
    }
    let mut cache = engine.lookup_index_cache().borrow_mut();
    if let Some(index) = cache.get(area) {
        return Some(index);
    }
    if !cache.record_search(area) {
        return None;
    }
    drop(cache);
    let index = Arc::new(ExactLookupIndex::build(engine, context, area)?);
    engine
        .lookup_index_cache()
        .borrow_mut()
        .insert(area, Arc::clone(&index));
    Some(index)
}

/// Charges an indexed search what the linear search would have charged, or returns `None` when a
/// full linear search could exceed the remaining budget, so that the scan reports which limit it
/// reaches first.
fn charge_indexed_search(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    area: LookupArea,
    reads: u32,
    work: u64,
    full_work: u64,
) -> Option<Result<(), ErrorKind>> {
    if engine.remaining_reference_cells(context) < u64::from(area.length)
        || engine.remaining_function_iterations(context) < full_work
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
            .charge_reference_cells(context, u64::from(reads))
            .and_then(|()| engine.charge_function_iterations(context, work)),
    )
}

fn union(left: Option<Span>, right: Option<Span>) -> Option<Span> {
    match (left, right) {
        (Some(left), Some(right)) => Some(Span {
            first: left.first.min(right.first),
            last: left.last.max(right.last),
        }),
        (span, None) | (None, span) => span,
    }
}

/// The key under which `coerce::compare` finds two numbers equal: zero and negative zero share one.
fn number_key(number: f64) -> Option<u64> {
    (!number.is_nan()).then(|| (number + 0.0).to_bits())
}

/// The character sequence `coerce::compare_text_case_insensitive` compares.
fn lowercase(text: &str) -> Box<str> {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Whether a criterion built from `text` would treat any character as wildcard syntax.
fn has_wildcard_syntax(text: &str) -> bool {
    text.contains(['?', '*', '~'])
}

#[cfg(test)]
#[path = "lookup_index_tests.rs"]
mod tests;
