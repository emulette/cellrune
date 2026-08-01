use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::CombinatoricsFunction;
use super::util::{excel_numeric_arguments, required_number};

const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
const MAX_EXCEL_ARGUMENTS: usize = 255;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: CombinatoricsFunction,
    args: &[Expr],
) -> Value {
    match function {
        CombinatoricsFunction::Fact => factorial_function(engine, context, args, false),
        CombinatoricsFunction::FactDouble => factorial_function(engine, context, args, true),
        CombinatoricsFunction::Gcd => gcd_lcm(engine, context, args, false),
        CombinatoricsFunction::Lcm => gcd_lcm(engine, context, args, true),
        CombinatoricsFunction::Combin => selection(engine, context, args, Selection::Combination),
        CombinatoricsFunction::Combina => {
            selection(engine, context, args, Selection::CombinationWithRepetition)
        }
        CombinatoricsFunction::Permut => selection(engine, context, args, Selection::Permutation),
        CombinatoricsFunction::PermutationA => {
            selection(engine, context, args, Selection::PermutationWithRepetition)
        }
        CombinatoricsFunction::Multinomial => multinomial(engine, context, args),
    }
}

fn factorial_function(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    double: bool,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_nonnegative_integer(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let step = if double { 2 } else { 1 };
    finite(factorial_step(number, step))
}

fn factorial_step(number: u64, step: u64) -> f64 {
    let mut result = 1.0;
    let mut factor = number;
    while factor > 1 {
        result *= factor as f64;
        if !result.is_finite() {
            break;
        }
        factor = factor.saturating_sub(step);
    }
    result
}

fn gcd_lcm(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    calculate_lcm: bool,
) -> Value {
    if args.is_empty() || args.len() > MAX_EXCEL_ARGUMENTS {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match excel_numeric_arguments(engine, context, args) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    let mut result = None;
    for number in numbers {
        let number = match exact_nonnegative_integer(number) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        };
        result = Some(if calculate_lcm {
            match result.map_or(Some(number), |current| lcm(current, number)) {
                Some(result) if (result as f64) < MAX_EXACT_INTEGER => result,
                Some(_) | None => return Value::Error(ErrorKind::Num),
            }
        } else {
            result.map_or(number, |current| gcd(current, number))
        });
    }
    Value::Number(result.unwrap_or(0) as f64)
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn lcm(left: u64, right: u64) -> Option<u64> {
    if left == 0 || right == 0 {
        Some(0)
    } else {
        (left / gcd(left, right)).checked_mul(right)
    }
}

#[derive(Debug, Clone, Copy)]
enum Selection {
    Combination,
    CombinationWithRepetition,
    Permutation,
    PermutationWithRepetition,
}

fn selection(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    selection: Selection,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_nonnegative_integer(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let chosen = match required_nonnegative_integer(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let result = match selection {
        Selection::Combination if chosen <= number => combination(engine, number, chosen),
        Selection::Combination => Err(ErrorKind::Num),
        Selection::CombinationWithRepetition if number == 0 && chosen > 0 => Err(ErrorKind::Num),
        Selection::CombinationWithRepetition if chosen == 0 => Ok(1.0),
        Selection::CombinationWithRepetition => {
            let expanded = match number
                .checked_add(chosen)
                .and_then(|sum| sum.checked_sub(1))
            {
                Some(expanded) => expanded,
                None => return Value::Error(ErrorKind::Num),
            };
            combination(engine, expanded, chosen)
        }
        Selection::Permutation if chosen <= number => permutation(number, chosen),
        Selection::Permutation => Err(ErrorKind::Num),
        Selection::PermutationWithRepetition if number == 0 && chosen > 0 => Err(ErrorKind::Num),
        Selection::PermutationWithRepetition => Ok((number as f64).powf(chosen as f64)),
    };
    result.map_or_else(Value::Error, finite)
}

fn combination(engine: &Engine<'_>, number: u64, chosen: u64) -> Result<f64, ErrorKind> {
    let count = chosen.min(number - chosen);
    if count > engine.max_function_iterations() {
        return Err(ErrorKind::ResourceLimit(
            super::super::limits::CalculationLimitKind::FunctionIterations,
        ));
    }
    let mut result = 1.0;
    for index in 1..=count {
        result *= (number - count + index) as f64;
        result /= index as f64;
        if !result.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(result.round())
}

fn permutation(number: u64, chosen: u64) -> Result<f64, ErrorKind> {
    let mut result = 1.0;
    for offset in 0..chosen {
        result *= (number - offset) as f64;
        if !result.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(result)
}

fn multinomial(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > MAX_EXCEL_ARGUMENTS {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match excel_numeric_arguments(engine, context, args) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    let mut total = 0_u64;
    let mut result = 1.0;
    for number in numbers {
        let number = match exact_nonnegative_integer(number) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        };
        let Some(next_total) = total.checked_add(number) else {
            return Value::Error(ErrorKind::Num);
        };
        let factor = match combination(engine, next_total, number) {
            Ok(factor) => factor,
            Err(kind) => return Value::Error(kind),
        };
        result *= factor;
        if !result.is_finite() {
            return Value::Error(ErrorKind::Num);
        }
        total = next_total;
    }
    Value::Number(result.round())
}

fn required_nonnegative_integer(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<u64, ErrorKind> {
    exact_nonnegative_integer(required_number(engine, context, expr)?)
}

fn exact_nonnegative_integer(number: f64) -> Result<u64, ErrorKind> {
    if !(0.0..MAX_EXACT_INTEGER).contains(&number) {
        Err(ErrorKind::Num)
    } else {
        Ok(number.trunc() as u64)
    }
}

fn finite(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::{factorial_step, gcd, lcm};

    #[test]
    fn integer_kernels_cover_zero_and_double_factorial() {
        assert_eq!(factorial_step(0, 1), 1.0);
        assert_eq!(factorial_step(5, 1), 120.0);
        assert_eq!(factorial_step(10, 2), 3_840.0);
        assert_eq!(gcd(24, 36), 12);
        assert_eq!(lcm(4, 6), Some(12));
        assert_eq!(lcm(0, 6), Some(0));
    }
}
