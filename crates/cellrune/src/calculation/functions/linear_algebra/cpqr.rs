use super::super::super::value::ErrorKind;
use super::DenseMatrix;

const CPQR_RANK_EPSILON: f64 = f64::EPSILON;

#[derive(Debug, Clone)]
pub(in crate::calculation::functions) struct LeastSquaresSolution {
    pub(in crate::calculation::functions) coefficients: Vec<f64>,
    pub(in crate::calculation::functions) active: Vec<bool>,
    pub(in crate::calculation::functions) covariance_diagonal: Vec<f64>,
    pub(in crate::calculation::functions) rank: usize,
}

pub(in crate::calculation::functions) fn solve_least_squares(
    mut design: DenseMatrix,
    mut response: Vec<f64>,
    mut charge_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<LeastSquaresSolution, ErrorKind> {
    if response.len() != design.rows() || response.iter().any(|value| !value.is_finite()) {
        return Err(ErrorKind::Value);
    }
    let rows = design.rows();
    let cols = design.cols();
    let factor_count = rows.min(cols);
    let mut scales = vec![1.0; cols];
    let mut original_columns = (0..cols).collect::<Vec<_>>();
    for (col, scale) in scales.iter_mut().enumerate() {
        charge_work(rows as u64)?;
        *scale = column_norm(&design, col, 0);
        if *scale > 0.0 {
            for row in 0..rows {
                design.set(row, col, design.get(row, col) / *scale);
            }
        }
    }

    let mut reflector = vec![0.0; rows];
    for pivot in 0..factor_count {
        let mut selected = pivot;
        let mut selected_norm = column_norm(&design, pivot, pivot);
        for col in (pivot + 1)..cols {
            let norm = column_norm(&design, col, pivot);
            if norm > selected_norm
                || (norm == selected_norm && original_columns[col] < original_columns[selected])
            {
                selected = col;
                selected_norm = norm;
            }
        }
        charge_product(&mut charge_work, cols - pivot, rows - pivot)?;
        design.swap_cols(pivot, selected);
        scales.swap(pivot, selected);
        original_columns.swap(pivot, selected);
        if selected_norm == 0.0 {
            continue;
        }

        let diagonal = design.get(pivot, pivot);
        let alpha = if diagonal >= 0.0 {
            -selected_norm
        } else {
            selected_norm
        };
        reflector[pivot] = diagonal - alpha;
        for (row, value) in reflector.iter_mut().enumerate().take(rows).skip(pivot + 1) {
            *value = design.get(row, pivot);
        }
        let reflector_norm = stable_norm(reflector[pivot..].iter().copied());
        if reflector_norm == 0.0 || !reflector_norm.is_finite() {
            return Err(ErrorKind::Num);
        }
        for value in &mut reflector[pivot..] {
            *value /= reflector_norm;
        }
        for col in pivot..cols {
            let mut projection = 0.0;
            for (row, reflector_value) in
                reflector.iter().copied().enumerate().take(rows).skip(pivot)
            {
                projection += reflector_value * design.get(row, col);
            }
            for (row, reflector_value) in
                reflector.iter().copied().enumerate().take(rows).skip(pivot)
            {
                let next = design.get(row, col) - 2.0 * reflector_value * projection;
                if !next.is_finite() {
                    return Err(ErrorKind::Num);
                }
                design.set(row, col, next);
            }
        }
        let mut response_projection = 0.0;
        for (reflector_value, response_value) in reflector[pivot..]
            .iter()
            .copied()
            .zip(response[pivot..].iter().copied())
        {
            response_projection += reflector_value * response_value;
        }
        for (response_value, reflector_value) in response[pivot..]
            .iter_mut()
            .zip(reflector[pivot..].iter().copied())
        {
            *response_value -= 2.0 * reflector_value * response_projection;
            if !response_value.is_finite() {
                return Err(ErrorKind::Num);
            }
        }
        design.set(pivot, pivot, alpha);
        for row in (pivot + 1)..rows {
            design.set(row, pivot, 0.0);
        }
        charge_product(&mut charge_work, cols - pivot + 1, rows - pivot)?;
    }

    let leading = if factor_count == 0 {
        0.0
    } else {
        design.get(0, 0).abs()
    };
    let rank_threshold = CPQR_RANK_EPSILON * rows.max(cols) as f64 * leading;
    let rank = (0..factor_count)
        .take_while(|index| design.get(*index, *index).abs() > rank_threshold)
        .count();
    let mut permuted_coefficients = vec![0.0; cols];
    for row in (0..rank).rev() {
        let mut value = response[row];
        for (col, coefficient) in permuted_coefficients
            .iter()
            .copied()
            .enumerate()
            .take(rank)
            .skip(row + 1)
        {
            value -= design.get(row, col) * coefficient;
        }
        permuted_coefficients[row] = value / design.get(row, row);
    }
    charge_product(&mut charge_work, rank, rank)?;

    let mut permuted_covariance = vec![0.0; cols];
    let mut inverse = vec![0.0; rank.checked_mul(rank).ok_or(ErrorKind::Num)?];
    for output_col in 0..rank {
        for row in (0..=output_col).rev() {
            let mut value = if row == output_col { 1.0 } else { 0.0 };
            for col in (row + 1)..=output_col {
                value -= design.get(row, col) * inverse[col * rank + output_col];
            }
            inverse[row * rank + output_col] = value / design.get(row, row);
        }
    }
    for row in 0..rank {
        let mut diagonal = 0.0;
        for col in row..rank {
            let value = inverse[row * rank + col];
            diagonal += value * value;
        }
        permuted_covariance[row] = diagonal / (scales[row] * scales[row]);
    }
    let rank_u64 = u64::try_from(rank).map_err(|_| ErrorKind::Num)?;
    let square = rank_u64.checked_mul(rank_u64).ok_or(ErrorKind::Num)?;
    let covariance_work = square
        .checked_mul(rank_u64)
        .and_then(|value| value.checked_add(square))
        .ok_or(ErrorKind::Num)?;
    charge_work(covariance_work)?;

    let mut coefficients = vec![0.0; cols];
    let mut active = vec![false; cols];
    let mut covariance_diagonal = vec![0.0; cols];
    for permuted in 0..cols {
        let original = original_columns[permuted];
        coefficients[original] = permuted_coefficients[permuted] / scales[permuted];
        active[original] = permuted < rank;
        covariance_diagonal[original] = permuted_covariance[permuted];
    }
    if coefficients.iter().any(|value| !value.is_finite())
        || covariance_diagonal.iter().any(|value| !value.is_finite())
    {
        return Err(ErrorKind::Num);
    }
    Ok(LeastSquaresSolution {
        coefficients,
        active,
        covariance_diagonal,
        rank,
    })
}

fn column_norm(matrix: &DenseMatrix, col: usize, row_start: usize) -> f64 {
    stable_norm((row_start..matrix.rows()).map(|row| matrix.get(row, col)))
}

fn stable_norm(values: impl IntoIterator<Item = f64>) -> f64 {
    values
        .into_iter()
        .fold(0.0_f64, |norm, value| norm.hypot(value))
}

fn charge_product(
    charge_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
    left: usize,
    right: usize,
) -> Result<(), ErrorKind> {
    let left = u64::try_from(left).map_err(|_| ErrorKind::Num)?;
    let right = u64::try_from(right).map_err(|_| ErrorKind::Num)?;
    charge_work(left.checked_mul(right).ok_or(ErrorKind::Num)?)
}

#[cfg(test)]
mod tests {
    use super::super::DenseMatrix;
    use super::solve_least_squares;

    #[test]
    fn cpqr_solves_a_linear_model() {
        let matrix = DenseMatrix::new(4, 2, vec![1.0, 1.0, 2.0, 1.0, 3.0, 1.0, 4.0, 1.0]).unwrap();
        let solved = solve_least_squares(matrix, vec![5.0, 7.0, 9.0, 11.0], |_| Ok(())).unwrap();
        assert_eq!(solved.rank, 2);
        assert!((solved.coefficients[0] - 2.0).abs() < 1e-12);
        assert!((solved.coefficients[1] - 3.0).abs() < 1e-12);
    }

    #[test]
    fn rank_deficient_columns_are_removed_deterministically() {
        let matrix =
            DenseMatrix::new(3, 3, vec![1.0, 2.0, 1.0, 2.0, 4.0, 1.0, 3.0, 6.0, 1.0]).unwrap();
        let solved = solve_least_squares(matrix, vec![3.0, 5.0, 7.0], |_| Ok(())).unwrap();
        assert_eq!(solved.rank, 2);
        assert_eq!(solved.active, vec![true, false, true]);
    }

    #[test]
    fn cpqr_rank_tolerance_removes_a_tiny_residual_column() {
        let matrix = DenseMatrix::new(
            4,
            3,
            vec![
                1.0, 1.0, 1.0, 0.0, 1.0e-20, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
        let solved = solve_least_squares(matrix, vec![2.0, 1.0, 1.0, 1.0], |_| Ok(())).unwrap();
        assert_eq!(solved.rank, 2);
        assert_eq!(solved.active, vec![true, false, true]);
    }
}
