use crate::DateSystem;

const EXCEL_1900_PRE_LEAP_OFFSET: i64 = 25_568;
const EXCEL_1900_POST_LEAP_OFFSET: i64 = 25_569;
const EXCEL_1904_OFFSET: i64 = 24_107;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Date {
    pub(super) year: i32,
    pub(super) month: u32,
    pub(super) day: u32,
}

pub(super) fn date_from_serial(serial: f64, system: DateSystem) -> Option<Date> {
    if system == DateSystem::Excel1900 && serial.is_finite() && (0.0..1.0).contains(&serial) {
        return Some(Date {
            year: 1900,
            month: 1,
            day: 0,
        });
    }
    if system == DateSystem::Excel1900 && serial.is_finite() && (60.0..61.0).contains(&serial) {
        return Some(Date {
            year: 1900,
            month: 2,
            day: 29,
        });
    }
    let days = unix_days_from_serial(serial, system)?;
    Some(civil_from_days(days))
}

pub(super) fn weekday_monday_zero(serial: f64, system: DateSystem) -> Option<i32> {
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let maximum = match system {
        DateSystem::Excel1900 => days_from_civil(9_999, 12, 31) + EXCEL_1900_POST_LEAP_OFFSET,
        DateSystem::Excel1904 => days_from_civil(9_999, 12, 31) + EXCEL_1904_OFFSET,
    };
    if serial > maximum as f64 {
        return None;
    }
    let serial = serial.floor() as i64;
    match system {
        // Excel intentionally preserves its historical 1900 weekday sequence:
        // serial 1 is Sunday and serial 60 is the fictitious 1900-02-29.
        DateSystem::Excel1900 => Some((serial + 5).rem_euclid(7) as i32),
        DateSystem::Excel1904 => {
            let unix_days = serial - EXCEL_1904_OFFSET;
            Some((unix_days + 3).rem_euclid(7) as i32)
        }
    }
}

pub(super) fn unix_days_from_serial(serial: f64, system: DateSystem) -> Option<i64> {
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let maximum = match system {
        DateSystem::Excel1900 => days_from_civil(9_999, 12, 31) + EXCEL_1900_POST_LEAP_OFFSET,
        DateSystem::Excel1904 => days_from_civil(9_999, 12, 31) + EXCEL_1904_OFFSET,
    };
    if serial > maximum as f64 {
        return None;
    }
    let serial = serial.floor() as i64;
    match system {
        DateSystem::Excel1900 if serial == 60 => None,
        DateSystem::Excel1900 if serial < 60 => Some(serial - EXCEL_1900_PRE_LEAP_OFFSET),
        DateSystem::Excel1900 => Some(serial - EXCEL_1900_POST_LEAP_OFFSET),
        DateSystem::Excel1904 => Some(serial - EXCEL_1904_OFFSET),
    }
}

pub(super) fn serial_from_date(date: Date, system: DateSystem) -> Option<f64> {
    if system == DateSystem::Excel1900
        && date
            == (Date {
                year: 1900,
                month: 2,
                day: 29,
            })
    {
        return Some(60.0);
    }
    serial_from_unix_days(days_from_civil(date.year, date.month, date.day), system)
}

pub(super) fn serial_from_unix_days(days: i64, system: DateSystem) -> Option<f64> {
    let offset = match system {
        DateSystem::Excel1900 if days < days_from_civil(1900, 3, 1) => EXCEL_1900_PRE_LEAP_OFFSET,
        DateSystem::Excel1900 => EXCEL_1900_POST_LEAP_OFFSET,
        DateSystem::Excel1904 => EXCEL_1904_OFFSET,
    };
    let serial = days.checked_add(offset)?;
    let maximum_offset = match system {
        DateSystem::Excel1900 => EXCEL_1900_POST_LEAP_OFFSET,
        DateSystem::Excel1904 => EXCEL_1904_OFFSET,
    };
    let maximum = days_from_civil(9_999, 12, 31).checked_add(maximum_offset)?;
    (serial >= 0 && serial <= maximum).then_some(serial as f64)
}

pub(super) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

pub(super) fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub(super) fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i32::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era - 719_468) as i64
}

fn civil_from_days(days: i64) -> Date {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Date {
        year: year as i32,
        month: month as u32,
        day: day as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_excel_serial_round_trips() {
        let date = Date {
            year: 2026,
            month: 7,
            day: 22,
        };
        assert_eq!(
            serial_from_date(date, DateSystem::Excel1900),
            Some(46_225.0)
        );
        assert_eq!(
            date_from_serial(46_225.0, DateSystem::Excel1900),
            Some(date)
        );
    }

    #[test]
    fn early_excel_1900_serials_preserve_the_leap_bug_boundary() {
        let january_first = Date {
            year: 1900,
            month: 1,
            day: 1,
        };
        let february_last = Date {
            year: 1900,
            month: 2,
            day: 28,
        };
        let march_first = Date {
            year: 1900,
            month: 3,
            day: 1,
        };
        assert_eq!(
            date_from_serial(0.0, DateSystem::Excel1900),
            Some(Date {
                year: 1900,
                month: 1,
                day: 0,
            })
        );
        assert_eq!(
            date_from_serial(1.0, DateSystem::Excel1900),
            Some(january_first)
        );
        assert_eq!(
            date_from_serial(59.0, DateSystem::Excel1900),
            Some(february_last)
        );
        assert_eq!(
            date_from_serial(60.0, DateSystem::Excel1900),
            Some(Date {
                year: 1900,
                month: 2,
                day: 29,
            })
        );
        assert_eq!(
            serial_from_date(
                Date {
                    year: 1900,
                    month: 2,
                    day: 29,
                },
                DateSystem::Excel1900,
            ),
            Some(60.0)
        );
        assert_eq!(
            date_from_serial(61.0, DateSystem::Excel1900),
            Some(march_first)
        );
        assert_eq!(
            serial_from_date(january_first, DateSystem::Excel1900),
            Some(1.0)
        );
        assert_eq!(
            serial_from_date(february_last, DateSystem::Excel1900),
            Some(59.0)
        );
        assert_eq!(
            serial_from_date(march_first, DateSystem::Excel1900),
            Some(61.0)
        );
        assert_eq!(weekday_monday_zero(0.0, DateSystem::Excel1900), Some(5));
        assert_eq!(weekday_monday_zero(60.0, DateSystem::Excel1900), Some(2));
        assert_eq!(weekday_monday_zero(61.0, DateSystem::Excel1900), Some(3));
    }
}
