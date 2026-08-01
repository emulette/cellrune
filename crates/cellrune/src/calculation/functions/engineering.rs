use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::EngineeringFunction;
use super::util::{required_number, required_text};

const MAX_BIT_VALUE: u64 = (1_u64 << 48) - 1;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: EngineeringFunction,
    args: &[Expr],
) -> Value {
    match function {
        EngineeringFunction::BitAnd => {
            bit_binary(engine, context, args, |left, right| left & right)
        }
        EngineeringFunction::BitOr => bit_binary(engine, context, args, |left, right| left | right),
        EngineeringFunction::BitXor => {
            bit_binary(engine, context, args, |left, right| left ^ right)
        }
        EngineeringFunction::BitLShift => bit_shift(engine, context, args, true),
        EngineeringFunction::BitRShift => bit_shift(engine, context, args, false),
        EngineeringFunction::Bin2Dec => {
            convert_source(engine, context, args, SourceRadix::Binary, None)
        }
        EngineeringFunction::Bin2Hex => convert_source(
            engine,
            context,
            args,
            SourceRadix::Binary,
            Some(TargetRadix::Hex),
        ),
        EngineeringFunction::Bin2Oct => convert_source(
            engine,
            context,
            args,
            SourceRadix::Binary,
            Some(TargetRadix::Octal),
        ),
        EngineeringFunction::Hex2Bin => convert_source(
            engine,
            context,
            args,
            SourceRadix::Hex,
            Some(TargetRadix::Binary),
        ),
        EngineeringFunction::Hex2Dec => {
            convert_source(engine, context, args, SourceRadix::Hex, None)
        }
        EngineeringFunction::Hex2Oct => convert_source(
            engine,
            context,
            args,
            SourceRadix::Hex,
            Some(TargetRadix::Octal),
        ),
        EngineeringFunction::Oct2Bin => convert_source(
            engine,
            context,
            args,
            SourceRadix::Octal,
            Some(TargetRadix::Binary),
        ),
        EngineeringFunction::Oct2Dec => {
            convert_source(engine, context, args, SourceRadix::Octal, None)
        }
        EngineeringFunction::Oct2Hex => convert_source(
            engine,
            context,
            args,
            SourceRadix::Octal,
            Some(TargetRadix::Hex),
        ),
        EngineeringFunction::Dec2Bin => convert_decimal(engine, context, args, TargetRadix::Binary),
        EngineeringFunction::Dec2Hex => convert_decimal(engine, context, args, TargetRadix::Hex),
        EngineeringFunction::Dec2Oct => convert_decimal(engine, context, args, TargetRadix::Octal),
        EngineeringFunction::Delta => comparison(engine, context, args, true),
        EngineeringFunction::GeStep => comparison(engine, context, args, false),
        EngineeringFunction::Erf => erf(engine, context, args, false),
        EngineeringFunction::ErfPrecise => erf(engine, context, args, true),
        EngineeringFunction::Erfc | EngineeringFunction::ErfcPrecise => erfc(engine, context, args),
    }
}

fn bit_binary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(u64, u64) -> u64,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    match (
        required_bit_value(engine, context, &args[0]),
        required_bit_value(engine, context, &args[1]),
    ) {
        (Ok(left), Ok(right)) => Value::Number(operation(left, right) as f64),
        (Err(kind), _) | (_, Err(kind)) => Value::Error(kind),
    }
}

