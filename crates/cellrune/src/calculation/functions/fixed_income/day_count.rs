use super::super::calendar::{Date, days_from_civil, is_leap_year};
/// Basis-specific day measures for fixed-income securities.
///
/// Coupon `A`/`DSC` and discount `DSM` use the basis day count between two validated dates. The
/// annual denominator `B` is fixed for every basis except Actual/Actual, where it is the average
/// length of the years actually spanned by the interval. `Actual/Actual` coupon period length is
/// deliberately *not* routed through this denominator: a coupon period uses its own actual day
/// count, not a yearly average.
use super::model::DayCountBasis;

pub(super) fn days_between(start: Date, end: Date, basis: DayCountBasis) -> f64 {
    match basis {
        DayCountBasis::Us30360 => super::super::date::days_360_us(start, end),
        DayCountBasis::ActualActual | DayCountBasis::Actual360 | DayCountBasis::Actual365 => {
            actual_days(start, end)
        }
        DayCountBasis::European30360 => super::super::date::days_360_european(start, end),
    }
}

pub(super) fn annual_denominator(basis: DayCountBasis, start: Date, end: Date) -> f64 {
    match basis {
        DayCountBasis::Us30360 | DayCountBasis::Actual360 | DayCountBasis::European30360 => 360.0,
        DayCountBasis::Actual365 => 365.0,
        DayCountBasis::ActualActual => actual_actual_denominator(start, end),
    }
}

pub(super) fn actual_days(start: Date, end: Date) -> f64 {
    (days_from_civil(end.year, end.month, end.day)
        - days_from_civil(start.year, start.month, start.day)) as f64
}

fn actual_actual_denominator(start: Date, end: Date) -> f64 {
    if start.year == end.year {
        return if is_leap_year(start.year) {
            366.0
        } else {
            365.0
        };
    }
    let no_more_than_one_year =
        end.year == start.year + 1 && (end.month, end.day) <= (start.month, start.day);
    if no_more_than_one_year {
        let includes_leap_day = (is_leap_year(start.year) && (start.month, start.day) <= (2, 29))
            || (is_leap_year(end.year) && (end.month, end.day) >= (2, 29));
        return if includes_leap_day { 366.0 } else { 365.0 };
    }
    let year_count = i64::from(end.year - start.year + 1);
    let days = (start.year..=end.year)
        .map(|year| if is_leap_year(year) { 366_i64 } else { 365_i64 })
        .sum::<i64>();
    days as f64 / year_count as f64
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::*;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn us_30_360_matches_coupon_fixture() {
        let prev = date(2025, 1, 1);
        let settlement = date(2025, 3, 15);
        let next = date(2025, 7, 1);
        assert_eq!(days_between(prev, settlement, DayCountBasis::Us30360), 74.0);
        assert_eq!(
            days_between(settlement, next, DayCountBasis::Us30360),
            106.0
        );
        assert_eq!(days_between(prev, next, DayCountBasis::Us30360), 180.0);
    }

    #[test]
    fn actual_and_european_basis_measures() {
        let start = date(2025, 1, 31);
        let end = date(2025, 2, 28);
        assert_eq!(days_between(start, end, DayCountBasis::ActualActual), 28.0);
        assert_eq!(days_between(start, end, DayCountBasis::European30360), 28.0);
    }

    #[test]
    fn actual_actual_denominator_handles_leap_boundaries() {
        assert_eq!(
            annual_denominator(
                DayCountBasis::ActualActual,
                date(2024, 1, 1),
                date(2024, 12, 31)
            ),
            366.0
        );
        assert_eq!(
            annual_denominator(
                DayCountBasis::ActualActual,
                date(2023, 1, 1),
                date(2023, 12, 31)
            ),
            365.0
        );
    }
}
