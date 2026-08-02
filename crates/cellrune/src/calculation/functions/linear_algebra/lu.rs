use super::super::super::value::ErrorKind;
use super::DenseMatrix;

const LU_SINGULAR_EPSILON: f64 = f64::EPSILON;

pub(in crate::calculation::functions) fn invert(
    mut matrix: DenseMatrix,
    mut charge_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<DenseMatrix, ErrorKind> {
    if matrix.rows() != matrix.cols() {
        return Err(ErrorKind::Value);
    }
    let dimension = matrix.rows();
    let scale = maximum_absolute_value(&matrix);
    if scale == 0.0 || !scale.is_finite() {
        return Err(ErrorKind::Num);
    }
    let singular_threshold = LU_SINGULAR_EPSILON * dimension as f64 * scale;
    let mut permutation = (0..dimension).collect::<Vec<_>>();

    for pivot_col in 0..dimension {
        charge_work((dimension - pivot_col) as u64)?;
        let (pivot_row, pivot_abs) = select_pivot_row(&matrix, pivot_col);
        if !pivot_abs.is_finite() || pivot_abs <= singular_threshold {
            return Err(ErrorKind::Num);
        }
        matrix.swap_rows(pivot_col, pivot_row);
        permutation.swap(pivot_col, pivot_row);

        let pivot = matrix.get(pivot_col, pivot_col);
        for row in (pivot_col + 1)..dimension {
            let factor = matrix.get(row, pivot_col) / pivot;
            if !factor.is_finite() {
                return Err(ErrorKind::Num);
            }
            matrix.set(row, pivot_col, factor);
            charge_work((dimension - pivot_col - 1) as u64)?;
            for col in (pivot_col + 1)..dimension {
                let next = matrix.get(row, col) - factor * matrix.get(pivot_col, col);
                if !next.is_finite() {
                    return Err(ErrorKind::Num);
                }
                matrix.set(row, col, next);
            }
        }
    }

    let cells = dimension.checked_mul(dimension).ok_or(ErrorKind::Num)?;
    let mut inverse = vec![0.0; cells];
    let mut rhs = vec![0.0; dimension];
    let mut solution = vec![0.0; dimension];
    let dimension_u64 = u64::try_from(dimension).map_err(|_| ErrorKind::Num)?;
    let solve_work = dimension_u64
        .checked_mul(dimension_u64)
        .and_then(|value| value.checked_mul(2))
        .ok_or(ErrorKind::Num)?;
    for output_col in 0..dimension {
        charge_work(solve_work)?;
        for row in 0..dimension {
            let permuted_identity = if permutation[row] == output_col {
                1.0
            } else {
                0.0
            };
            let mut value = permuted_identity;
            for (col, rhs_value) in rhs.iter().copied().enumerate().take(row) {
                value -= matrix.get(row, col) * rhs_value;
            }
            rhs[row] = value;
        }
        for row in (0..dimension).rev() {
            let mut value = rhs[row];
            for (col, solution_value) in solution
                .iter()
                .copied()
                .enumerate()
                .take(dimension)
                .skip(row + 1)
            {
                value -= matrix.get(row, col) * solution_value;
            }
            let solved = value / matrix.get(row, row);
            if !solved.is_finite() {
                return Err(ErrorKind::Num);
            }
            solution[row] = solved;
            inverse[row * dimension + output_col] = solved;
        }
    }
    DenseMatrix::new(dimension, dimension, inverse)
}

fn select_pivot_row(matrix: &DenseMatrix, pivot_col: usize) -> (usize, f64) {
    let mut pivot_row = pivot_col;
    let mut pivot_abs = matrix.get(pivot_col, pivot_col).abs();
    for row in (pivot_col + 1)..matrix.rows() {
        let candidate = matrix.get(row, pivot_col).abs();
        if candidate > pivot_abs {
            pivot_abs = candidate;
            pivot_row = row;
        }
    }
    (pivot_row, pivot_abs)
}

fn maximum_absolute_value(matrix: &DenseMatrix) -> f64 {
    let mut maximum = 0.0_f64;
    for row in 0..matrix.rows() {
        for col in 0..matrix.cols() {
            maximum = maximum.max(matrix.get(row, col).abs());
        }
    }
    maximum
}

#[cfg(test)]
mod tests {
    use super::super::DenseMatrix;
    use super::{invert, select_pivot_row};
    use crate::calculation::value::ErrorKind;

    #[test]
    fn partial_pivot_lu_inverts_a_well_conditioned_matrix() {
        let matrix = DenseMatrix::new(2, 2, vec![4.0, 7.0, 2.0, 6.0]).unwrap();
        let inverse = invert(matrix, |_| Ok(())).unwrap().into_data();
        let expected = [0.6, -0.7, -0.2, 0.4];
        for (actual, expected) in inverse.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn singular_matrix_is_rejected() {
        let matrix = DenseMatrix::new(2, 2, vec![1.0, 2.0, 2.0, 4.0]).unwrap();
        assert!(matches!(invert(matrix, |_| Ok(())), Err(ErrorKind::Num)));
    }

    #[test]
    fn equal_absolute_pivots_keep_the_lower_current_row() {
        let matrix =
            DenseMatrix::new(3, 3, vec![2.0, 0.0, 0.0, -2.0, 1.0, 0.0, 2.0, 0.0, 1.0]).unwrap();
        assert_eq!(select_pivot_row(&matrix, 0), (0, 2.0));
    }

    #[test]
    fn lu_singularity_tolerance_rejects_a_tiny_pivot() {
        let matrix = DenseMatrix::new(2, 2, vec![1.0, 0.0, 0.0, 1.0e-20]).unwrap();
        assert!(matches!(invert(matrix, |_| Ok(())), Err(ErrorKind::Num)));
    }
}
