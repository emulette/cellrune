//! Fixed-income securities function family (0.1.15).
//!
//! Worksheet adapters coerce raw scalar arguments to typed dates, basis codes and frequencies,
//! then delegate to the shared schedule and cash-flow kernels below. No adapter duplicates
//! day-count, coupon schedule, or pricing arithmetic.

mod accrint;
mod accrintm;
mod coupon;
mod day_count;
mod discount;
mod model;
mod odd_price;
mod odd_schedule;
mod odd_yield;
mod regular_bond;
mod schedule;
mod solver;
mod treasury_bill;
mod yield_;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::{ErrorKind, Value};
use super::calendar::{Date, date_from_serial};
use super::kernel::FixedIncomeFunction;
use super::util::required_number;
use model::{CouponFrequency, DayCountBasis};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: FixedIncomeFunction,
    args: &[Expr],
) -> Value {
    match function {
        FixedIncomeFunction::Accrint => accrint::call(engine, context, args),
        FixedIncomeFunction::Accrintm => accrintm::call(engine, context, args),
        FixedIncomeFunction::CoupDayBs => coupon::coup_day_bs(engine, context, args),
        FixedIncomeFunction::CoupDays => coupon::coup_days(engine, context, args),
        FixedIncomeFunction::CoupDaysNc => coupon::coup_days_nc(engine, context, args),
        FixedIncomeFunction::CoupNcd => coupon::coup_ncd(engine, context, args),
        FixedIncomeFunction::CoupNum => coupon::coup_num(engine, context, args),
        FixedIncomeFunction::CoupPcd => coupon::coup_pcd(engine, context, args),
        FixedIncomeFunction::Disc => discount::disc(engine, context, args),
        FixedIncomeFunction::Duration => regular_bond::duration(engine, context, args),
        FixedIncomeFunction::IntRate => discount::int_rate(engine, context, args),
        FixedIncomeFunction::MDuration => regular_bond::m_duration(engine, context, args),
        FixedIncomeFunction::OddFPrice => odd_price::odd_f_price(engine, context, args),
        FixedIncomeFunction::OddFYield => odd_yield::call(engine, context, args),
        FixedIncomeFunction::OddLPrice => odd_price::odd_l_price(engine, context, args),
        FixedIncomeFunction::OddLYield => odd_price::odd_l_yield(engine, context, args),
        FixedIncomeFunction::Price => regular_bond::price(engine, context, args),
        FixedIncomeFunction::PriceDisc => discount::price_disc(engine, context, args),
        FixedIncomeFunction::PriceMat => discount::price_mat(engine, context, args),
        FixedIncomeFunction::Received => discount::received(engine, context, args),
        FixedIncomeFunction::TbillEq => treasury_bill::tbill_eq(engine, context, args),
        FixedIncomeFunction::TbillPrice => treasury_bill::tbill_price(engine, context, args),
        FixedIncomeFunction::TbillYield => treasury_bill::tbill_yield(engine, context, args),
        FixedIncomeFunction::Yield => yield_::call(engine, context, args),
        FixedIncomeFunction::YieldDisc => discount::yield_disc(engine, context, args),
        FixedIncomeFunction::YieldMat => discount::yield_mat(engine, context, args),
    }
}

pub(super) fn coerce_date(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<f64, ErrorKind> {
    let serial = required_number(engine, context, expr)?.trunc();
    if date_from_serial(serial, engine.date_system()).is_none() {
        return Err(ErrorKind::Value);
    }
    Ok(serial)
}

pub(in crate::calculation::functions::fixed_income) fn coerce_basis(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<DayCountBasis, ErrorKind> {
    let code = match expr {
        Some(expr) => required_number(engine, context, expr)?.trunc() as i32,
        None => 0,
    };
    DayCountBasis::from_code(code).ok_or(ErrorKind::Num)
}

pub(in crate::calculation::functions::fixed_income) fn coerce_frequency(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<CouponFrequency, ErrorKind> {
    let code = required_number(engine, context, expr)?.trunc() as i32;
    CouponFrequency::from_code(code).ok_or(ErrorKind::Num)
}

pub(super) fn date_from_serial_arg(serial: f64, engine: &Engine<'_>) -> Result<Date, ErrorKind> {
    date_from_serial(serial, engine.date_system()).ok_or(ErrorKind::Value)
}

/// Discount a set of `(period_time, cash_flow)` pairs and return the dirty value together with the
/// time-weighted cash-flow sum `Σ t·CF·q^(-t)`. Macaulay duration and the analytic yield
/// derivative both derive from this single pure pass.
pub(super) fn cash_flow_reduction(
    flows: &[(f64, f64)],
    frequency: f64,
    yield_rate: f64,
) -> (f64, f64) {
    let log_q = (yield_rate / frequency).ln_1p();
    let mut value = 0.0;
    let mut time_weighted = 0.0;
    for (time, cash_flow) in flows {
        let discount = (-time * log_q).exp();
        value += cash_flow * discount;
        time_weighted += time * cash_flow * discount;
    }
    (value, time_weighted)
}

pub(super) fn charge_work(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    units: usize,
) -> Result<(), ErrorKind> {
    if context.is_cancelled() {
        return Err(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ));
    }
    engine.charge_function_iterations(context, units.max(1) as u64)
}

pub(super) fn finite_number(value: f64) -> Value {
    if value.is_finite() {
        Value::Number(value)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

pub(in crate::calculation::functions::fixed_income) fn coupon_amount(
    rate: f64,
    frequency: CouponFrequency,
) -> f64 {
    100.0 * rate / frequency.as_f64()
}
