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
