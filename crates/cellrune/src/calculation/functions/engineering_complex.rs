use super::super::ast::Expr;
use super::super::coerce::to_text;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::complex::{ComplexSuffix, ComplexValue, EXCEL_COMPLEX_NUMBER_BOUNDARY};
use super::util::{collect_argument_values, required_number, required_text};

pub(super) fn construct(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if !(2..=3).contains(&args.len()) {
        return Value::Error(ErrorKind::Value);
    }
    let real = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let imaginary = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    // Excel rejects COMPLEX components at 1E308 even though that boundary is
    // still representable by the engine's wider finite-number container.
    if real.abs() >= EXCEL_COMPLEX_NUMBER_BOUNDARY
        || imaginary.abs() >= EXCEL_COMPLEX_NUMBER_BOUNDARY
    {
        return Value::Error(ErrorKind::Num);
    }
    let suffix = match args.get(2) {
        Some(argument) => {
            let text = match required_text(engine, context, argument) {
                Ok(text) => text,
                Err(kind) => return Value::Error(kind),
            };
            match ComplexSuffix::from_text(&text) {
                Some(suffix) => suffix,
                None => return Value::Error(ErrorKind::Value),
            }
        }
        None => ComplexSuffix::I,
    };
    complex_text(engine, ComplexValue::new(real, imaginary, suffix))
}

pub(super) fn magnitude(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let value = match unary_complex(engine, context, args) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let magnitude = value.magnitude();
    if magnitude.is_finite() {
        Value::Number(magnitude)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

pub(super) fn imaginary(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match unary_complex(engine, context, args) {
        Ok(value) => Value::Number(value.imaginary()),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn argument(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match unary_complex(engine, context, args) {
        Ok(value) if value.real() == 0.0 && value.imaginary() == 0.0 => {
            Value::Error(ErrorKind::Div0)
        }
        Ok(value) => Value::Number(value.argument()),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn conjugate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args).map(ComplexValue::conjugate),
    )
}

pub(super) fn real(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match unary_complex(engine, context, args) {
        Ok(value) => Value::Number(value.real()),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn divide(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let (left, right) = match binary_complex(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    complex_text(engine, left.divide(right))
}

pub(super) fn power(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let value = match complex_argument(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let exponent = match required_number(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    // Excel rejects exponents beyond the integer-power kernel's range rather
    // than treating a large integral value as a fractional polar exponent.
    if exponent.abs() >= i64::MAX as f64 {
        return Value::Error(ErrorKind::Num);
    }
    let result = value.power(exponent, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    });
    complex_text(engine, result)
}

pub(super) fn product(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    collection(engine, context, args, ComplexValue::product)
}

pub(super) fn subtract(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let (left, right) = match binary_complex(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    complex_text(engine, left.subtract(right))
}

pub(super) fn sum(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    collection(engine, context, args, ComplexValue::sum)
}

pub(super) fn exponential(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args).and_then(ComplexValue::exponential),
    )
}

pub(super) fn logarithm(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args).and_then(ComplexValue::logarithm),
    )
}

pub(super) fn square_root(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args).and_then(ComplexValue::square_root),
    )
}

fn unary_complex(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ComplexValue, ErrorKind> {
    if args.len() != 1 {
        return Err(ErrorKind::Value);
    }
    complex_argument(engine, context, &args[0])
}

fn binary_complex(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<(ComplexValue, ComplexValue), ErrorKind> {
    if args.len() != 2 {
        return Err(ErrorKind::Value);
    }
    Ok((
        complex_argument(engine, context, &args[0])?,
        complex_argument(engine, context, &args[1])?,
    ))
}

fn complex_argument(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<ComplexValue, ErrorKind> {
    ComplexValue::parse(&required_text(engine, context, argument)?)
}

fn collection(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(&[ComplexValue]) -> Result<ComplexValue, ErrorKind>,
) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut complex = Vec::with_capacity(values.len());
    let mut collection_blanks = 0_usize;
    for value in values {
        if let Err(kind) =
            poll_cancellation(context).and_then(|()| engine.charge_function_iterations(context, 1))
        {
            return Value::Error(kind);
        }
        if matches!(&value.value, Value::Blank)
            && value.from_collection
            && !value.from_single_cell_reference
        {
            // Excel treats a blank inside a multi-cell collection as complex
            // zero, while a direct blank or blank single-cell reference is an
            // invalid inumber. Delay the zero so it inherits an observed j
            // suffix instead of forcing the collection to i.
            collection_blanks += 1;
            continue;
        }
        let text = match to_text(&value.value) {
            Ok(text) => text,
            Err(kind) => return Value::Error(kind),
        };
        match ComplexValue::parse(&text) {
            Ok(value) => complex.push(value),
            Err(kind) => return Value::Error(kind),
        }
    }
    if collection_blanks > 0 {
        let suffix = complex
            .first()
            .map(|value| value.suffix())
            .unwrap_or(ComplexSuffix::I);
        complex.extend((0..collection_blanks).map(|_| ComplexValue::zero(suffix)));
    }
    complex_text(engine, operation(&complex))
}

fn complex_text(engine: &Engine<'_>, result: Result<ComplexValue, ErrorKind>) -> Value {
    match result {
        Ok(value) => engine.bounded_text(value.format()),
        Err(kind) => Value::Error(kind),
    }
}
