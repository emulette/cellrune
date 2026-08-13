/// Regular coupon schedule projection.
///
/// A regular schedule is the maturity-anchored walk backward by `12 / frequency` months. The
/// maturity end-of-month policy decides whether every coupon date snaps to the last day of its
/// month; otherwise the maturity day-of-month is preserved and clamped to the target month length.
use crate::DateSystem;

use super::super::calendar::{Date, days_from_civil, days_in_month, serial_from_date};
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
        if self.previous_coupon < date_system_epoch(system) {
            return match system {
                DateSystem::Excel1900 => Some(0.0),
                DateSystem::Excel1904 => Some(coupon_actual_days(
                    date_system_epoch(system),
                    self.previous_coupon,
                )),
            };
        }
        serial_from_date(self.previous_coupon, system)
    }

    pub(super) fn previous_coupon_precedes_epoch(self, system: DateSystem) -> bool {
        self.previous_coupon < date_system_epoch(system)
    }

    pub(super) fn next_coupon_serial(self, system: DateSystem) -> Option<f64> {
        serial_from_date(self.next_coupon, system)
    }
}

pub(super) fn is_end_of_month(date: Date) -> bool {
    // Excel's 1900 date system exposes the fictitious 1900-02-29. Treat it as
    // month-end even though the proleptic Gregorian calendar has only 28 days.
    date.day >= days_in_month(date.year, date.month)
}

pub(super) fn add_months(date: Date, months: i64, end_of_month: bool) -> Option<Date> {
    if months == 0 {
        return Some(date);
    }
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
    date_system: DateSystem,
) -> Option<RegularSchedule> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(maturity);

    let maturity_index = month_index(maturity);
    let settlement_index = month_index(settlement);
    let month_distance = maturity_index.checked_sub(settlement_index)?;
    let mut steps = month_distance.div_euclid(months);
    let candidate = add_months(maturity, -steps.checked_mul(months)?, end_of_month)?;
    if candidate > settlement {
        steps = steps.checked_add(1)?;
    }
    let previous = add_months(maturity, -steps.checked_mul(months)?, end_of_month)?;
    let next = if steps == 1 {
        maturity
    } else {
        add_months(previous, months, end_of_month)?
    };

    // Coupon dates can project before the workbook epoch. Excel reports the
    // previous coupon as serial zero and measures accrued days from that epoch.
    let accrued_start = match date_system {
        DateSystem::Excel1900 => previous.max(date_system_epoch(date_system)),
        DateSystem::Excel1904 => previous,
    };
    let accrued_days = super::day_count::days_between(accrued_start, settlement, basis);
    let days_to_next = super::day_count::days_between(settlement, next, basis);
    let period_days = period_days(previous, next, basis, frequency);
    let coupon_count = steps;

    Some(RegularSchedule {
        previous_coupon: previous,
        next_coupon: next,
        accrued_days,
        days_to_next,
        period_days,
        coupon_count,
    })
}

pub(super) fn estimated_periods(
    start: Date,
    end: Date,
    frequency: CouponFrequency,
) -> Option<usize> {
    let month_distance = month_index(end).checked_sub(month_index(start))?;
    usize::try_from(month_distance.div_euclid(frequency.months()) + 2).ok()
}

fn month_index(date: Date) -> i64 {
    i64::from(date.year) * 12 + i64::from(date.month) - 1
}

fn period_days(
    previous: Date,
    next: Date,
    basis: DayCountBasis,
    frequency: CouponFrequency,
) -> f64 {
    match basis {
        DayCountBasis::ActualActual => coupon_actual_days(previous, next),
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
            Some(coupon_actual_days(start, end))
        }
        DayCountBasis::Us30360 | DayCountBasis::Actual360 | DayCountBasis::European30360 => {
            Some(360.0 / frequency.as_f64())
        }
        DayCountBasis::Actual365 => Some(365.0 / frequency.as_f64()),
    }
}

fn date_system_epoch(system: DateSystem) -> Date {
    match system {
        DateSystem::Excel1900 => Date {
            year: 1900,
            month: 1,
            day: 0,
        },
        DateSystem::Excel1904 => Date {
            year: 1904,
            month: 1,
            day: 1,
        },
    }
}

fn coupon_actual_days(start: Date, end: Date) -> f64 {
    fn gregorian_coupon_date(date: Date) -> Date {
        if date
            == (Date {
                year: 1900,
                month: 2,
                day: 29,
            })
        {
            Date {
                year: 1900,
                month: 2,
                day: 28,
            }
        } else {
            date
        }
    }

    let start = gregorian_coupon_date(start);
    let end = gregorian_coupon_date(end);
    (days_from_civil(end.year, end.month, end.day)
        - days_from_civil(start.year, start.month, start.day)) as f64
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
            DateSystem::Excel1900,
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
            DateSystem::Excel1900,
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
            DateSystem::Excel1900,
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
            DateSystem::Excel1900,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(2022, 8, 31));
        assert_eq!(schedule.next_coupon, date(2023, 2, 28));
    }

    #[test]
    fn excel_1900_coupon_period_excludes_the_fictitious_leap_day() {
        let schedule = regular_schedule(
            date(1900, 2, 28),
            date(1900, 2, 29),
            CouponFrequency::Semiannual,
            DayCountBasis::ActualActual,
            DateSystem::Excel1900,
        )
        .unwrap();
        assert_eq!(schedule.previous_coupon, date(1899, 8, 31));
        assert_eq!(
            schedule.previous_coupon_serial(DateSystem::Excel1900),
            Some(0.0)
        );
        assert_eq!(schedule.accrued_days, 59.0);
        assert_eq!(schedule.days_to_next, 1.0);
        assert_eq!(schedule.period_days, 181.0);
    }
}
