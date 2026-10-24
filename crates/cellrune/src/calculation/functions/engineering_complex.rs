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

pub(super) fn logarithm_base_10(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    scaled_logarithm(engine, context, args, core::f64::consts::LOG10_E)
}

pub(super) fn logarithm_base_2(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    scaled_logarithm(engine, context, args, core::f64::consts::LOG2_E)
}

/// The principal logarithm in another base: the natural logarithm scaled by 1/ln(base).
fn scaled_logarithm(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    reciprocal_ln_base: f64,
) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args)
            .and_then(ComplexValue::logarithm)
            .and_then(|value| {
                ComplexValue::new(
                    value.real() * reciprocal_ln_base,
                    value.imaginary() * reciprocal_ln_base,
                    value.suffix(),
                )
            }),
    )
}

pub(super) fn square_root(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    complex_text(
        engine,
        unary_complex(engine, context, args).and_then(ComplexValue::square_root),
    )
}

/// The complex trigonometric and hyperbolic functions IMSIN through IMSECH.
#[derive(Debug, Clone, Copy)]
pub(super) enum ComplexTrigonometry {
    Sine,
    Cosine,
    Tangent,
    Cotangent,
    Secant,
    Cosecant,
    HyperbolicSine,
    HyperbolicCosine,
    HyperbolicSecant,
    HyperbolicCosecant,
}

/// Evaluates a complex trigonometric function. Microsoft documents `#VALUE!` for a logical
/// inumber and `#NUM!` for other text that is not a complex number, and Excel returns `#NUM!` at
/// the pole of `IMCOT`, `IMCSC`, and `IMCSCH`, zero. Where the sine, cosine, or hyperbolic
/// counterpart that a bounded function divides by overflows, Excel returns the limit once the
/// remaining parts underflow: `IMTAN("1+400i")` is `i` and `IMSECH("1000")` is `0`. Before they
/// underflow the result is a subnormal number Excel was not measured on, so it is unsupported.
pub(super) fn trigonometry(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    function: ComplexTrigonometry,
) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let value = match engine.eval_scalar(context, argument) {
        Value::Logical(_) => return Value::Error(ErrorKind::Value),
        value => match to_text(&value).and_then(|text| ComplexValue::parse(&text)) {
            Ok(value) => value,
            Err(kind) => return Value::Error(kind),
        },
    };
    let pole = value.real() == 0.0 && value.imaginary() == 0.0;
    if pole
        && matches!(
            function,
            ComplexTrigonometry::Cotangent
                | ComplexTrigonometry::Cosecant
                | ComplexTrigonometry::HyperbolicCosecant
        )
    {
        return Value::Error(ErrorKind::Num);
    }
    let result = match function {
        ComplexTrigonometry::Sine => value.sine(),
        ComplexTrigonometry::Cosine => value.cosine(),
        ComplexTrigonometry::Tangent => value.tangent(),
        ComplexTrigonometry::Cotangent => value.cotangent(),
        ComplexTrigonometry::Secant => reciprocal(value.cosine(), value.imaginary(), value),
        ComplexTrigonometry::Cosecant => reciprocal(value.sine(), value.imaginary(), value),
        ComplexTrigonometry::HyperbolicSine => value.hyperbolic_sine(),
        ComplexTrigonometry::HyperbolicCosine => value.hyperbolic_cosine(),
        ComplexTrigonometry::HyperbolicSecant => {
            reciprocal(value.hyperbolic_cosine(), value.real(), value)
        }
        ComplexTrigonometry::HyperbolicCosecant => {
            reciprocal(value.hyperbolic_sine(), value.real(), value)
        }
    };
    // The bounded functions stay finite where sine, cosine, or cosh overflow, so an overflow left
    // after the limits above is a result Excel was not measured on rather than #NUM!.
    let bounded = !matches!(
        function,
        ComplexTrigonometry::Sine
            | ComplexTrigonometry::Cosine
            | ComplexTrigonometry::HyperbolicSine
            | ComplexTrigonometry::HyperbolicCosine
    );
    match result {
        Err(ErrorKind::Num) if bounded => Value::Error(ErrorKind::Unsupported),
        result => complex_text(engine, result),
    }
}

/// The reciprocal of a sine, cosine, or hyperbolic counterpart of `value`. When that overflowed,
/// its magnitude is at least e^|growth| / 2 for the component `growth` that drives it, so the
/// reciprocal is zero once 2·e^(−|growth|) underflows and `#NUM!` before that.
fn reciprocal(
    divisor: Result<ComplexValue, ErrorKind>,
    growth: f64,
    value: ComplexValue,
) -> Result<ComplexValue, ErrorKind> {
    match divisor {
        Ok(divisor) => divisor.reciprocal(),
        Err(ErrorKind::Num) if 2.0 * libm::exp(-growth.abs()) == 0.0 => {
            Ok(ComplexValue::zero(value.suffix()))
        }
        Err(kind) => Err(kind),
    }
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
