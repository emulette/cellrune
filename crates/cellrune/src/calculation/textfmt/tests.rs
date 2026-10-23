use super::super::value::ErrorKind;
use super::{FormatContext, format_number};
use crate::DateSystem;

const CONTEXT: FormatContext = FormatContext {
    date_system: DateSystem::Excel1900,
};

fn text(value: f64, format: &str) -> Result<String, ErrorKind> {
    format_number(value, format, CONTEXT)
}

fn assert_text(value: f64, format: &str, expected: &str) {
    assert_eq!(
        text(value, format).as_deref(),
        Ok(expected),
        "{value} with {format:?}"
    );
}

fn assert_unsupported(value: f64, format: &str) {
    assert_eq!(
        text(value, format),
        Err(ErrorKind::Unsupported),
        "{value} with {format:?}"
    );
}

/// Serial date-time of a clock time on day zero.
fn clock(hours: f64, minutes: f64, seconds: f64) -> f64 {
    (hours * 3_600.0 + minutes * 60.0 + seconds) / 86_400.0
}

#[test]
fn single_section_digit_codes_keep_their_earlier_results() {
    assert_text(1234.567, "$#,##0.00", "$1,234.57");
    assert_text(-5.0, "$0", "-$5");
    assert_text(12.0, "00000", "00012");
    assert_text(123.0, "\"000\"#", "000123");
    assert_text(0.631, "0.#", "0.6");
    assert_text(12.0, "#.0#", "12.0");
    assert_text(0.5, "#.00", ".50");
    assert_text(0.0, "#", "0");
    assert_text(1.0, "0.#", "1");
    assert_text(-0.4, "0", "-0");
    assert_text(-0.001, "#.##", "");
    assert_text(0.285, "0.0%", "28.5%");
    assert_text(2.5, "0/2", "3/2");
    assert_text(5.0, "0 a/p", "5 a/p");
    assert_eq!(text(1e16, "0"), Err(ErrorKind::Num));
}

#[test]
fn sections_select_positive_negative_and_zero_layouts() {
    assert_text(5.0, "0.00;(0.00)", "5.00");
    assert_text(-5.0, "0.00;(0.00)", "(5.00)");
    assert_text(0.0, "0.00;(0.00)", "0.00");
    assert_text(0.0, "0;-0;\"zero\"", "zero");
    assert_text(-0.0, "0;-0;\"zero\"", "zero");
    assert_text(-3.0, "0;;0", "");
    assert_text(7.0, ";;;", "");
    assert_text(7.0, "", "");
    assert_text(7.0, "0;-0;0;@", "7");
}

#[test]
fn unquoted_letters_in_a_zero_section_read_e_as_the_year() {
    // Excel Online stores "z1900ro" for TEXT(0*-1,"0.00;-0.00;zero").
    assert_text(-0.0, "0.00;-0.00;zero", "z1900ro");
}

#[test]
fn conditions_choose_sections_and_colors_are_not_shown() {
    let phone = "[<=9999999]###-####;(###) ###-####";
    assert_text(1_234_567_898.0, phone, "(123) 456-7898");
    assert_text(5_551_234.0, phone, "555-1234");
    assert_text(150.0, "[Red][<=100]0;[Blue][>100]0.0", "150.0");
    assert_text(50.0, "[Red][<=100]0;[Blue][>100]0.0", "50");
    assert_text(1.5, "[Red]0.00", "1.50");
    assert_unsupported(-5.0, "[<=100]0;0");
    assert_unsupported(5.0, "[Color10]0");
}

#[test]
fn literals_between_digit_placeholders_take_digits_from_the_right() {
    assert_text(123_456_789.0, "000-00-0000", "123-45-6789");
    assert_text(123_456.0, "##0° 00' 00''", "12° 34' 56''");
    assert_text(7.0, "0_)", "7 ");
}

#[test]
fn question_marks_pad_with_spaces() {
    assert_text(44.398, "???.???", " 44.398");
    assert_text(102.65, "???.???", "102.65 ");
    assert_text(2.8, "???.???", "  2.8  ");
    assert_text(8.9, "0.0?", "8.9 ");
    assert_text(88.99, "0.0?", "88.99");
}

#[test]
fn trailing_commas_scale_by_thousands() {
    assert_text(12_000.0, "#,###", "12,000");
    assert_text(12_000.0, "#,", "12");
    assert_text(12_200_000.0, "0.0,,\"M\"", "12.2M");
    assert_text(12_200_000.0, "#,###.0,", "12,200.0");
}

#[test]
fn scientific_layouts_show_the_exponent() {
    assert_text(12_200_000.0, "0.00E+00", "1.22E+07");
    assert_text(12_200_000.0, "#0.0E+0", "12.2E+6");
    assert_text(12_200_000.0, "##0.0E+0", "12.2E+6");
    assert_text(0.000_12, "0.00E+00", "1.20E-04");
    assert_text(0.000_12, "0.00E-00", "1.20E-04");
    assert_text(1_200.0, "0.00e-00", "1.20e03");
    assert_text(9.999, "0.00E+00", "1.00E+01");
    assert_text(0.0, "0.00E+00", "0.00E+00");
    assert_text(-12_200_000.0, "0.00E+00", "-1.22E+07");
    assert_unsupported(12.0, "00.0E+0");
}

