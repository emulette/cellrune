/// Regular coupon schedule projection.
///
/// A regular schedule is the maturity-anchored walk backward by `12 / frequency` months. The
/// maturity end-of-month policy decides whether every coupon date snaps to the last day of its
/// month; otherwise the maturity day-of-month is preserved and clamped to the target month length.
use crate::DateSystem;

use super::super::calendar::{Date, days_in_month, serial_from_date};
use super::model::{CouponFrequency, DayCountBasis};

#[derive(Debug, Clone, Copy)]
pub(super) struct RegularSchedule {
    pub(super) previous_coupon: Date,
    pub(super) next_coupon: Date,
    pub(super) accrued_days: f64,
    pub(super) days_to_next: f64,
    pub(super) period_days: f64,
    pub(super) coupon_count: i64,
}

impl RegularSchedule {
    pub(super) fn previous_coupon_serial(self, system: DateSystem) -> Option<f64> {
        serial_from_date(self.previous_coupon, system)
    }

    pub(super) fn next_coupon_serial(self, system: DateSystem) -> Option<f64> {
        serial_from_date(self.next_coupon, system)
    }
}

pub(super) fn is_end_of_month(date: Date) -> bool {
    date.day == days_in_month(date.year, date.month)
}

pub(super) fn add_months(date: Date, months: i64, end_of_month: bool) -> Option<Date> {
    let total = i64::from(date.year)
        .checked_mul(12)?
        .checked_add(i64::from(date.month).checked_sub(1)?)?
        .checked_add(months)?;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) as u32 + 1;
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let day = if end_of_month {
        days_in_month(year as i32, month)
    } else {
        date.day.min(days_in_month(year as i32, month))
    };
    Some(Date {
        year: year as i32,
        month,
        day,
    })
}

pub(super) fn regular_schedule(
    settlement: Date,
    maturity: Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
) -> Option<RegularSchedule> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(maturity);

    let mut next = maturity;
    let previous = loop {
        let prior = add_months(next, -months, end_of_month)?;
        if prior <= settlement {
            break prior;
        }
        next = prior;
    };

    let accrued_days = super::day_count::days_between(previous, settlement, basis);
    let days_to_next = super::day_count::days_between(settlement, next, basis);
    let period_days = period_days(previous, next, basis, frequency);
    let coupon_count = coupon_count(next, maturity, months);

    Some(RegularSchedule {
        previous_coupon: previous,
        next_coupon: next,
        accrued_days,
        days_to_next,
        period_days,
        coupon_count,
    })
}

fn period_days(
    previous: Date,
    next: Date,
    basis: DayCountBasis,
    frequency: CouponFrequency,
) -> f64 {
    match basis {
        DayCountBasis::ActualActual => super::day_count::days_between(previous, next, basis),
        DayCountBasis::Us30360 | DayCountBasis::Actual360 | DayCountBasis::European30360 => {
            360.0 / frequency.as_f64()
        }
        DayCountBasis::Actual365 => 365.0 / frequency.as_f64(),
    }
}

pub(super) fn normal_period_days(
    start: Date,
    months: i64,
    basis: DayCountBasis,
    frequency: CouponFrequency,
    end_of_month: bool,
) -> Option<f64> {
    match basis {
        DayCountBasis::ActualActual => {
            let end = add_months(start, months, end_of_month)?;
            Some(super::day_count::days_between(start, end, basis))
        }
        DayCountBasis::Us30360 | DayCountBasis::Actual360 | DayCountBasis::European30360 => {
            Some(360.0 / frequency.as_f64())
        }
        DayCountBasis::Actual365 => Some(365.0 / frequency.as_f64()),
    }
}

fn coupon_count(next: Date, maturity: Date, months: i64) -> i64 {
    let next_index = i64::from(next.year) * 12 + i64::from(next.month) - 1;
    let maturity_index = i64::from(maturity.year) * 12 + i64::from(maturity.month) - 1;
    (maturity_index - next_index) / months + 1
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
    fn one_coupon_fixture() {
        let schedule = regular_schedule(
            date(2025, 3, 15),
            date(2025, 7, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(2025, 1, 1));
        assert_eq!(schedule.next_coupon, date(2025, 7, 1));
        assert_eq!(schedule.accrued_days, 74.0);
        assert_eq!(schedule.days_to_next, 106.0);
        assert_eq!(schedule.period_days, 180.0);
        assert_eq!(schedule.coupon_count, 1);
    }

    #[test]
    fn settlement_on_coupon_date_starts_a_fresh_period() {
        let schedule = regular_schedule(
            date(2025, 7, 1),
            date(2027, 1, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(2025, 7, 1));
        assert_eq!(schedule.next_coupon, date(2026, 1, 1));
        assert_eq!(schedule.accrued_days, 0.0);
        assert_eq!(schedule.days_to_next, 180.0);
        assert_eq!(schedule.coupon_count, 3);
    }

    #[test]
    fn end_of_month_maturity_pins_coupons_to_month_end() {
        let schedule = regular_schedule(
            date(2025, 1, 20),
            date(2025, 8, 31),
            CouponFrequency::Semiannual,
            DayCountBasis::ActualActual,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(2024, 8, 31));
        assert_eq!(schedule.next_coupon, date(2025, 2, 28));
        assert_eq!(schedule.coupon_count, 2);
    }

    #[test]
    fn february_eom_coupon_walks_across_leap_and_common_years() {
        let schedule = regular_schedule(
            date(2023, 1, 1),
            date(2024, 2, 29),
            CouponFrequency::Semiannual,
            DayCountBasis::ActualActual,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(2022, 8, 31));
        assert_eq!(schedule.next_coupon, date(2023, 2, 28));
    }
}
