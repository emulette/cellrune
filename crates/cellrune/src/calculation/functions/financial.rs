use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::calendar::date_from_serial;
use super::util::{
    collect_argument_values, collect_argument_values_with_counter, excel_sum, required_number,
};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "PMT" => pmt(engine, context, args),
        "FV" => fv(engine, context, args),
        "PV" => pv(engine, context, args),
        "NPER" => nper(engine, context, args),
        "IPMT" => ipmt(engine, context, args),
        "PPMT" => ppmt(engine, context, args),
        "NPV" => npv(engine, context, args),
        "IRR" => irr(engine, context, args),
        "XIRR" => xirr(engine, context, args),
        "RATE" => rate(engine, context, args),
        "SLN" => sln(engine, context, args),
        "SYD" => syd(engine, context, args),
        "DB" => db(engine, context, args),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn pmt(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 5) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    financial_value(payment(
        values[0], values[1], values[2], values[3], values[4],
    ))
}

fn fv(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 5) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (rate, periods, payment, present, payment_type) =
        (values[0], values[1], values[2], values[3], values[4]);
    let result = if rate == 0.0 {
        -(present + payment * periods)
    } else {
        let power = (1.0 + rate).powf(periods);
        -(present * power + payment * (1.0 + rate * payment_type) * (power - 1.0) / rate)
    };
    financial_value(result)
}

fn pv(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 5) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (rate, periods, payment, future, payment_type) =
        (values[0], values[1], values[2], values[3], values[4]);
    let result = if rate == 0.0 {
        -future - payment * periods
    } else {
        let power = (1.0 + rate).powf(periods);
        -(future + payment * (1.0 + rate * payment_type) * (power - 1.0) / rate) / power
    };
    financial_value(result)
}

fn nper(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 5) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (rate, payment, present, future, payment_type) =
        (values[0], values[1], values[2], values[3], values[4]);
    let result = if rate == 0.0 {
        -(present + future) / payment
    } else {
        let adjusted_payment = payment * (1.0 + rate * payment_type) / rate;
        ((adjusted_payment - future) / (present + adjusted_payment)).ln() / (1.0 + rate).ln()
    };
    financial_value(result)
}

fn ipmt(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 4 || args.len() > 6 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 6) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (rate, period, periods, present, future, payment_type) = (
        values[0], values[1], values[2], values[3], values[4], values[5],
    );
    if period < 1.0 || period > periods || periods <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    if rate == 0.0 || (payment_type != 0.0 && period == 1.0) {
        return Value::Number(0.0);
    }
    let payment = payment(rate, periods, present, future, payment_type);
    if !payment.is_finite() {
        return Value::Error(ErrorKind::Num);
    }
    let elapsed = period - 1.0;
    let power = (1.0 + rate).powf(elapsed);
    let balance = if payment_type == 0.0 {
        present * power + payment * (power - 1.0) / rate
    } else {
        present * power + payment * (1.0 + rate) * (power - 1.0) / rate
    };
    financial_value(-balance * rate)
}

fn ppmt(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let interest = ipmt(engine, context, args);
    let Value::Number(interest) = interest else {
        return interest;
    };
    let mut payment_args = Vec::with_capacity(5);
    payment_args.push(args[0].clone());
    payment_args.push(args[2].clone());
    payment_args.push(args[3].clone());
    if let Some(future) = args.get(4) {
        payment_args.push(future.clone());
    }
    if let Some(payment_type) = args.get(5) {
        if payment_args.len() == 3 {
            payment_args.push(Expr::Number(0.0));
        }
        payment_args.push(payment_type.clone());
    }
    match pmt(engine, context, &payment_args) {
        Value::Number(payment) => financial_value(payment - interest),
        other => other,
    }
}

fn npv(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 {
        return Value::Error(ErrorKind::Value);
    }
    let rate = match required_number(engine, context, &args[0]) {
        Ok(rate) if rate != -1.0 => rate,
        Ok(_) => return Value::Error(ErrorKind::Div0),
        Err(kind) => return Value::Error(kind),
    };
    let cashflows = match numeric_cashflows(engine, context, &args[1..]) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let total = excel_sum(
        cashflows
            .iter()
            .enumerate()
            .map(|(index, value)| value / (1.0 + rate).powi(index as i32 + 1)),
    );
    financial_value(total)
}

fn irr(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let cashflows = match numeric_cashflows(engine, context, &args[..1]) {
        Ok(values)
            if values.iter().any(|value| *value > 0.0)
                && values.iter().any(|value| *value < 0.0) =>
        {
            values
        }
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let guess = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
        None => 0.1,
    };
    solve_newton(engine, cashflows.len(), guess, |rate| {
        let mut value = 0.0;
        let mut derivative = 0.0;
        for (period, cashflow) in cashflows.iter().enumerate() {
            let power = (1.0 + rate).powi(period as i32);
            value += cashflow / power;
            if period > 0 {
                derivative -= period as f64 * cashflow / (1.0 + rate).powi(period as i32 + 1);
            }
        }
        (value, derivative)
    })
}

