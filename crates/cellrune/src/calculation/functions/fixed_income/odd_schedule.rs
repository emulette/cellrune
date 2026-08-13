use super::super::calendar::Date;
/// Odd-period schedule construction.
///
/// Odd-first and odd-last securities share the same quasi-coupon day-fraction machinery: build the
/// actual quasi-coupon periods first, then reduce their accrued/coupon/settlement fractions. The
/// odd-last form reduces those fractions for the direct `ODDLPRICE`/`ODDLYIELD` pair; the
/// odd-first form exposes the day measures the discounted cash-flow kernel needs.
use super::model::{CouponFrequency, DayCountBasis};
use super::schedule::{add_months, is_end_of_month, normal_period_days};

#[derive(Debug, Clone, Copy)]
pub(super) struct OddLastMeasures {
    pub(super) accrued_fraction: f64,
    pub(super) coupon_days_fraction: f64,
    pub(super) to_maturity_fraction: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OddFirstMeasures {
    pub(super) accrued_days: f64,
    pub(super) days_to_first_coupon: f64,
    pub(super) first_period_days: f64,
    pub(super) period_days: f64,
    pub(super) coupon_count: i64,
}

pub(super) fn odd_last_measures(
    last_interest: Date,
    settlement: Date,
    maturity: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
) -> Option<OddLastMeasures> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(last_interest);

    let mut accrued_fraction = 0.0;
    let mut coupon_days_fraction = 0.0;
    let mut to_maturity_fraction = 0.0;

    let mut period_start = last_interest;
    loop {
        let period_end = add_months(period_start, months, end_of_month)?.min(maturity);
        let normal_days = normal_period_days(period_start, months, basis, frequency, end_of_month)?;
        let period_days = super::day_count::days_between(period_start, period_end, basis);

        coupon_days_fraction += period_days / normal_days;
        accrued_fraction += accrued_in(period_start, period_end, settlement, basis) / normal_days;
        to_maturity_fraction +=
            settlement_to_end(period_start, period_end, settlement, basis) / normal_days;

        if period_end >= maturity {
            break;
        }
        period_start = period_end;
    }

    Some(OddLastMeasures {
        accrued_fraction,
        coupon_days_fraction,
        to_maturity_fraction,
    })
}

pub(super) fn odd_first_measures(
    issue: Date,
    first_coupon: Date,
    settlement: Date,
    maturity: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
) -> Option<OddFirstMeasures> {
    let months = frequency.months();
    let accrued_days = super::day_count::days_between(issue, settlement, basis);
    let days_to_first_coupon = super::day_count::days_between(settlement, first_coupon, basis);
    let first_period_days = super::day_count::days_between(issue, first_coupon, basis);
    let period_days = normal_period_days(
        first_coupon,
        -months,
        basis,
        frequency,
        is_end_of_month(first_coupon),
    )?;
    let coupon_count = coupon_count(first_coupon, maturity, months);

    Some(OddFirstMeasures {
        accrued_days,
        days_to_first_coupon,
        first_period_days,
        period_days,
        coupon_count,
    })
}

fn accrued_in(period_start: Date, period_end: Date, settlement: Date, basis: DayCountBasis) -> f64 {
    if settlement <= period_start {
        0.0
    } else if settlement >= period_end {
        super::day_count::days_between(period_start, period_end, basis)
    } else {
        super::day_count::days_between(period_start, settlement, basis)
    }
}

fn settlement_to_end(
    period_start: Date,
    period_end: Date,
    settlement: Date,
    basis: DayCountBasis,
) -> f64 {
    if settlement <= period_start {
        super::day_count::days_between(period_start, period_end, basis)
    } else if settlement >= period_end {
        0.0
    } else {
        super::day_count::days_between(settlement, period_end, basis)
    }
}

fn coupon_count(first_coupon: Date, maturity: Date, months: i64) -> i64 {
    let first_index = i64::from(first_coupon.year) * 12 + i64::from(first_coupon.month) - 1;
    let maturity_index = i64::from(maturity.year) * 12 + i64::from(maturity.month) - 1;
    (maturity_index - first_index) / months + 1
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::super::model::{CouponFrequency, DayCountBasis};
    use super::*;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn odd_last_measures_match_frozen_fixture() {
        let measures = odd_last_measures(
            date(2024, 10, 15),
            date(2025, 2, 1),
            date(2025, 6, 15),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert!((measures.accrued_fraction - 106.0 / 180.0).abs() < 1e-12);
        assert!((measures.coupon_days_fraction - 240.0 / 180.0).abs() < 1e-12);
        assert!((measures.to_maturity_fraction - 134.0 / 180.0).abs() < 1e-12);
    }

    #[test]
    fn odd_first_measures_match_frozen_short_fixture() {
        let measures = odd_first_measures(
            date(2024, 11, 15),
            date(2025, 3, 1),
            date(2025, 2, 1),
            date(2030, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert_eq!(measures.accrued_days, 76.0);
        assert_eq!(measures.days_to_first_coupon, 30.0);
        assert_eq!(measures.first_period_days, 106.0);
        assert_eq!(measures.period_days, 180.0);
        assert_eq!(measures.coupon_count, 11);
    }
}
