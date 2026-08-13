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
    (actual_day_ordinal(end) - actual_day_ordinal(start)) as f64
}

fn actual_day_ordinal(date: Date) -> i64 {
    let fictitious_leap_day = Date {
        year: 1900,
        month: 2,
        day: 29,
    };
    let march_first = Date {
        year: 1900,
        month: 3,
        day: 1,
    };
    if date == fictitious_leap_day {
        return days_from_civil(1900, 2, 28) + 1;
    }
    let ordinal = days_from_civil(date.year, date.month, date.day);
    if date >= march_first {
        ordinal + 1
    } else {
        ordinal
    }
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

    #[test]
    fn actual_days_preserve_the_excel_1900_fictitious_day() {
        assert_eq!(actual_days(date(1900, 2, 28), date(1900, 2, 29)), 1.0);
        assert_eq!(actual_days(date(1900, 2, 29), date(1900, 3, 1)), 1.0);
        assert_eq!(actual_days(date(1900, 2, 28), date(1900, 3, 1)), 2.0);
        assert_eq!(actual_days(date(1900, 1, 0), date(1900, 1, 1)), 1.0);
    }

    #[test]
    fn multi_year_actual_actual_denominators_match_independent_rationals() {
        let first_start = date(2019, 7, 1);
        let first_end = date(2021, 7, 1);
        let first_days = actual_days(first_start, first_end);
        let first_denominator =
            annual_denominator(DayCountBasis::ActualActual, first_start, first_end);
        assert_eq!(first_days, 731.0);
        assert_eq!(first_denominator, 1_096.0 / 3.0);
        let first_disc = 0.05 * first_denominator / first_days;
        assert!((first_disc - 274.0 / 10_965.0).abs() < 1e-15);

        let second_start = date(2019, 12, 31);
        let second_end = date(2022, 1, 1);
        let second_days = actual_days(second_start, second_end);
        let second_denominator =
            annual_denominator(DayCountBasis::ActualActual, second_start, second_end);
        assert_eq!(second_days, 732.0);
        assert_eq!(second_denominator, 1_461.0 / 4.0);
        let second_disc = 0.03 * second_denominator / second_days;
        assert!((second_disc - 1_461.0 / 97_600.0).abs() < 1e-15);
    }
}
