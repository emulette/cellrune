use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};

pub(super) fn validate_array_input(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    array: &Array,
) -> Result<(), ErrorKind> {
    engine.charge_function_iterations(context, cell_count(array.rows, array.cols)?)?;
    for value in &array.data {
        poll_cancellation(context)?;
        if let Value::Error(kind) = value
            && kind.is_engine_issue()
        {
            return Err(*kind);
        }
    }
    Ok(())
}

pub(super) fn cell_count(rows: u32, cols: u32) -> Result<u64, ErrorKind> {
    u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)
}

pub(super) fn poll_cancellation(context: EvalContext<'_>) -> Result<(), ErrorKind> {
    if context.is_cancelled() {
        Err(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ))
    } else {
        Ok(())
    }
}