#[test]
fn fractions_use_the_closest_denominator_their_placeholders_allow() {
    assert_text(4.34, "# ?/?", "4 1/3");
    assert_text(0.34, "# ?/?", " 1/3");
    assert_text(5.25, "# ???/???", "5   1/4  ");
    assert_text(5.3, "# ???/???", "5   3/10 ");
    assert_text(5.25, "?/?", "21/4");
    assert_text(-4.34, "# ?/?", "-4 1/3");
    assert_unsupported(5.0, "# ?/?");
    assert_unsupported(0.3, "?/8");
    assert_unsupported(2.375, "# 0/8");
}

#[test]
fn dates_follow_both_date_systems() {
    // DATE(2026,7,27)
    assert_text(46_230.0, "yyyy-mm-dd", "2026-07-27");
    assert_text(46_230.0, "MM/DD/YY", "07/27/26");
    assert_text(46_230.0, "d-mmm-yy", "27-Jul-26");
    assert_text(46_230.0, "dddd, mmmm d, yyyy", "Monday, July 27, 2026");
    assert_text(46_230.0, "ddd mmmmm", "Mon J");
    assert_text(46_230.0, " ", " ");
    assert_text(60.0, "yyyy-mm-dd", "1900-02-29");
    assert_text(0.0, "yyyy-mm-dd ddd", "1900-01-00 Sat");
    assert_text(1.0, "dddd", "Sunday");
    let excel_1904 = FormatContext {
        date_system: DateSystem::Excel1904,
    };
    assert_eq!(
        format_number(0.0, "yyyy-mm-dd", excel_1904).as_deref(),
        Ok("1904-01-01")
    );
    assert_eq!(
        format_number(44_768.0, "yyyy-mm-dd", excel_1904).as_deref(),
        Ok("2026-07-27")
    );
    assert_unsupported(-1.0, "yyyy");
    assert_unsupported(2_958_466.0, "yyyy");
    assert_unsupported(46_230.0, "yyy");
}

#[test]
fn times_use_twelve_hours_with_a_marker_and_round_to_the_shown_second() {
    let afternoon = clock(16.0, 36.0, 3.75);
    assert_text(clock(4.0, 0.0, 0.0), "h AM/PM", "4 AM");
    assert_text(afternoon, "h:mm AM/PM", "4:36 PM");
    assert_text(afternoon, "h:mm:ss A/P", "4:36:04 P");
    assert_text(afternoon, "hh:mm:ss.00", "16:36:03.75");
    assert_text(afternoon, "h:mm:ss a/p", "4:36:04 p");
    assert_text(clock(0.0, 30.0, 0.0), "h:mm AM/PM", "12:30 AM");
    assert_text(clock(12.0, 0.0, 0.0), "h AM/PM", "12 PM");
    assert_text(
        clock(23.0, 59.0, 59.6),
        "yyyy-mm-dd hh:mm:ss",
        "1900-01-01 00:00:00",
    );
    assert_text(46_230.5, "m/d/yyyy h:mm", "7/27/2026 12:00");
    assert_unsupported(afternoon, "h:mm am/pm");
}

#[test]
fn elapsed_codes_count_past_a_day() {
    assert_text(clock(1.0, 2.0, 0.0), "[h]:mm", "1:02");
    assert_text(clock(0.0, 62.0, 16.0), "[mm]:ss", "62:16");
    assert_text(clock(0.0, 0.0, 3_735.8), "[ss].00", "3735.80");
    assert_text(1.5, "[h]:mm:ss", "36:00:00");
}

#[test]
fn general_shows_the_number_as_text() {
    assert_text(1234.5, "General", "1234.5");
    assert_text(-5.0, "General", "-5");
    assert_text(-5.0, "General;(General)", "(5)");
    assert_text(3.0, "General\" units\"", "3 units");
}

#[test]
fn unmeasured_constructs_stay_unsupported() {
    assert_unsupported(5.0, "*-0");
    assert_unsupported(5.0, "@");
    assert_unsupported(-5.0, "0;@;0");
    assert_unsupported(-5.0, "abc");
    assert_unsupported(46_230.0, "[$-F800]dddd");
    assert_unsupported(46_230.0, "[$-412]mmmm");
    assert_unsupported(46_230.0, "aaa");
    assert_unsupported(46_230.0, "ggge");
    assert_unsupported(5.0, "0;0;0;0;0");
    assert_text(46_230.0, "[$-409]mmmm", "July");
    assert_text(1234.5, "[$₩-412]#,##0", "₩1,235");
}
