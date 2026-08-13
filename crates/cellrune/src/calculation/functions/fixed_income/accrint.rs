/// `ACCRINT` — periodic accrued interest over a quasi-coupon schedule.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::days_between;
use super::model::{CouponFrequency, DayCountBasis};
use super::schedule::{add_months, estimated_periods, is_end_of_month, normal_period_days};
use super::{
    charge_work, check_cancellation, coerce_basis, coerce_date, coerce_frequency,
    date_from_serial_arg, finite_number, poll_loop_cancellation,
};

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 6 || args.len() > 8 {
        return Value::Error(ErrorKind::Value);
    }
    let issue = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let first_interest = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let settlement = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let par = match args.get(4) {
        Some(Expr::Missing) | None => 1000.0,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
    };
    let frequency = match coerce_frequency(engine, context, &args[5]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(6)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    let calc_method = match args.get(7) {
        None => true,
        Some(Expr::Missing) => true,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number != 0.0,
            Err(kind) => return Value::Error(kind),
        },
    };

    if issue >= settlement {
        return Value::Error(ErrorKind::Num);
    }
    if rate <= 0.0 || par <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let first_interest_date = match date_from_serial_arg(first_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let work_end = settlement_date.max(first_interest_date);
    let Some(work) = estimated_periods(issue_date, work_end, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work) {
        return Value::Error(kind);
    }

    let mut poll = || check_cancellation(context);
    let fraction = match accrued_fraction(
        issue_date,
        first_interest_date,
        settlement_date,
        frequency,
        basis,
        calc_method,
        &mut poll,
    ) {
        Ok(Some(fraction)) => fraction,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };

    finite_number(par * rate / frequency.as_f64() * fraction)
}

fn accrued_fraction(
    issue: super::super::calendar::Date,
    first_interest: super::super::calendar::Date,
    settlement: super::super::calendar::Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
    calc_method: bool,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Option<f64>, ErrorKind> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(first_interest);
    let mut period_end = first_interest;
    let mut periods = 0_usize;
    let mut period_start = loop {
        periods = periods.saturating_add(1);
        poll_loop_cancellation(periods, poll)?;
        let Some(prior) = add_months(period_end, -months, end_of_month) else {
            return Ok(None);
        };
        if prior <= issue {
            break prior;
        }
        period_end = prior;
    };

    let mut fraction = 0.0;
    loop {
        periods = periods.saturating_add(1);
        poll_loop_cancellation(periods, poll)?;
        let Some(normal_days) =
            normal_period_days(period_start, months, basis, frequency, end_of_month)
        else {
            return Ok(None);
        };
        let accrued_end = period_end.min(settlement);
        let accrued_start = period_start.max(issue);
        let issue_period = period_start <= issue && issue < period_end;
        if accrued_start < accrued_end && (calc_method || !issue_period) {
            fraction += if accrued_start == period_start && accrued_end == period_end {
                1.0
            } else {
                days_between(accrued_start, accrued_end, basis) / normal_days
            };
        }
        if period_end >= settlement {
            break;
        }
        period_start = period_end;
        let Some(next_period_end) = add_months(period_end, months, end_of_month) else {
            return Ok(None);
        };
        period_end = next_period_end;
    }
    Ok(Some(fraction))
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::*;
    use crate::{
        CalculationCellId, CalculationCellResult, CalculationHints, CellAddress, CellContent,
        CellValue, DateSystem, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
        Provenance, ProviderIdentity, SavedResult, Sheet, SheetId, SheetName, SheetVisibility,
        WorkbookSnapshot, WorkbookSource,
    };

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn documented_long_first_quasi_coupon_sum_is_segmented() {
        let fraction = accrued_fraction(
            date(2007, 3, 1),
            date(2008, 8, 31),
            date(2008, 5, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            true,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let interest = 1_000.0 * 0.1 / 2.0 * fraction;
        assert!(
            (interest - 116.944_444_444_444).abs() < 1e-12,
            "actual interest: {interest}"
        );
    }

    #[test]
    fn documented_false_method_excludes_the_issue_quasi_period() {
        let fraction = accrued_fraction(
            date(2007, 3, 1),
            date(2008, 8, 31),
            date(2008, 5, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            false,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        let interest = 1_000.0 * 0.1 / 2.0 * fraction;
        assert!((interest - 66.944_444_444_444_5).abs() < 1e-12);
    }

    #[test]
    fn calc_method_false_starts_at_first_interest() {
        let fraction = accrued_fraction(
            date(2024, 1, 1),
            date(2024, 7, 1),
            date(2025, 1, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            false,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fraction, 1.0);
    }

    #[test]
    fn calc_method_false_excludes_the_issue_period_before_first_interest() {
        let fraction = accrued_fraction(
            date(2024, 1, 1),
            date(2024, 7, 1),
            date(2024, 4, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            false,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fraction, 0.0);
    }

    #[test]
    fn explicit_missing_calc_method_matches_absent_formula_evaluation() {
        let mut sheet = Sheet::new(
            SheetId::new(1).unwrap(),
            SheetName::new("FixedIncome").unwrap(),
            SheetVisibility::Visible,
        );
        for (address, formula) in [
            (
                "A1",
                "ACCRINT(DATE(2007,3,1),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0)",
            ),
            (
                "A2",
                "ACCRINT(DATE(2007,3,1),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0,)",
            ),
        ] {
            sheet
                .insert_cell(
                    CellAddress::from_a1(address).unwrap(),
                    CellContent::Formula(FormulaCell::new(
                        FormulaDialect::ExcelA1,
                        FormulaText::from_xlsx(formula).unwrap(),
                        SavedResult::Missing,
                        FormulaMetadata::Normal,
                    )),
                )
                .unwrap();
        }
        let workbook = WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("fixed-income-test", "1").unwrap(),
                None,
            ),
        )
        .unwrap();
        let calculation =
            crate::calculation::calculate_workbook(&workbook, crate::CalculationOptions::default());
        let result = |address: &str| {
            calculation.cell(CalculationCellId::new(
                SheetId::new(1).unwrap(),
                CellAddress::from_a1(address).unwrap(),
            ))
        };
        assert_eq!(result("A1"), result("A2"));
        let Some(CalculationCellResult::Value(CellValue::Number(value))) = result("A1") else {
            panic!("expected numeric ACCRINT result");
        };
        assert!((value.get() - 116.944_444_444_444).abs() < 1e-12);
    }
}
