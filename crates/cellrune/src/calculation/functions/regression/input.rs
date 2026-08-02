use super::super::super::ast::Expr;
use super::super::super::coerce::to_number;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::runtime::Array;
use super::super::super::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Single,
    MultiColumn,
    MultiRow,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedRegressionInput {
    pub(super) response: Vec<f64>,
    pub(super) predictors: Vec<f64>,
    pub(super) samples: usize,
    pub(super) variables: usize,
    orientation: Orientation,
    known_rows: u32,
    known_cols: u32,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedPredictionInput {
    pub(super) predictors: Vec<f64>,
    pub(super) samples: usize,
    pub(super) rows: u32,
    pub(super) cols: u32,
}

pub(super) fn normalize_known(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    known_y: &Expr,
    known_x: Option<&Expr>,
) -> Result<NormalizedRegressionInput, ErrorKind> {
    let y = evaluated_array(engine, context, known_y)?;
    let samples = y.array.data.len();
    if samples == 0 {
        return Err(ErrorKind::Value);
    }
    let Some(known_x) = present(known_x) else {
        let y_rows = y.array.rows;
        let y_cols = y.array.cols;
        return Ok(NormalizedRegressionInput {
            response: y.into_numbers()?,
            predictors: (1..=samples).map(|value| value as f64).collect(),
            samples,
            variables: 1,
            orientation: Orientation::Single,
            known_rows: y_rows,
            known_cols: y_cols,
        });
    };
    let x = evaluated_array(engine, context, known_x)?;
    let y_rows = y.array.rows;
    let y_cols = y.array.cols;
    let x_rows = x.array.rows;
    let x_cols = x.array.cols;
    if x_rows == y_rows && x_cols == y_cols {
        return Ok(NormalizedRegressionInput {
            response: y.into_numbers()?,
            predictors: x.into_numbers()?,
            samples,
            variables: 1,
            orientation: Orientation::Single,
            known_rows: y_rows,
            known_cols: y_cols,
        });
    }
    if y_cols == 1 && x_rows == y_rows {
        return Ok(NormalizedRegressionInput {
            response: y.into_numbers()?,
            predictors: x.into_numbers()?,
            samples,
            variables: x_cols as usize,
            orientation: Orientation::MultiColumn,
            known_rows: y_rows,
            known_cols: y_cols,
        });
    }
    if y_rows == 1 && x_cols == y_cols {
        let response = y.into_numbers()?;
        let x_data = x.into_numbers()?;
        let variables = x_rows as usize;
        let mut predictors = Vec::with_capacity(x_data.len());
        for sample in 0..samples {
            for variable in 0..variables {
                predictors.push(x_data[variable * samples + sample]);
            }
        }
        return Ok(NormalizedRegressionInput {
            response,
            predictors,
            samples,
            variables,
            orientation: Orientation::MultiRow,
            known_rows: y_rows,
            known_cols: y_cols,
        });
    }
    Err(ErrorKind::Ref)
}

pub(super) fn normalize_prediction(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    known: &NormalizedRegressionInput,
    new_x: Option<&Expr>,
) -> Result<NormalizedPredictionInput, ErrorKind> {
    let Some(new_x) = present(new_x) else {
        engine.ensure_array_cells(known.predictors.len() as u64)?;
        return Ok(NormalizedPredictionInput {
            predictors: known.predictors.clone(),
            samples: known.samples,
            rows: known.known_rows,
            cols: known.known_cols,
        });
    };
    let x = evaluated_array(engine, context, new_x)?;
    match known.orientation {
        Orientation::Single => Ok(NormalizedPredictionInput {
            samples: x.array.data.len(),
            rows: x.array.rows,
            cols: x.array.cols,
            predictors: x.into_numbers()?,
        }),
        Orientation::MultiColumn if x.array.cols as usize == known.variables => {
            let rows = x.array.rows;
            Ok(NormalizedPredictionInput {
                samples: rows as usize,
                predictors: x.into_numbers()?,
                rows,
                cols: 1,
            })
        }
        Orientation::MultiRow if x.array.rows as usize == known.variables => {
            let cols = x.array.cols;
            let x_data = x.into_numbers()?;
            let prediction_samples = cols as usize;
            let mut predictors = Vec::with_capacity(x_data.len());
            for sample in 0..prediction_samples {
                for variable in 0..known.variables {
                    predictors.push(x_data[variable * prediction_samples + sample]);
                }
            }
            Ok(NormalizedPredictionInput {
                samples: prediction_samples,
                predictors,
                rows: 1,
                cols,
            })
        }
        Orientation::MultiColumn | Orientation::MultiRow => Err(ErrorKind::Ref),
    }
}

pub(super) fn optional_logical(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expression: Option<&Expr>,
    default: bool,
) -> Result<bool, ErrorKind> {
    match present(expression) {
        Some(expression) => {
            super::super::super::coerce::to_logical(&engine.eval_scalar(context, expression))
        }
        None => Ok(default),
    }
}

fn present(expression: Option<&Expr>) -> Option<&Expr> {
    expression.filter(|expression| !matches!(expression, Expr::Missing))
}

struct EvaluatedArray {
    array: Array,
    scalar_coercion: bool,
}

impl EvaluatedArray {
    fn into_numbers(self) -> Result<Vec<f64>, ErrorKind> {
        coerce_cells(self.array.data, self.scalar_coercion)
    }
}

fn evaluated_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expression: &Expr,
) -> Result<EvaluatedArray, ErrorKind> {
    let array = engine.eval_array(context, expression)?;
    let scalar_coercion = array.is_scalar() && is_scalar_expression(expression);
    engine.ensure_array_cells(array.data.len() as u64)?;
    engine.charge_function_iterations(context, array.data.len() as u64)?;
    Ok(EvaluatedArray {
        array,
        scalar_coercion,
    })
}

