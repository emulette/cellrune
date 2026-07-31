use std::iter::Once;
use std::ops::RangeInclusive;

use super::value::ErrorKind;
use super::value::Value;
use super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

pub(super) type CellId = (usize, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArrayExtent {
    row_end: u32,
}

impl ArrayExtent {
    pub(super) const fn new(row_end: u32) -> Self {
        Self {
            row_end: if row_end == 0 { 1 } else { row_end },
        }
    }

    pub(super) const fn row_end(self) -> u32 {
        self.row_end
    }

    pub(super) const fn merged(self, other: Self) -> Self {
        Self::new(if self.row_end >= other.row_end {
            self.row_end
        } else {
            other.row_end
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) sheet: usize,
    pub(super) row_start: u32,
    pub(super) col_start: u32,
    pub(super) row_end: u32,
    pub(super) col_end: u32,
    pub(super) whole_rows: bool,
}

impl Rect {
    pub(super) fn height(&self) -> u64 {
        u64::from(self.row_end - self.row_start) + 1
    }

    pub(super) fn width(&self) -> u64 {
        u64::from(self.col_end - self.col_start) + 1
    }

    pub(super) fn is_single_cell(&self) -> bool {
        self.row_start == self.row_end && self.col_start == self.col_end
    }

    pub(super) fn resized_from_anchor(self, height: u64, width: u64) -> Option<Self> {
        let row_offset = u32::try_from(height.checked_sub(1)?).ok()?;
        let col_offset = u32::try_from(width.checked_sub(1)?).ok()?;
        let row_end = self.row_start.checked_add(row_offset)?;
        let col_end = self.col_start.checked_add(col_offset)?;
        if row_end > EXCEL_MAX_ROWS || col_end > EXCEL_MAX_COLUMNS {
            return None;
        }
        Some(Self {
            sheet: self.sheet,
            row_start: self.row_start,
            col_start: self.col_start,
            row_end,
            col_end,
            whole_rows: self.row_start == 1 && row_end == EXCEL_MAX_ROWS,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SheetSpan {
    sheets: RangeInclusive<usize>,
    explicit_range: bool,
}

impl SheetSpan {
    pub(super) fn new(start: usize, end: usize) -> Self {
        Self {
            sheets: start.min(end)..=start.max(end),
            explicit_range: true,
        }
    }

    pub(super) fn single(sheet: usize) -> Self {
        Self {
            sheets: sheet..=sheet,
            explicit_range: false,
        }
    }

    fn iter(&self) -> RangeInclusive<usize> {
        self.sheets.clone()
    }

    pub(super) fn is_single_sheet(&self) -> bool {
        self.sheets.start() == self.sheets.end()
    }

    fn is_explicit_range(&self) -> bool {
        self.explicit_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RectSpan {
    sheets: SheetSpan,
    rect: Rect,
}

impl RectSpan {
    pub(super) fn new(sheets: SheetSpan, rect: Rect) -> Self {
        let rect = Rect {
            sheet: *sheets.sheets.start(),
            ..rect
        };
        Self { sheets, rect }
    }

    pub(super) fn single(rect: Rect) -> Self {
        Self::new(SheetSpan::single(rect.sheet), rect)
    }

    pub(super) fn rects(&self) -> RectSpanRects {
        RectSpanRects {
            sheets: self.sheets.iter(),
            rect: self.rect,
        }
    }

    pub(super) fn is_sheet_range(&self) -> bool {
        self.sheets.is_explicit_range()
    }

    pub(super) fn sort_key(&self) -> (usize, usize, bool, u32, u32, u32, u32, bool) {
        (
            *self.sheets.sheets.start(),
            *self.sheets.sheets.end(),
            self.sheets.explicit_range,
            self.rect.row_start,
            self.rect.col_start,
            self.rect.row_end,
            self.rect.col_end,
            self.rect.whole_rows,
        )
    }

    pub(super) fn into_rect(self) -> Result<Rect, ErrorKind> {
        if !self.sheets.is_single_sheet() {
            return Err(ErrorKind::Ref);
        }
        Ok(Rect {
            sheet: *self.sheets.sheets.start(),
            ..self.rect
        })
    }
}

pub(super) struct RectSpanRects {
    sheets: RangeInclusive<usize>,
    rect: Rect,
}

impl Iterator for RectSpanRects {
    type Item = Rect;

    fn next(&mut self) -> Option<Self::Item> {
        self.sheets.next().map(|sheet| Rect { sheet, ..self.rect })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReferenceArea {
    Rect(Rect),
    SheetSpan(RectSpan),
}

impl ReferenceArea {
    pub(super) fn from_span(span: RectSpan) -> Self {
        if span.is_sheet_range() {
            Self::SheetSpan(span)
        } else {
            Self::Rect(
                span.into_rect()
                    .expect("a non-range sheet span always contains exactly one sheet"),
            )
        }
    }

    pub(super) fn rects(&self) -> ReferenceAreaRects {
        match self {
            Self::Rect(rect) => ReferenceAreaRects::Rect(std::iter::once(*rect)),
            Self::SheetSpan(span) => ReferenceAreaRects::SheetSpan(span.rects()),
        }
    }

    pub(super) fn as_span(&self) -> RectSpan {
        match self {
            Self::Rect(rect) => RectSpan::single(*rect),
            Self::SheetSpan(span) => span.clone(),
        }
    }

    pub(super) fn is_sheet_span(&self) -> bool {
        matches!(self, Self::SheetSpan(_))
    }

    fn sheet_bounds(&self) -> (usize, usize) {
        match self {
            Self::Rect(rect) => (rect.sheet, rect.sheet),
            Self::SheetSpan(span) => (*span.sheets.sheets.start(), *span.sheets.sheets.end()),
        }
    }

    fn template_rect(&self) -> Rect {
        match self {
            Self::Rect(rect) => *rect,
            Self::SheetSpan(span) => span.rect,
        }
    }

    pub(super) fn intersection(&self, other: &Self) -> Option<Self> {
        let (left_sheet_start, left_sheet_end) = self.sheet_bounds();
        let (right_sheet_start, right_sheet_end) = other.sheet_bounds();
        let sheet_start = left_sheet_start.max(right_sheet_start);
        let sheet_end = left_sheet_end.min(right_sheet_end);
        if sheet_start > sheet_end {
            return None;
        }
        let left = self.template_rect();
        let right = other.template_rect();
        let row_start = left.row_start.max(right.row_start);
        let row_end = left.row_end.min(right.row_end);
        let col_start = left.col_start.max(right.col_start);
        let col_end = left.col_end.min(right.col_end);
        if row_start > row_end || col_start > col_end {
            return None;
        }
        let rect = Rect {
            sheet: sheet_start,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows: row_start == 1
                && row_end == EXCEL_MAX_ROWS
                && left.whole_rows
                && right.whole_rows,
        };
        if matches!(self, Self::SheetSpan(_)) && matches!(other, Self::SheetSpan(_)) {
            Some(Self::SheetSpan(RectSpan::new(
                SheetSpan::new(sheet_start, sheet_end),
                rect,
            )))
        } else {
            Some(Self::Rect(rect))
        }
    }
}

pub(super) enum ReferenceAreaRects {
    Rect(Once<Rect>),
    SheetSpan(RectSpanRects),
}

impl Iterator for ReferenceAreaRects {
    type Item = Rect;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rect(rect) => rect.next(),
            Self::SheetSpan(span) => span.next(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NonEmptyReferenceAreas(Box<[ReferenceArea]>);

impl NonEmptyReferenceAreas {
    fn new(areas: Vec<ReferenceArea>) -> Option<Self> {
        (!areas.is_empty()).then(|| Self(areas.into_boxed_slice()))
    }

    fn as_slice(&self) -> &[ReferenceArea] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReferenceValue {
    Empty,
    Areas(NonEmptyReferenceAreas),
}

impl ReferenceValue {
    pub(super) fn from_rect(rect: Rect) -> Self {
        Self::from_area(ReferenceArea::Rect(rect))
    }

    pub(super) fn from_span(span: RectSpan) -> Self {
        Self::from_area(ReferenceArea::from_span(span))
    }

    pub(super) fn from_area(area: ReferenceArea) -> Self {
        Self::Areas(
            NonEmptyReferenceAreas::new(vec![area])
                .expect("a one-element reference area collection is non-empty"),
        )
    }

    pub(super) fn from_areas(areas: Vec<ReferenceArea>) -> Self {
        NonEmptyReferenceAreas::new(areas).map_or(Self::Empty, Self::Areas)
    }

    pub(super) fn areas(&self) -> &[ReferenceArea] {
        match self {
            Self::Empty => &[],
            Self::Areas(areas) => areas.as_slice(),
        }
    }

    pub(super) fn area_count(&self) -> usize {
        self.areas().len()
    }

    pub(super) fn has_sheet_span(&self) -> bool {
        self.areas().iter().any(ReferenceArea::is_sheet_span)
    }

    pub(super) fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.areas().iter().flat_map(ReferenceArea::rects)
    }

    pub(super) fn single_rect(&self) -> Result<Rect, ErrorKind> {
        match self {
            Self::Empty => Err(ErrorKind::Ref),
            Self::Areas(areas) => match areas.as_slice() {
                [ReferenceArea::Rect(rect)] => Ok(*rect),
                _ => Err(ErrorKind::Value),
            },
        }
    }

    pub(super) fn into_single_rect(self) -> Result<Rect, ErrorKind> {
        match self {
            Self::Areas(areas) => {
                let mut areas = Vec::from(areas.0);
                if areas.len() != 1 {
                    return Err(ErrorKind::Value);
                }
                match areas.pop().expect("length checked") {
                    ReferenceArea::Rect(rect) => Ok(rect),
                    ReferenceArea::SheetSpan(_) => Err(ErrorKind::Value),
                }
            }
            Self::Empty => Err(ErrorKind::Ref),
        }
    }

    pub(super) fn single_area_span(&self) -> Result<RectSpan, ErrorKind> {
        let area = match self {
            Self::Empty => return Err(ErrorKind::Ref),
            Self::Areas(areas) => match areas.as_slice() {
                [area] => area,
                _ => return Err(ErrorKind::Value),
            },
        };
        match area {
            ReferenceArea::Rect(rect) => Ok(RectSpan::single(*rect)),
            ReferenceArea::SheetSpan(span) => Ok(span.clone()),
        }
    }

    pub(super) fn area_span(&self, one_based_index: usize) -> Result<RectSpan, ErrorKind> {
        if one_based_index == 0 {
            return Err(ErrorKind::Value);
        }
        self.areas()
            .get(one_based_index - 1)
            .map(ReferenceArea::as_span)
            .ok_or(ErrorKind::Ref)
    }

    pub(super) fn bounding_rect(&self) -> Result<Rect, ErrorKind> {
        let mut areas = self.areas().iter();
        let Some(ReferenceArea::Rect(first)) = areas.next() else {
            return Err(match self {
                Self::Empty => ErrorKind::Ref,
                Self::Areas(_) => ErrorKind::Value,
            });
        };
        let mut result = *first;
        for area in areas {
            let ReferenceArea::Rect(rect) = area else {
                return Err(ErrorKind::Value);
            };
            if rect.sheet != result.sheet {
                return Err(ErrorKind::Value);
            }
            result.row_start = result.row_start.min(rect.row_start);
            result.col_start = result.col_start.min(rect.col_start);
            result.row_end = result.row_end.max(rect.row_end);
            result.col_end = result.col_end.max(rect.col_end);
            result.whole_rows = result.row_start == 1
                && result.row_end == EXCEL_MAX_ROWS
                && (result.whole_rows || rect.whole_rows);
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Array {
    pub(super) rows: u32,
    pub(super) cols: u32,
    pub(super) data: Vec<Value>,
}

impl Array {
    pub(super) fn scalar(value: Value) -> Self {
        Self {
            rows: 1,
            cols: 1,
            data: vec![value],
        }
    }

    pub(super) fn at(&self, row: u32, col: u32) -> &Value {
        &self.data[(row * self.cols + col) as usize]
    }

    pub(super) fn is_scalar(&self) -> bool {
        self.rows == 1 && self.cols == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rect() -> Rect {
        Rect {
            sheet: 99,
            row_start: 2,
            col_start: 3,
            row_end: 4,
            col_end: 5,
            whole_rows: false,
        }
    }

    #[test]
    fn rect_span_expands_in_workbook_order() {
        let span = RectSpan::new(SheetSpan::new(3, 1), sample_rect());
        let rects = span.rects().collect::<Vec<_>>();

        assert_eq!(
            rects.iter().map(|rect| rect.sheet).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(rects.iter().all(|rect| {
            rect.row_start == 2 && rect.col_start == 3 && rect.row_end == 4 && rect.col_end == 5
        }));
    }

    #[test]
    fn rect_span_narrows_only_when_it_has_one_sheet() {
        let rect = RectSpan::new(SheetSpan::single(4), sample_rect())
            .into_rect()
            .expect("single-sheet span");
        assert_eq!(rect.sheet, 4);

        assert_eq!(
            RectSpan::new(SheetSpan::new(4, 5), sample_rect()).into_rect(),
            Err(ErrorKind::Ref)
        );
    }

    #[test]
    fn rect_span_preserves_explicit_same_sheet_range_syntax() {
        let span = RectSpan::new(SheetSpan::new(4, 4), sample_rect());

        assert!(span.is_sheet_range());
        assert_eq!(span.rects().count(), 1);
    }

    #[test]
    fn reference_value_distinguishes_empty_and_preserves_area_identity() {
        let rect = sample_rect();
        let repeated =
            ReferenceValue::from_areas(vec![ReferenceArea::Rect(rect), ReferenceArea::Rect(rect)]);

        assert_eq!(
            ReferenceValue::from_areas(Vec::new()),
            ReferenceValue::Empty
        );
        assert_eq!(ReferenceValue::Empty.single_rect(), Err(ErrorKind::Ref));
        assert_eq!(repeated.area_count(), 2);
        assert_eq!(repeated.rects().collect::<Vec<_>>(), vec![rect, rect]);
        assert_eq!(repeated.single_rect(), Err(ErrorKind::Value));
    }

    #[test]
    fn reference_value_narrows_only_one_plain_rectangle() {
        let rect = sample_rect();
        assert_eq!(ReferenceValue::from_rect(rect).single_rect(), Ok(rect));
        assert_eq!(
            ReferenceValue::from_span(RectSpan::new(SheetSpan::new(4, 4), rect)).single_rect(),
            Err(ErrorKind::Value)
        );
    }

    #[test]
    fn reference_value_bounds_ordered_same_sheet_areas_without_merging_identity() {
        let first = sample_rect();
        let second = Rect {
            row_start: 1,
            col_start: 2,
            row_end: 6,
            col_end: 7,
            ..first
        };
        let reference = ReferenceValue::from_areas(vec![
            ReferenceArea::Rect(first),
            ReferenceArea::Rect(second),
        ]);

        assert_eq!(reference.area_count(), 2);
        assert_eq!(
            reference.bounding_rect(),
            Ok(Rect {
                row_start: 1,
                col_start: 2,
                row_end: 6,
                col_end: 7,
                ..first
            })
        );
    }
}
