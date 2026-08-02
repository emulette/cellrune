use super::super::value::ErrorKind;

mod cpqr;
mod lu;

pub(super) use cpqr::{LeastSquaresSolution, solve_least_squares};
pub(super) use lu::invert;

#[derive(Debug, Clone)]
pub(super) struct DenseMatrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl DenseMatrix {
    pub(super) fn new(rows: usize, cols: usize, data: Vec<f64>) -> Result<Self, ErrorKind> {
        let cells = rows.checked_mul(cols).ok_or(ErrorKind::Num)?;
        if rows == 0
            || cols == 0
            || cells != data.len()
            || data.iter().any(|value| !value.is_finite())
        {
            return Err(ErrorKind::Value);
        }
        Ok(Self { rows, cols, data })
    }

    pub(super) const fn rows(&self) -> usize {
        self.rows
    }

    pub(super) const fn cols(&self) -> usize {
        self.cols
    }

    pub(super) fn get(&self, row: usize, col: usize) -> f64 {
        self.data[row * self.cols + col]
    }

    pub(super) fn set(&mut self, row: usize, col: usize, value: f64) {
        self.data[row * self.cols + col] = value;
    }

    fn swap_rows(&mut self, left: usize, right: usize) {
        if left == right {
            return;
        }
        for col in 0..self.cols {
            self.data
                .swap(left * self.cols + col, right * self.cols + col);
        }
    }

    fn swap_cols(&mut self, left: usize, right: usize) {
        if left == right {
            return;
        }
        for row in 0..self.rows {
            self.data
                .swap(row * self.cols + left, row * self.cols + right);
        }
    }

    pub(super) fn into_data(self) -> Vec<f64> {
        self.data
    }
}
