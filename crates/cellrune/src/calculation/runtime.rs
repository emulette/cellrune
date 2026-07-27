use std::ops::RangeInclusive;

use super::value::ErrorKind;
use super::value::Value;
use super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

pub(super) type CellId = (usize, u32, u32);

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
#[allow(dead_code)] // Introduced for the 0.1.5 3-D resolver; 0.1.4 establishes the boundary.
pub(super) struct SheetSpan {
    sheets: RangeInclusive<usize>,
}

#[allow(dead_code)]
impl SheetSpan {
    pub(super) fn new(start: usize, end: usize) -> Self {
        Self {
            sheets: start.min(end)..=start.max(end),
        }
    }

    fn iter(&self) -> RangeInclusive<usize> {
        self.sheets.clone()
    }

    fn is_single_sheet(&self) -> bool {
        self.sheets.start() == self.sheets.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Introduced for the 0.1.5 3-D resolver; 0.1.4 establishes the boundary.
pub(super) struct RectSpan {
    sheets: SheetSpan,
    rect: Rect,
}

#[allow(dead_code)]
impl RectSpan {
    pub(super) fn new(sheets: SheetSpan, rect: Rect) -> Self {
        Self { sheets, rect }
    }

    pub(super) fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.sheets.iter().map(|sheet| Rect { sheet, ..self.rect })
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
        let rect = RectSpan::new(SheetSpan::new(4, 4), sample_rect())
            .into_rect()
            .expect("single-sheet span");
        assert_eq!(rect.sheet, 4);

        assert_eq!(
            RectSpan::new(SheetSpan::new(4, 5), sample_rect()).into_rect(),
            Err(ErrorKind::Ref)
        );
    }
}
