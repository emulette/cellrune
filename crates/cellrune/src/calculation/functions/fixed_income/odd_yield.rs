/// `ODDFYIELD` — safeguarded inverse of the odd-first price kernel.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::odd_price::odd_first_flows;
use super::odd_schedule::odd_first_measures;
use super::schedule::estimated_periods;
use super::solver::{EXCEL_YIELD_POLICY, EXTENDED_YIELD_POLICY, solve};
use super::{
    cash_flow_reduction_with_poll, charge_work, check_cancellation, coerce_basis, coerce_date,
    coerce_frequency, date_from_serial_arg, finite_number,
};
use crate::FinancialSolverSemantics;

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 8 || args.len() > 9 {
        return Value::Error(ErrorKind::Value);
    }
    let settlement = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let maturity = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let issue = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let first_coupon = match coerce_date(engine, context, &args[3]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let price = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[6]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[7]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(8)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > first_coupon && first_coupon > settlement && settlement > issue) {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let maturity_date = match date_from_serial_arg(maturity, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let first_coupon_date = match date_from_serial_arg(first_coupon, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(work) = estimated_periods(issue_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work.saturating_mul(3)) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measures = match odd_first_measures(
        issue_date,
        first_coupon_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
        &mut poll,
    ) {
        Ok(Some(measures)) => measures,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let flows = match odd_first_flows(&measures, rate, redemption, frequency, &mut poll) {
        Ok(flows) => flows,
        Err(kind) => return Value::Error(kind),
    };

    let frequency_value = frequency.as_f64();
    let lower_bound = -frequency_value;
    let policy = match engine.financial_solver_semantics() {
        FinancialSolverSemantics::ExcelIterationBudget => EXCEL_YIELD_POLICY,
        FinancialSolverSemantics::ExtendedSearch => EXTENDED_YIELD_POLICY,
    };
    let accrued_interest = flows.accrued_interest;
    let residual = |yield_: f64| -> Result<(f64, f64), ErrorKind> {
        let (value, time_weighted) =
            cash_flow_reduction_with_poll(&flows.flows, frequency_value, yield_, &mut || {
                check_cancellation(context)
            })?;
        let q = 1.0 + yield_ / frequency_value;
        let clean = value - accrued_interest;
        let derivative = -time_weighted / (frequency_value * q);
        Ok((clean - price, derivative))
    };
    match solve(
        engine,
        context,
        lower_bound,
        flows.flows.len(),
        policy,
        &residual,
    ) {
        Ok(yield_) => finite_number(yield_),
        Err(kind) => Value::Error(kind),
    }
}
