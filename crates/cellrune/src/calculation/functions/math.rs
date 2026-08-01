use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::MathFunction;
use super::util::{collect_argument_values, required_number, required_text};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: MathFunction,
    args: &[Expr],
) -> Value {
    match function {
        MathFunction::Abs => unary(engine, context, args, f64::abs),
        MathFunction::Int => unary(engine, context, args, f64::floor),
        MathFunction::Sign => unary(engine, context, args, excel_sign),
        MathFunction::Exp => unary(engine, context, args, f64::exp),
        MathFunction::Ln => unary_checked(engine, context, args, |number| {
            (number > 0.0).then(|| number.ln())
        }),
        MathFunction::Sqrt => unary_checked(engine, context, args, |number| {
            (number >= 0.0).then(|| number.sqrt())
        }),
        MathFunction::Round => round(engine, context, args, RoundMode::Nearest),
        MathFunction::RoundDown | MathFunction::Trunc => {
            round(engine, context, args, RoundMode::TowardZero)
        }
        MathFunction::RoundUp => round(engine, context, args, RoundMode::AwayFromZero),
        MathFunction::Mod => checked_binary(engine, context, args, excel_mod),
        MathFunction::Power => checked_binary(engine, context, args, excel_power),
        MathFunction::Ceiling => multiple(engine, context, args, true),
        MathFunction::Floor => multiple(engine, context, args, false),
        MathFunction::Even => parity_round(engine, context, args, false),
        MathFunction::Odd => parity_round(engine, context, args, true),
        MathFunction::Log => logarithm(engine, context, args),
        MathFunction::Log10 => unary_checked(engine, context, args, |number| {
            (number > 0.0).then(|| number.log10())
        }),
        MathFunction::MRound => mround(engine, context, args),
        MathFunction::Pi if args.is_empty() => Value::Number(std::f64::consts::PI),
        MathFunction::Pi => Value::Error(ErrorKind::Value),
        MathFunction::Quotient => {
            checked_binary(engine, context, args, |numerator, denominator| {
                if denominator == 0.0 {
                    Err(ErrorKind::Div0)
                } else {
                    Ok((numerator / denominator).trunc())
                }
            })
        }
        MathFunction::SqrtPi => unary_checked(engine, context, args, |number| {
            (number >= 0.0).then(|| (number * std::f64::consts::PI).sqrt())
        }),
        MathFunction::CeilingMath => {
            modern_multiple(engine, context, args, ModernMultiple::CeilingMath)
        }
        MathFunction::CeilingPrecise | MathFunction::IsoCeiling => {
            modern_multiple(engine, context, args, ModernMultiple::CeilingPrecise)
        }
        MathFunction::FloorMath => {
            modern_multiple(engine, context, args, ModernMultiple::FloorMath)
        }
        MathFunction::FloorPrecise => {
            modern_multiple(engine, context, args, ModernMultiple::FloorPrecise)
        }
        MathFunction::Base => base(engine, context, args),
        MathFunction::Decimal => decimal(engine, context, args),
        MathFunction::SeriesSum => series_sum(engine, context, args),
    }
}

fn unary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(f64) -> f64,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_number(engine, context, &args[0]) {
        Ok(number) => finite(operation(number)),
        Err(kind) => Value::Error(kind),
    }
}

fn unary_checked(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(f64) -> Option<f64>,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_number(engine, context, &args[0]) {
        Ok(number) => operation(number).map_or(Value::Error(ErrorKind::Num), finite),
        Err(kind) => Value::Error(kind),
    }
}

fn checked_binary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(f64, f64) -> Result<f64, ErrorKind>,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let left = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let right = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    operation(left, right).map_or_else(Value::Error, finite)
}

fn excel_sign(number: f64) -> f64 {
    if number == 0.0 { 0.0 } else { number.signum() }
}

fn excel_mod(number: f64, divisor: f64) -> Result<f64, ErrorKind> {
    if divisor == 0.0 {
        Err(ErrorKind::Div0)
    } else {
        Ok(number - divisor * (number / divisor).floor())
    }
}

fn excel_power(base: f64, exponent: f64) -> Result<f64, ErrorKind> {
    if base == 0.0 && exponent == 0.0 {
        Err(ErrorKind::Num)
    } else {
        Ok(base.powf(exponent))
    }
}

#[derive(Debug, Clone, Copy)]
enum RoundMode {
    Nearest,
    TowardZero,
    AwayFromZero,
}

fn round(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], mode: RoundMode) -> Value {
    let valid_len = match mode {
        RoundMode::TowardZero => args.len() == 1 || args.len() == 2,
        RoundMode::Nearest | RoundMode::AwayFromZero => args.len() == 2,
    };
    if !valid_len {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let digits = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
        None => 0,
    };
    let scale = 10_f64.powi(digits.abs());
    let scaled = if digits >= 0 {
        number * scale
    } else {
        number / scale
    };
    let rounded = match mode {
        RoundMode::Nearest => scaled.round(),
        RoundMode::TowardZero => scaled.trunc(),
        RoundMode::AwayFromZero => {
            if scaled.is_sign_negative() {
                scaled.floor()
            } else {
                scaled.ceil()
            }
        }
    };
    finite(if digits >= 0 {
        rounded / scale
    } else {
        rounded * scale
    })
}

