/// `ACCRINTM` — accrued interest for a security that pays interest at maturity.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::{annual_denominator, days_between};
use super::{coerce_basis, coerce_date, date_from_serial_arg, finite_number};

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let issue = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let settlement = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[2]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let par_is_absent = args.get(3).is_none();
    let par = match args.get(3) {
        Some(Expr::Missing) => 1000.0,
        None => 0.0,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
    };
    let basis = match coerce_basis(engine, context, args.get(4)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    if issue >= settlement {
        return Value::Error(ErrorKind::Num);
    }
    if rate <= 0.0 || (!par_is_absent && par <= 0.0) {
        return Value::Error(ErrorKind::Num);
    }
    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let accrued_days = days_between(issue_date, settlement_date, basis);
    let denominator = annual_denominator(basis, issue_date, settlement_date);
    finite_number(par * rate * accrued_days / denominator)
}