fn bit_shift(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], left: bool) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_bit_value(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let shift = match required_number(engine, context, &args[1]) {
        Ok(shift) if shift.abs().trunc() <= 53.0 => shift.trunc() as i32,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let shift_left = left == (shift >= 0);
    let amount = shift.unsigned_abs();
    let result = if shift_left {
        number
            .checked_shl(amount)
            .filter(|result| *result <= MAX_BIT_VALUE)
    } else {
        number.checked_shr(amount)
    };
    result.map_or(Value::Error(ErrorKind::Num), |result| {
        Value::Number(result as f64)
    })
}

fn required_bit_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<u64, ErrorKind> {
    match required_number(engine, context, expr)? {
        number if number >= 0.0 && number <= MAX_BIT_VALUE as f64 && number.fract() == 0.0 => {
            Ok(number as u64)
        }
        _ => Err(ErrorKind::Num),
    }
}

#[derive(Debug, Clone, Copy)]
enum SourceRadix {
    Binary,
    Octal,
    Hex,
}

impl SourceRadix {
    const fn radix(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Hex => 16,
        }
    }

    const fn bits(self) -> u32 {
        match self {
            Self::Binary => 10,
            Self::Octal => 30,
            Self::Hex => 40,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TargetRadix {
    Binary,
    Octal,
    Hex,
}

impl TargetRadix {
    const fn radix(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Hex => 16,
        }
    }

    const fn bits(self) -> u32 {
        match self {
            Self::Binary => 10,
            Self::Octal => 30,
            Self::Hex => 40,
        }
    }
}

fn convert_source(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    source: SourceRadix,
    target: Option<TargetRadix>,
) -> Value {
    let valid_len = if target.is_some() {
        (1..=2).contains(&args.len())
    } else {
        args.len() == 1
    };
    if !valid_len {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let value = match parse_source(&text, source) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match target {
        Some(target) => format_target(engine, context, args.get(1), value, target),
        None => Value::Number(value as f64),
    }
}

fn parse_source(text: &str, source: SourceRadix) -> Result<i64, ErrorKind> {
    if text.is_empty() || text.len() > 10 {
        return Err(ErrorKind::Num);
    }
    let raw = u64::from_str_radix(text, source.radix()).map_err(|_| ErrorKind::Num)?;
    let bits = source.bits();
    if raw >= (1_u64 << bits) {
        return Err(ErrorKind::Num);
    }
    if text.len() == 10 && raw & (1_u64 << (bits - 1)) != 0 {
        Ok(raw as i64 - (1_i64 << bits))
    } else {
        Ok(raw as i64)
    }
}

fn convert_decimal(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    target: TargetRadix,
) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let value = match required_number(engine, context, &args[0]) {
        Ok(value) => value.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    format_target(engine, context, args.get(1), value, target)
}

fn format_target(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    places: Option<&Expr>,
    value: i64,
    target: TargetRadix,
) -> Value {
    let bits = target.bits();
    let minimum = -(1_i64 << (bits - 1));
    let maximum = (1_i64 << (bits - 1)) - 1;
    if value < minimum || value > maximum {
        return Value::Error(ErrorKind::Num);
    }
    let places = match places {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(places) if (1.0..=10.0).contains(&places.trunc()) => places.trunc() as usize,
            Ok(_) => return Value::Error(ErrorKind::Num),
            Err(kind) => return Value::Error(kind),
        },
        None => 0,
    };
    let raw = if value < 0 {
        (1_i64 << bits) + value
    } else {
        value
    };
    let encoded = encode_unsigned(raw as u64, target.radix());
    let width = if value < 0 { 10 } else { places };
    if width != 0 && encoded.len() > width {
        return Value::Error(ErrorKind::Num);
    }
    let padded = if encoded.len() < width {
        format!("{}{}", "0".repeat(width - encoded.len()), encoded)
    } else {
        encoded
    };
    engine.bounded_text(padded)
}

fn encode_unsigned(mut number: u64, radix: u32) -> String {
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

fn comparison(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], equal: bool) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let comparison = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
        None => 0.0,
    };
    Value::Number(f64::from(if equal {
        number == comparison
    } else {
        number >= comparison
    }))
}

fn erf(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], precise: bool) -> Value {
    let valid_len = if precise {
        args.len() == 1
    } else {
        (1..=2).contains(&args.len())
    };
    if !valid_len {
        return Value::Error(ErrorKind::Value);
    }
    let lower = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let result = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(upper) => libm::erf(upper) - libm::erf(lower),
            Err(kind) => return Value::Error(kind),
        },
        None => libm::erf(lower),
    };
    Value::Number(result)
}

fn erfc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_number(engine, context, &args[0]) {
        Ok(number) => Value::Number(libm::erfc(number)),
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::{SourceRadix, parse_source};

    #[test]
    fn ten_digit_sources_use_the_excel_twos_complement_width() {
        assert_eq!(parse_source("1111111111", SourceRadix::Binary), Ok(-1));
        assert_eq!(parse_source("7777777777", SourceRadix::Octal), Ok(-1));
        assert_eq!(parse_source("FFFFFFFFFF", SourceRadix::Hex), Ok(-1));
    }
}
