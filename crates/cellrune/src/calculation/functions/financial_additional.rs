use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::financial::{financial_value, scalar_arguments};
use super::kernel::FinancialAdditionalFunction;
use super::util::{collect_argument_values, required_number};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: FinancialAdditionalFunction,
    args: &[Expr],
) -> Value {
    match function {
        FinancialAdditionalFunction::DollarDe => dollar_fraction(engine, context, args, false),
        FinancialAdditionalFunction::DollarFr => dollar_fraction(engine, context, args, true),
        FinancialAdditionalFunction::Effect => annual_rate(engine, context, args, false),
        FinancialAdditionalFunction::Nominal => annual_rate(engine, context, args, true),
        FinancialAdditionalFunction::Rri => rri(engine, context, args),
        FinancialAdditionalFunction::PDuration => pduration(engine, context, args),
        FinancialAdditionalFunction::IsPmt => ispmt(engine, context, args),
        FinancialAdditionalFunction::FvSchedule => fvschedule(engine, context, args),
        FinancialAdditionalFunction::Mirr => mirr(engine, context, args),
    }
}

fn dollar_fraction(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    to_fraction: bool,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let value = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let denominator = match required_number(engine, context, &args[1]) {
        Ok(value) if value.trunc() == 0.0 => return Value::Error(ErrorKind::Div0),
        Ok(value) if value.trunc() < 0.0 => return Value::Error(ErrorKind::Num),
        Ok(value) => value.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let digits = denominator.log10().ceil() as i32;
    let scale = 10_f64.powi(digits);
    let integer = value.trunc();
    let fraction = value - integer;
    financial_value(if to_fraction {
        integer + fraction * denominator / scale
    } else {
        integer + fraction * scale / denominator
    })
}

fn annual_rate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    nominal: bool,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let rate = match required_number(engine, context, &args[0]) {
        Ok(rate) if rate > 0.0 => rate,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let periods = match required_number(engine, context, &args[1]) {
        Ok(periods) if periods >= 1.0 => periods.trunc(),
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    financial_value(if nominal {
        periods * ((1.0 + rate).powf(1.0 / periods) - 1.0)
    } else {
        (1.0 + rate / periods).powf(periods) - 1.0
    })
}

fn rri(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let periods = match required_number(engine, context, &args[0]) {
        Ok(periods) if periods > 0.0 => periods,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let present = match required_number(engine, context, &args[1]) {
        Ok(present) if present != 0.0 => present,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let future = match required_number(engine, context, &args[2]) {
        Ok(future) => future,
        Err(kind) => return Value::Error(kind),
    };
    financial_value((future / present).powf(1.0 / periods) - 1.0)
}

fn pduration(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let rate = match required_number(engine, context, &args[0]) {
        Ok(rate) if rate > 0.0 => rate,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let present = match required_number(engine, context, &args[1]) {
        Ok(present) if present > 0.0 => present,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let future = match required_number(engine, context, &args[2]) {
        Ok(future) if future > 0.0 => future,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    financial_value((future / present).ln() / (1.0 + rate).ln())
}

fn ispmt(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 4) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    if values[2] == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    financial_value(-values[3] * values[0] * (1.0 - values[1] / values[2]))
}

fn fvschedule(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let principal = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let schedule = match collect_argument_values(engine, context, &args[1..]) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut result = principal;
    for item in schedule {
        match item.value {
            Value::Number(rate) => result *= 1.0 + rate,
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank => {}
            Value::Text(_) | Value::Logical(_) => return Value::Error(ErrorKind::Value),
        }
        if !result.is_finite() {
            return Value::Error(ErrorKind::Num);
        }
    }
    Value::Number(result)
}

fn mirr(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values(engine, context, &args[..1]) {
        Ok(values) if values.len() >= 2 => values,
        Ok(_) => return Value::Error(ErrorKind::Div0),
        Err(kind) => return Value::Error(kind),
    };
    let finance_rate = match required_number(engine, context, &args[1]) {
        Ok(rate) => rate,
        Err(kind) => return Value::Error(kind),
    };
    let reinvest_rate = match required_number(engine, context, &args[2]) {
        Ok(-1.0) => return Value::Error(ErrorKind::Div0),
        Ok(rate) => rate,
        Err(kind) => return Value::Error(kind),
    };
    let periods = values.len();
    let mut present_negative = 0.0;
    let mut future_positive = 0.0;
    for (period, item) in values.into_iter().enumerate() {
        match item.value {
            Value::Number(value) if value < 0.0 => {
                if finance_rate == -1.0 && period > 0 {
                    return Value::Error(ErrorKind::Div0);
                }
                present_negative += value / (1.0 + finance_rate).powi(period as i32);
            }
            Value::Number(value) if value > 0.0 => {
                future_positive +=
                    value * (1.0 + reinvest_rate).powi((periods - period - 1) as i32);
            }
            Value::Number(_) | Value::Blank | Value::Text(_) | Value::Logical(_) => {}
            Value::Error(kind) => return Value::Error(kind),
        }
    }
    if present_negative == 0.0 || future_positive == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    financial_value((-future_positive / present_negative).powf(1.0 / (periods - 1) as f64) - 1.0)
}
