use super::super::super::value::ErrorKind;
use super::super::calendar::Date;
/// Odd-period schedule construction.
///
/// Odd-first and odd-last securities share the same quasi-coupon day-fraction machinery: build the
/// actual quasi-coupon periods first, then reduce their accrued/coupon/settlement fractions. The
/// odd-last form reduces those fractions for the direct `ODDLPRICE`/`ODDLYIELD` pair; the
/// odd-first form exposes the day measures the discounted cash-flow kernel needs.
use super::model::{CouponFrequency, DayCountBasis};
use super::poll_loop_cancellation;
use super::schedule::{add_months, is_end_of_month, normal_period_days};

#[derive(Debug, Clone, Copy)]
pub(super) struct OddLastMeasures {
    pub(super) accrued_fraction: f64,
    pub(super) coupon_days_fraction: f64,
    pub(super) to_maturity_fraction: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OddFirstMeasures {
    pub(super) is_long_first: bool,
    pub(super) accrued_fraction: f64,
    pub(super) issue_to_first_fraction: f64,
    pub(super) settlement_to_first_fraction: f64,
    pub(super) coupon_count: i64,
}

pub(super) fn odd_last_measures(
    last_interest: Date,
    settlement: Date,
    maturity: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Option<OddLastMeasures>, ErrorKind> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(last_interest);

    let mut accrued_fraction = 0.0;
    let mut coupon_days_fraction = 0.0;
    let mut to_maturity_fraction = 0.0;

    let mut period_start = last_interest;
    let mut periods = 0_usize;
    loop {
        periods = periods.saturating_add(1);
        poll_loop_cancellation(periods, poll)?;
        let Some(period_end) = add_months(period_start, months, end_of_month) else {
            return Ok(None);
        };
        let period_end = period_end.min(maturity);
        let Some(normal_days) =
            normal_period_days(period_start, months, basis, frequency, end_of_month)
        else {
            return Ok(None);
        };
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

    Ok(Some(OddLastMeasures {
        accrued_fraction,
        coupon_days_fraction,
        to_maturity_fraction,
    }))
}

pub(super) fn odd_first_measures(
    issue: Date,
    first_coupon: Date,
    settlement: Date,
    maturity: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Option<OddFirstMeasures>, ErrorKind> {
    let months = frequency.months();
    let issue_to_first_fraction =
        match quasi_fraction(issue, first_coupon, first_coupon, frequency, basis, poll)? {
            Some(fraction) => fraction,
            None => return Ok(None),
        };
    let accrued_fraction =
        match quasi_fraction(issue, settlement, first_coupon, frequency, basis, poll)? {
            Some(fraction) => fraction,
            None => return Ok(None),
        };
    let settlement_to_first_fraction = match quasi_fraction(
        settlement,
        first_coupon,
        first_coupon,
        frequency,
        basis,
        poll,
    )? {
        Some(fraction) => fraction,
        None => return Ok(None),
    };
    let coupon_count = coupon_count(first_coupon, maturity, months);
    let Some(regular_period_start) =
        add_months(first_coupon, -months, is_end_of_month(first_coupon))
    else {
        return Ok(None);
    };

    Ok(Some(OddFirstMeasures {
        is_long_first: issue < regular_period_start,
        accrued_fraction,
        issue_to_first_fraction,
        settlement_to_first_fraction,
        coupon_count,
    }))
}

fn quasi_fraction(
    start: Date,
    end: Date,
    anchor: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Option<f64>, ErrorKind> {
    if start >= end || end > anchor {
        return Ok((start == end).then_some(0.0));
    }
    let months = frequency.months();
    let end_of_month = is_end_of_month(anchor);
    let mut period_end = anchor;
    let mut periods = 0_usize;
    let mut period_start = loop {
        periods = periods.saturating_add(1);
        poll_loop_cancellation(periods, poll)?;
        let Some(prior) = add_months(period_end, -months, end_of_month) else {
            return Ok(None);
        };
        if prior <= start {
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
        let overlap_start = period_start.max(start);
        let overlap_end = period_end.min(end);
        if overlap_start < overlap_end {
            fraction +=
                super::day_count::days_between(overlap_start, overlap_end, basis) / normal_days;
        }
        if period_end >= end {
            return Ok(Some(fraction));
        }
        period_start = period_end;
        let Some(next_period_end) = add_months(period_end, months, end_of_month) else {
            return Ok(None);
        };
        period_end = next_period_end;
    }
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
            &mut || Ok(()),
        )
        .unwrap()
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
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!((measures.accrued_fraction - 76.0 / 180.0).abs() < 1e-12);
        assert!((measures.settlement_to_first_fraction - 30.0 / 180.0).abs() < 1e-12);
        assert!((measures.issue_to_first_fraction - 106.0 / 180.0).abs() < 1e-12);
        assert!(!measures.is_long_first);
        assert_eq!(measures.coupon_count, 11);
    }

    #[test]
    fn odd_first_long_period_is_split_into_quasi_coupon_fractions() {
        let measures = odd_first_measures(
            date(2023, 9, 1),
            date(2025, 3, 1),
            date(2025, 2, 1),
            date(2030, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(measures.issue_to_first_fraction, 3.0);
        assert!((measures.accrued_fraction - 17.0 / 6.0).abs() < 1e-12);
        assert!((measures.settlement_to_first_fraction - 1.0 / 6.0).abs() < 1e-12);
        assert!(measures.is_long_first);
    }
}
