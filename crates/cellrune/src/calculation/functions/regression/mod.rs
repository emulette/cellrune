use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::cell_count;
use super::kernel::RegressionFunction;

mod input;
mod model;

use input::{normalize_known, normalize_prediction, optional_logical};
use model::{RegressionModel, fit, predict};

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: RegressionFunction,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    match function {
        RegressionFunction::LinEst => regression_array(engine, context, args, false),
        RegressionFunction::LogEst => regression_array(engine, context, args, true),
        RegressionFunction::Trend => prediction_array(engine, context, args, false),
        RegressionFunction::Growth => prediction_array(engine, context, args, true),
    }
}

pub(super) fn call_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: RegressionFunction,
    args: &[Expr],
) -> Value {
    match call_array(engine, context, function, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Err(kind) => Value::Error(kind),
    }
}

fn regression_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    logarithmic: bool,
) -> Result<Array, ErrorKind> {
    let Some(known_y) = args.first() else {
        return Err(ErrorKind::Value);
    };
    let known = normalize_known(engine, context, known_y, args.get(1))?;
    let include_intercept = optional_logical(engine, context, args.get(2), true)?;
    let include_statistics = optional_logical(engine, context, args.get(3), false)?;
    let rows = if include_statistics { 5 } else { 1 };
    let cols = u32::try_from(known.variables.checked_add(1).ok_or(ErrorKind::Num)?)
        .map_err(|_| ErrorKind::Num)?;
    let output_cells = cell_count(rows, cols)?;
    ensure_regression_workspace(
        engine,
        known.samples,
        known.variables + usize::from(include_intercept),
        output_cells,
    )?;
    let model = fit(&known, include_intercept, logarithmic, |work| {
        engine.charge_function_iterations(context, work)
    })?;
    materialize_statistics(model, known.variables, rows, cols, logarithmic)
}

fn prediction_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    logarithmic: bool,
) -> Result<Array, ErrorKind> {
    let Some(known_y) = args.first() else {
        return Err(ErrorKind::Value);
    };
    let known = normalize_known(engine, context, known_y, args.get(1))?;
    let prediction = normalize_prediction(engine, context, &known, args.get(2))?;
    let include_intercept = optional_logical(engine, context, args.get(3), true)?;
    let output_cells = cell_count(prediction.rows, prediction.cols)?;
    ensure_regression_workspace(
        engine,
        known.samples,
        known.variables + usize::from(include_intercept),
        output_cells,
    )?;
    let model = fit(&known, include_intercept, logarithmic, |work| {
        engine.charge_function_iterations(context, work)
    })?;
    let values = predict(&model, &prediction, logarithmic, |work| {
        engine.charge_function_iterations(context, work)
    })?;
    Ok(Array {
        rows: prediction.rows,
        cols: prediction.cols,
        data: values.into_iter().map(Value::Number).collect(),
    })
}

fn ensure_regression_workspace(
    engine: &Engine<'_>,
    samples: usize,
    parameters: usize,
    output_cells: u64,
) -> Result<(), ErrorKind> {
    let samples = u64::try_from(samples).map_err(|_| ErrorKind::Num)?;
    let parameters = u64::try_from(parameters).map_err(|_| ErrorKind::Num)?;
    let design = samples.checked_mul(parameters).ok_or(ErrorKind::Num)?;
    let square = parameters.checked_mul(parameters).ok_or(ErrorKind::Num)?;
    let workspace = design
        .checked_mul(2)
        .and_then(|value| value.checked_add(samples.checked_mul(5)?))
        .and_then(|value| value.checked_add(square.checked_mul(2)?))
        .and_then(|value| value.checked_add(parameters.checked_mul(8)?))
        .and_then(|value| value.checked_add(output_cells))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(workspace)
}

fn materialize_statistics(
    model: RegressionModel,
    variables: usize,
    rows: u32,
    cols: u32,
    logarithmic: bool,
) -> Result<Array, ErrorKind> {
    let cells = cell_count(rows, cols)?;
    let mut data = vec![Value::Error(ErrorKind::NA); cells as usize];
    for output_col in 0..variables {
        let input_col = variables - output_col - 1;
        let mut coefficient = if model.active_slopes[input_col] {
            model.slopes[input_col]
        } else {
            0.0
        };
        if logarithmic {
            coefficient = coefficient.exp();
        }
        if !coefficient.is_finite() {
            return Err(ErrorKind::Num);
        }
        data[output_col] = Value::Number(coefficient);
        if rows == 5 {
            data[cols as usize + output_col] = result_value(model.slope_standard_errors[input_col]);
        }
    }
    let intercept_col = variables;
    let mut intercept = if model.intercept_active {
        model.intercept
    } else {
        0.0
    };
    if logarithmic {
        intercept = intercept.exp();
    }
    if !intercept.is_finite() {
        return Err(ErrorKind::Num);
    }
    data[intercept_col] = Value::Number(intercept);
    if rows == 5 {
        let width = cols as usize;
        data[width + intercept_col] = result_value(model.intercept_standard_error);
        data[2 * width] = result_value(model.r_squared);
        data[2 * width + 1] = result_value(model.standard_error_y);
        data[3 * width] = result_value(model.f_statistic);
        data[3 * width + 1] = Value::Number(model.degrees_of_freedom as f64);
        data[4 * width] = Value::Number(model.regression_sum_squares);
        data[4 * width + 1] = Value::Number(model.residual_sum_squares);
    }
    Ok(Array { rows, cols, data })
}

fn result_value(result: Result<f64, ErrorKind>) -> Value {
    match result {
        Ok(value) if value.is_finite() => Value::Number(value),
        Ok(_) => Value::Error(ErrorKind::Num),
        Err(kind) => Value::Error(kind),
    }
}