fn multiple(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], ceiling: bool) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let significance_value = engine.eval_scalar(context, &args[1]);
    if matches!(significance_value, Value::Blank) {
        return Value::Number(0.0);
    }
    let significance = match to_number(&significance_value) {
        Ok(significance) if significance == 0.0 && number == 0.0 => {
            return Value::Number(0.0);
        }
        Ok(0.0) => return Value::Error(ErrorKind::Div0),
        Ok(significance) => significance,
        Err(kind) => return Value::Error(kind),
    };
    if number > 0.0 && significance < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let magnitude = significance.abs();
    let quotient = number / magnitude;
    let rounded = if ceiling {
        if number >= 0.0 || significance > 0.0 {
            quotient.ceil()
        } else {
            quotient.floor()
        }
    } else if number >= 0.0 || significance > 0.0 {
        quotient.floor()
    } else {
        quotient.ceil()
    };
    finite(rounded * magnitude)
}

fn parity_round(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], odd: bool) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let rounded = number.abs().ceil();
    let is_odd = rounded.rem_euclid(2.0) == 1.0;
    let magnitude = if is_odd == odd {
        rounded
    } else {
        rounded + 1.0
    };
    finite(if number.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    })
}

fn logarithm(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let base = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
        None => 10.0,
    };
    if number <= 0.0 || base <= 0.0 || base == 1.0 {
        Value::Error(ErrorKind::Num)
    } else {
        finite(number.log(base))
    }
}

fn mround(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let significance = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    if significance == 0.0 {
        return Value::Number(0.0);
    }
    if number != 0.0 && number.is_sign_negative() != significance.is_sign_negative() {
        return Value::Error(ErrorKind::Num);
    }
    finite((number / significance).round() * significance)
}

#[derive(Debug, Clone, Copy)]
enum ModernMultiple {
    CeilingMath,
    CeilingPrecise,
    FloorMath,
    FloorPrecise,
}

fn modern_multiple(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: ModernMultiple,
) -> Value {
    let math_variant = matches!(
        operation,
        ModernMultiple::CeilingMath | ModernMultiple::FloorMath
    );
    let valid_len = if math_variant {
        (1..=3).contains(&args.len())
    } else {
        (1..=2).contains(&args.len())
    };
    if !valid_len {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let significance = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.abs(),
            Err(kind) => return Value::Error(kind),
        },
        None => 1.0,
    };
    if significance == 0.0 {
        return Value::Number(0.0);
    }
    let mode = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number != 0.0,
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    let quotient = number / significance;
    let rounded = match operation {
        ModernMultiple::CeilingPrecise => quotient.ceil(),
        ModernMultiple::FloorPrecise => quotient.floor(),
        ModernMultiple::CeilingMath if number < 0.0 && mode => quotient.floor(),
        ModernMultiple::CeilingMath => quotient.ceil(),
        ModernMultiple::FloorMath if number < 0.0 && mode => quotient.ceil(),
        ModernMultiple::FloorMath => quotient.floor(),
    };
    finite(rounded * significance)
}

fn base(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) if (0.0..9_007_199_254_740_992.0).contains(&number) => number.trunc() as u64,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let radix = match required_number(engine, context, &args[1]) {
        Ok(number) if (2.0..=36.0).contains(&number.trunc()) => number.trunc() as u32,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let minimum_length = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if (0.0..=255.0).contains(&number.trunc()) => number.trunc() as usize,
            Ok(_) => return Value::Error(ErrorKind::Num),
            Err(kind) => return Value::Error(kind),
        },
        None => 0,
    };
    let mut encoded = encode_radix(number, radix);
    if encoded.len() < minimum_length {
        encoded.insert_str(0, &"0".repeat(minimum_length - encoded.len()));
    }
    engine.bounded_text(encoded)
}

fn encode_radix(mut number: u64, radix: u32) -> String {
    if number == 0 {
        return "0".to_owned();
    }
    let mut digits = Vec::new();
    while number > 0 {
        let digit = (number % u64::from(radix)) as u8;
        digits.push(if digit < 10 {
            char::from(b'0' + digit)
        } else {
            char::from(b'A' + digit - 10)
        });
        number /= u64::from(radix);
    }
    digits.into_iter().rev().collect()
}

fn decimal(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) if !text.is_empty() && text.len() <= 255 => text,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let radix = match required_number(engine, context, &args[1]) {
        Ok(number) if (2.0..=36.0).contains(&number.trunc()) => number.trunc() as u32,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let mut result = 0.0;
    for character in text.chars() {
        let Some(digit) = character.to_digit(radix) else {
            return Value::Error(ErrorKind::Num);
        };
        result = result * f64::from(radix) + f64::from(digit);
        if !result.is_finite() {
            return Value::Error(ErrorKind::Num);
        }
    }
    Value::Number(result)
}

fn series_sum(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let initial_power = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let step = match required_number(engine, context, &args[2]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let coefficients = match collect_argument_values(engine, context, &args[3..]) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut result = 0.0;
    for (index, item) in coefficients.into_iter().enumerate() {
        match item.value {
            Value::Number(coefficient) => {
                result += coefficient * x.powf(initial_power + index as f64 * step);
            }
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    finite(result)
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
    use super::{excel_mod, excel_power, excel_sign};
    use crate::calculation::value::ErrorKind;

    #[test]
    fn mod_uses_the_divisor_sign() {
        assert_eq!(excel_mod(-3.0, 2.0), Ok(1.0));
        assert_eq!(excel_mod(3.0, -2.0), Ok(-1.0));
        assert_eq!(excel_mod(3.0, 0.0), Err(ErrorKind::Div0));
    }

    #[test]
    fn sign_and_power_handle_excel_zero_boundaries() {
        assert_eq!(excel_sign(0.0), 0.0);
        assert_eq!(excel_sign(-0.0), 0.0);
        assert_eq!(excel_power(0.0, 0.0), Err(ErrorKind::Num));
    }
}
