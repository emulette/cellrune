use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
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
        FinancialAdditionalFunction::CumIpmt => {
            cumulative_payment(engine, context, args, PaymentPart::Interest)
        }
        FinancialAdditionalFunction::CumPrinc => {
            cumulative_payment(engine, context, args, PaymentPart::Principal)
        }
        FinancialAdditionalFunction::Vdb => vdb(engine, context, args),
        FinancialAdditionalFunction::AmorDegrc => {
            super::amortization::amordegrc(engine, context, args)
        }
        FinancialAdditionalFunction::AmorLinc => {
            super::amortization::amorlinc(engine, context, args)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PaymentPart {
    Interest,
    Principal,
}

/// CUMIPMT and CUMPRINC(rate, nper, pv, start_period, end_period, type) total the interest or the
/// principal of the level payments from `start_period` through `end_period`. With end-of-period
/// payments and q = 1 + rate, the balance after j payments is pv·(q^n − q^j)/(q^n − 1), or
/// pv·q^j·E(n − j)/E(n) with E(x) = q^x − 1. Over the m periods after the first a payments, the
/// principal is the drop in that balance, pv·q^a·E(m)/E(n), and the interest is rate times the
/// balances the periods start from, pv·rate·q^a·(m·E(n − a) − X(m))/E(n) with
/// X(m) = (E(m) − m·rate)/rate. E is `exp_m1(x·ln_1p(rate))` and X a series for small m·rate, so
/// neither total is the small difference of two large ones, as the interest is when it is taken as
/// the payments minus the principal at a small rate.
///
/// With beginning-of-period payments (type 1) each payment is the end-of-period one divided by q
/// and so is every balance after it, so a period's interest and principal are those of the
/// end-of-period schedule divided by q, except that the first payment, made at the start, is all
/// principal: pv·(rate·q^(n−1))/E(n). This reproduces Excel's results for type 1.
///
/// Excel counts a fractional `start_period` of 1.5 as period 2. That one value cannot tell
/// rounding up from rounding to the nearest period, so a fractional start is accepted only where
/// rounding up, half up, and half to even agree, and other fractional starts and fractional ends
/// are unsupported rather than guessed.
fn cumulative_payment(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    part: PaymentPart,
) -> Value {
    let values = match scalar_arguments(engine, context, args, 6) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let [rate, periods, present, start, end, payment_type] = values[..] else {
        return Value::Error(ErrorKind::Value);
    };
    if rate <= 0.0
        || periods <= 0.0
        || present <= 0.0
        || start < 1.0
        || end < 1.0
        || start > end
        || end > periods
        || (payment_type != 0.0 && payment_type != 1.0)
    {
        return Value::Error(ErrorKind::Num);
    }
    if start.round_ties_even() != start.ceil() || end.fract() != 0.0 {
        return Value::Error(ErrorKind::Unsupported);
    }
    let start = start.ceil();
    let log_growth = rate.ln_1p();
    let growth = |count: f64| (count * log_growth).exp_m1();
    let total = growth(periods);
    if !total.is_finite() {
        return Value::Error(ErrorKind::Num);
    }
    let advance = payment_type == 1.0;
    // The first beginning-of-period payment carries no interest and is all principal.
    let first_payment = advance && start == 1.0;
    let start = if first_payment { 2.0 } else { start };
    let (paid, count) = (start - 1.0, end - start + 1.0);
    let divided_by_growth = if advance { 1.0 } else { 0.0 };
    let scale = ((paid - divided_by_growth) * log_growth).exp();
    financial_value(match part {
        PaymentPart::Principal => {
            let first = if first_payment {
                rate * ((periods - 1.0) * log_growth).exp() / total
            } else {
                0.0
            };
            -present * (first + growth(count) / total * scale)
        }
        PaymentPart::Interest if count == 0.0 => 0.0,
        PaymentPart::Interest => {
            let balances = count * (growth(periods - paid) / total)
                - compound_excess(rate, count, growth(count)) / total;
            -present * rate * balances * scale
        }
    })
}

/// ((1 + rate)^count − 1 − count·rate) / rate for a whole `count`, given the `growth`
/// (1 + rate)^count − 1. Below count·rate = 0.5 it sums the binomial series
/// Σ C(count, k)·rate^(k − 1) from k = 2, whose positive terms shrink at least sixfold, instead of
/// subtracting nearly equal numbers.
fn compound_excess(rate: f64, count: f64, growth: f64) -> f64 {
    if count * rate >= 0.5 {
        return (growth - count * rate) / rate;
    }
    let mut term = count * (count - 1.0) / 2.0 * rate;
    let mut total = 0.0;
    let mut k = 2.0;
    while total + term != total {
        total += term;
        term *= (count - k) / (k + 1.0) * rate;
        k += 1.0;
    }
    total
}

/// VDB(cost, salvage, life, start_period, end_period, [factor], [no_switch]) totals the
/// declining-balance depreciation of the periods after `start_period` through `end_period`. Each
/// period depreciates the book value by factor/life without going below salvage, and unless
/// `no_switch` is TRUE the remaining depreciable amount is spread evenly over the remaining life
/// from the first period in which that straight line exceeds the declining balance.
///
/// An end period beyond the life returns `#NUM!`, as Excel does. Fractional periods follow
/// [`partial_vdb`].
fn vdb(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [numbers @ .., no_switch] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let values = match scalar_arguments(engine, context, numbers, 6) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let [cost, salvage, life, start, end, factor] = values[..] else {
        return Value::Error(ErrorKind::Value);
    };
    let no_switch = match to_logical(&engine.eval_scalar(context, no_switch)) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if cost < 0.0
        || salvage < 0.0
        || life <= 0.0
        || start < 0.0
        || end < start
        || end > life
        || factor <= 0.0
    {
        return Value::Error(ErrorKind::Num);
    }
    if start.fract() != 0.0 || end.fract() != 0.0 {
        if no_switch {
            return Value::Error(ErrorKind::Unsupported);
        }
        return partial_vdb(engine, context, [cost, salvage, life, start, end], factor / life);
    }
    if let Err(kind) = engine.charge_function_iterations(context, end as u64) {
        return Value::Error(kind);
    }
    let rate = factor / life;
    let mut book = cost;
    let mut straight_line = None;
    let mut total = 0.0;
    for period in 1..=end as u64 {
        if let Err(kind) = poll_cancellation(context) {
            return Value::Error(kind);
        }
        let declining = (book * rate).min(book - salvage).max(0.0);
        if !no_switch && straight_line.is_none() {
            let even = (book - salvage) / (life - (period - 1) as f64);
            if even > declining {
                straight_line = Some(even);
            }
        }
        let depreciation = straight_line.unwrap_or(declining);
        if period as f64 > start {
            total += depreciation;
        }
        book -= depreciation;
    }
    financial_value(total)
}

/// VDB with a fractional `start_period` or `end_period`, as Excel 16.114 for Mac computes it: the
/// book value at the start is the cost less the whole periods before it and the started share of
/// the next one, and the depreciation is then taken over periods of one year counted from the
/// start, the last one prorated. Excel returns 432 for `VDB(2400,300,10,0.5,1.5)`, 315 for
/// Microsoft's `VDB(2400,300,10,0,0.875,1.5)`, and 247.5 for `VDB(2400,300,10,0.875,1.5)`, where
/// prorating calendar periods would give 252.
///
/// Those results never stop at salvage or switch to the straight line, so a period that would, or
/// `no_switch` TRUE, is unsupported rather than guessed.
fn partial_vdb(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    [cost, salvage, life, start, end]: [f64; 5],
    rate: f64,
) -> Value {
    let span = end - start;
    let (before, after) = (start.ceil(), span.ceil());
    if let Err(kind) = engine.charge_function_iterations(context, (before + after) as u64) {
        return Value::Error(kind);
    }
    // The declining-balance depreciation of a whole period from `book` with `remaining` years of
    // life, or `None` where it would stop at salvage or the straight line would exceed it.
    let declining = |book: f64, remaining: f64| {
        let amount = book * rate;
        let stops = amount > book - salvage || (book - salvage) / remaining > amount;
        (!stops).then_some(amount)
    };
    let mut book = cost;
    for period in 0..before as u64 {
        if let Err(kind) = poll_cancellation(context) {
            return Value::Error(kind);
        }
        let Some(amount) = declining(book, life - period as f64) else {
            return Value::Error(ErrorKind::Unsupported);
        };
        book -= (start - period as f64).min(1.0) * amount;
    }
    let mut total = 0.0;
    for period in 0..after as u64 {
        if let Err(kind) = poll_cancellation(context) {
            return Value::Error(kind);
        }
        let Some(amount) = declining(book, life - start - period as f64) else {
            return Value::Error(ErrorKind::Unsupported);
        };
        total += (span - period as f64).min(1.0) * amount;
        book -= amount;
    }
    financial_value(total)
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
    // Text, logical values, and empty cells are ignored rather than counted as periods.
    let values = match collect_argument_values(engine, context, &args[..1]).and_then(|values| {
        values
            .into_iter()
            .filter_map(|item| match item.value {
                Value::Number(number) => Some(Ok(number)),
                Value::Error(kind) => Some(Err(kind)),
                Value::Blank | Value::Text(_) | Value::Logical(_) => None,
            })
            .collect::<Result<Vec<_>, _>>()
    }) {
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
    for (period, value) in values.into_iter().enumerate() {
        if value < 0.0 {
            if finance_rate == -1.0 && period > 0 {
                return Value::Error(ErrorKind::Div0);
            }
            present_negative += value / (1.0 + finance_rate).powi(period as i32);
        } else if value > 0.0 {
            future_positive += value * (1.0 + reinvest_rate).powi((periods - period - 1) as i32);
        }
    }
    if present_negative == 0.0 || future_positive == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    financial_value((-future_positive / present_negative).powf(1.0 / (periods - 1) as f64) - 1.0)
}