fn xirr(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let mut visited_cells = 0_u64;
    let cashflow_items = match collect_argument_values_with_counter(
        engine,
        context,
        std::slice::from_ref(&args[0]),
        &mut visited_cells,
    ) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let date_items = match collect_argument_values_with_counter(
        engine,
        context,
        std::slice::from_ref(&args[1]),
        &mut visited_cells,
    ) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    if cashflow_items.is_empty() || cashflow_items.len() != date_items.len() {
        return Value::Error(ErrorKind::Num);
    }

    let mut cashflows = Vec::with_capacity(cashflow_items.len());
    for item in cashflow_items {
        match item.value {
            Value::Number(number) => cashflows.push(number),
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {
                return Value::Error(ErrorKind::Value);
            }
        }
    }
    if !cashflows.iter().any(|value| *value > 0.0) || !cashflows.iter().any(|value| *value < 0.0) {
        return Value::Error(ErrorKind::Num);
    }

    let mut dates = Vec::with_capacity(date_items.len());
    for item in date_items {
        let date = match item.value {
            Value::Number(number) => number.trunc(),
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {
                return Value::Error(ErrorKind::Value);
            }
        };
        if date_from_serial(date, engine.date_system()).is_none() {
            return Value::Error(ErrorKind::Value);
        }
        dates.push(date);
    }
    let start_date = dates[0];
    if dates.iter().any(|date| *date < start_date) {
        return Value::Error(ErrorKind::Num);
    }

    let guess = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
        None => 0.1,
    };
    solve_newton(engine, cashflows.len(), guess, |rate| {
        let base = 1.0 + rate;
        let mut value = 0.0;
        let mut derivative = 0.0;
        for (cashflow, date) in cashflows.iter().zip(&dates) {
            let years = (*date - start_date) / 365.0;
            value += cashflow / base.powf(years);
            if years != 0.0 {
                derivative -= years * cashflow / base.powf(years + 1.0);
            }
        }
        (value, derivative)
    })
}

fn rate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 6 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 6) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (periods, payment, present, future, payment_type, guess) = (
        values[0],
        values[1],
        values[2],
        values[3],
        values[4],
        if args.len() == 6 { values[5] } else { 0.1 },
    );
    solve_newton(engine, 1, guess, |rate| {
        let power = (1.0 + rate).powf(periods);
        let factor = if rate.abs() < 1e-12 {
            periods
        } else {
            (power - 1.0) / rate
        };
        let value = present * power + payment * (1.0 + rate * payment_type) * factor + future;
        let step = 1e-7;
        let next_power = (1.0 + rate + step).powf(periods);
        let next_factor = (next_power - 1.0) / (rate + step);
        let next_value = present * next_power
            + payment * (1.0 + (rate + step) * payment_type) * next_factor
            + future;
        (value, (next_value - value) / step)
    })
}

fn sln(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    depreciation(engine, context, args, false)
}

fn syd(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    depreciation(engine, context, args, true)
}

fn depreciation(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sum_of_years: bool,
) -> Value {
    let expected = if sum_of_years { 4 } else { 3 };
    if args.len() != expected {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, expected) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (cost, salvage, life) = (values[0], values[1], values[2]);
    if life <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let result = if sum_of_years {
        let period = values[3];
        (cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0))
    } else {
        (cost - salvage) / life
    };
    financial_value(result)
}

fn db(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 4 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match scalar_arguments(engine, context, args, 5) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let (cost, salvage, life, period) = (values[0], values[1], values[2], values[3]);
    let months = if args.len() == 5 {
        values[4].trunc()
    } else {
        12.0
    };
    if cost <= 0.0 || salvage < 0.0 || life <= 0.0 || period < 1.0 || months <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let rate = ((1.0 - (salvage / cost).powf(1.0 / life)) * 1000.0).round() / 1000.0;
    let mut book = cost;
    let mut depreciation = 0.0;
    let periods = period.trunc() as u64;
    if let Err(kind) = engine.ensure_function_iterations(periods) {
        return Value::Error(kind);
    }
    let life_periods = life.trunc() as u64;
    for current in 1..=periods {
        depreciation = if current == 1 {
            cost * rate * months / 12.0
        } else if current == life_periods.saturating_add(1) {
            book * rate * (12.0 - months) / 12.0
        } else {
            book * rate
        };
        book -= depreciation;
    }
    financial_value(depreciation)
}

fn payment(rate: f64, periods: f64, present: f64, future: f64, payment_type: f64) -> f64 {
    if rate == 0.0 {
        -(present + future) / periods
    } else {
        let power = (1.0 + rate).powf(periods);
        -(future + present * power) * rate / ((1.0 + rate * payment_type) * (power - 1.0))
    }
}

pub(super) fn scalar_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    target_len: usize,
) -> Result<Vec<f64>, ErrorKind> {
    let mut values = Vec::with_capacity(target_len);
    for arg in args {
        values.push(required_number(engine, context, arg)?);
    }
    values.resize(target_len, 0.0);
    Ok(values)
}

fn numeric_cashflows(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<f64>, ErrorKind> {
    let mut values = Vec::new();
    for item in collect_argument_values(engine, context, args)? {
        match item.value {
            Value::Number(number) => values.push(number),
            Value::Error(kind) => return Err(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    Ok(values)
}

fn solve_newton(
    engine: &Engine<'_>,
    work_per_iteration: usize,
    mut guess: f64,
    function: impl Fn(f64) -> (f64, f64),
) -> Value {
    let Ok(work_per_iteration) = u64::try_from(work_per_iteration.max(1)) else {
        return Value::Error(ErrorKind::Num);
    };
    for iteration in 1_u64..=100 {
        let Some(work) = work_per_iteration.checked_mul(iteration) else {
            return Value::Error(ErrorKind::Num);
        };
        if let Err(kind) = engine.ensure_function_iterations(work) {
            return Value::Error(kind);
        }
        if guess <= -1.0 {
            guess = -0.999_999;
        }
        let (value, derivative) = function(guess);
        if !value.is_finite() || !derivative.is_finite() || derivative.abs() < 1e-14 {
            return Value::Error(ErrorKind::Num);
        }
        let next = guess - value / derivative;
        if (next - guess).abs() <= 1e-10 {
            return financial_value(next);
        }
        guess = next;
    }
    Value::Error(ErrorKind::Num)
}

pub(super) fn financial_value(value: f64) -> Value {
    if value.is_finite() {
        Value::Number(value)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