fn coerce_cells(values: Vec<Value>, scalar_coercion: bool) -> Result<Vec<f64>, ErrorKind> {
    values
        .into_iter()
        .map(|value| {
            if scalar_coercion {
                to_number(&value)
            } else {
                match value {
                    Value::Number(number) if number.is_finite() => Ok(number),
                    Value::Error(kind) => Err(kind),
                    Value::Number(_) => Err(ErrorKind::Num),
                    Value::Blank | Value::Text(_) | Value::Logical(_) => Err(ErrorKind::Value),
                }
            }
        })
        .collect()
}

fn is_scalar_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => is_scalar_expression(inner),
        Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. }
        | Expr::SpillRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Range { .. }
        | Expr::Name(_)
        | Expr::Array(_)
        | Expr::Missing => false,
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::BuiltinCallable(_)
        | Expr::Call { .. }
        | Expr::Invoke { .. }
        | Expr::Unary { .. }
        | Expr::Binary { .. } => true,
    }
}

#[cfg(test)]
mod tests {
    use super::coerce_cells;
    use crate::calculation::value::{ErrorKind, Value};

    #[test]
    fn regression_coercion_table_distinguishes_scalars_and_collections() {
        assert_eq!(
            coerce_cells(vec![Value::Text("2".into())], true),
            Ok(vec![2.0])
        );
        assert_eq!(
            coerce_cells(vec![Value::Logical(true)], true),
            Ok(vec![1.0])
        );
        assert_eq!(coerce_cells(vec![Value::Blank], true), Ok(vec![0.0]));
        for value in [Value::Text("2".into()), Value::Logical(true), Value::Blank] {
            assert_eq!(coerce_cells(vec![value], false), Err(ErrorKind::Value));
        }
        assert_eq!(
            coerce_cells(vec![Value::Error(ErrorKind::Ref)], false),
            Err(ErrorKind::Ref)
        );
    }
}
