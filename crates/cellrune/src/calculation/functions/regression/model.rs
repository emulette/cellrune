use super::super::super::value::ErrorKind;
use super::super::linear_algebra::{DenseMatrix, LeastSquaresSolution, solve_least_squares};
use super::super::moments::NumericMoments;
use super::input::{NormalizedPredictionInput, NormalizedRegressionInput};

#[derive(Debug, Clone)]
pub(super) struct RegressionModel {
    pub(super) slopes: Vec<f64>,
    pub(super) intercept: f64,
    pub(super) active_slopes: Vec<bool>,
    pub(super) intercept_active: bool,
    pub(super) slope_standard_errors: Vec<Result<f64, ErrorKind>>,
    pub(super) intercept_standard_error: Result<f64, ErrorKind>,
    pub(super) r_squared: Result<f64, ErrorKind>,
    pub(super) standard_error_y: Result<f64, ErrorKind>,
    pub(super) f_statistic: Result<f64, ErrorKind>,
    pub(super) degrees_of_freedom: usize,
    pub(super) regression_sum_squares: f64,
    pub(super) residual_sum_squares: f64,
}

pub(super) fn fit(
    known: &NormalizedRegressionInput,
    include_intercept: bool,
    logarithmic: bool,
    mut charge_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<RegressionModel, ErrorKind> {
    let response = if logarithmic {
        known
            .response
            .iter()
            .map(|value| {
                if *value > 0.0 {
                    Ok(value.ln())
                } else {
                    Err(ErrorKind::Num)
                }
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        known.response.clone()
    };
    let parameter_count = known.variables + usize::from(include_intercept);
    if parameter_count == 0 {
        return Err(ErrorKind::Value);
    }
    let mut design_data = Vec::with_capacity(
        known
            .samples
            .checked_mul(parameter_count)
            .ok_or(ErrorKind::Num)?,
    );
    for sample in 0..known.samples {
        let start = sample * known.variables;
        design_data.extend_from_slice(&known.predictors[start..start + known.variables]);
        if include_intercept {
            design_data.push(1.0);
        }
    }
    let design = DenseMatrix::new(known.samples, parameter_count, design_data)?;
    let solution = solve_least_squares(design, response.clone(), &mut charge_work)?;
    materialize_model(
        known,
        response,
        solution,
        include_intercept,
        &mut charge_work,
    )
}

pub(super) fn predict(
    model: &RegressionModel,
    input: &NormalizedPredictionInput,
    logarithmic: bool,
    mut charge_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<Vec<f64>, ErrorKind> {
    if input.predictors.len() != input.samples * model.slopes.len() {
        return Err(ErrorKind::Ref);
    }
    let mut predictions = Vec::with_capacity(input.samples);
    for sample in 0..input.samples {
        let mut prediction = model.intercept;
        for variable in 0..model.slopes.len() {
            prediction +=
                input.predictors[sample * model.slopes.len() + variable] * model.slopes[variable];
        }
        if logarithmic {
            prediction = prediction.exp();
        }
        if !prediction.is_finite() {
            return Err(ErrorKind::Num);
        }
        predictions.push(prediction);
        charge_work((model.slopes.len() + 1) as u64)?;
    }
    Ok(predictions)
}

fn materialize_model(
    known: &NormalizedRegressionInput,
    response: Vec<f64>,
    solution: LeastSquaresSolution,
    include_intercept: bool,
    charge_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<RegressionModel, ErrorKind> {
    let intercept_index = include_intercept.then_some(known.variables);
    let intercept = intercept_index
        .map(|index| solution.coefficients[index])
        .unwrap_or(0.0);
    let intercept_active = intercept_index
        .map(|index| solution.active[index])
        .unwrap_or(false);
    let slopes = solution.coefficients[..known.variables].to_vec();
    let active_slopes = solution.active[..known.variables].to_vec();

    let mut residuals = Vec::with_capacity(known.samples);
    for (sample, actual) in response.iter().copied().enumerate() {
        let mut fitted = intercept;
        for (variable, slope) in slopes.iter().copied().enumerate() {
            fitted += known.predictors[sample * known.variables + variable] * slope;
        }
        residuals.push(actual - fitted);
    }
    let residual_moments = NumericMoments::collect_with_work(residuals, || charge_work(1))?;
    let response_moments = NumericMoments::collect_with_work(response, || charge_work(1))?;
    let total_sum_squares = if include_intercept {
        response_moments.second_moment()
    } else {
        response_moments.sum_squares_about_zero()?
    };
    let raw_residual = residual_moments.sum_squares_about_zero()?;
    let exact_fit_threshold = f64::EPSILON * 64.0 * total_sum_squares.max(1.0);
    let residual_sum_squares = if raw_residual <= exact_fit_threshold {
        0.0
    } else {
        raw_residual
    };
    let regression_sum_squares = (total_sum_squares - residual_sum_squares).max(0.0);
    let degrees_of_freedom = known.samples.saturating_sub(solution.rank);
    let mean_squared_error: Result<f64, ErrorKind> = if degrees_of_freedom == 0 {
        Err(ErrorKind::Num)
    } else {
        Ok(residual_sum_squares / degrees_of_freedom as f64)
    };
    let standard_error_y = mean_squared_error.map(f64::sqrt);
    let r_squared = if total_sum_squares > 0.0 {
        Ok((regression_sum_squares / total_sum_squares).clamp(0.0, 1.0))
    } else if residual_sum_squares == 0.0 {
        Ok(1.0)
    } else {
        Err(ErrorKind::Num)
    };
    let model_degrees = solution
        .rank
        .saturating_sub(usize::from(include_intercept && intercept_active));
    let f_statistic = match (model_degrees, mean_squared_error) {
        (0, _) | (_, Err(_)) => Err(ErrorKind::Num),
        (_, Ok(0.0)) => Err(ErrorKind::Num),
        (model_degrees, Ok(error)) => {
            let value = regression_sum_squares / model_degrees as f64 / error;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(ErrorKind::Num)
            }
        }
    };
    let slope_standard_errors = (0..known.variables)
        .map(|index| {
            if !solution.active[index] {
                Ok(0.0)
            } else {
                mean_squared_error
                    .and_then(|error| finite_sqrt(error * solution.covariance_diagonal[index]))
            }
        })
        .collect();
    let intercept_standard_error = match intercept_index {
        None => Err(ErrorKind::NA),
        Some(index) if !solution.active[index] => Ok(0.0),
        Some(index) => mean_squared_error
            .and_then(|error| finite_sqrt(error * solution.covariance_diagonal[index])),
    };
    Ok(RegressionModel {
        slopes,
        intercept,
        active_slopes,
        intercept_active,
        slope_standard_errors,
        intercept_standard_error,
        r_squared,
        standard_error_y,
        f_statistic,
        degrees_of_freedom,
        regression_sum_squares,
        residual_sum_squares,
    })
}

fn finite_sqrt(value: f64) -> Result<f64, ErrorKind> {
    let result = value.max(0.0).sqrt();
    if result.is_finite() {
        Ok(result)
    } else {
        Err(ErrorKind::Num)
    }
}
