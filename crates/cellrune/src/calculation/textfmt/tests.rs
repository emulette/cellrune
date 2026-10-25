use super::super::value::ErrorKind;
use super::{FormatContext, format_number, format_text};
use crate::{DateSystem, Locale};

const CONTEXT: FormatContext = FormatContext {
    date_system: DateSystem::Excel1900,
    locale: Locale::EnUs,
    formula: true,
    decimal: true,
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
fn digit_codes_follow_mac_excel_16_114() {
    assert_text(1234.567, "$#,##0.00", "$1,234.57");
    assert_text(-5.0, "$0", "-$5");
    assert_text(12.0, "00000", "00012");
    assert_text(123.0, "\"000\"#", "000123");
    assert_text(0.631, "0.#", "0.6");
    assert_text(12.0, "#.0#", "12.0");
    assert_text(0.5, "#.00", ".50");
    assert_text(0.5, "#", "1");
    assert_text(-2.5, "0", "-3");
    assert_text(-1234.5, "#,##0", "-1,235");
    // A zero integer part without a `0` placeholder shows nothing, and the decimal point always
    // shows.
    assert_text(0.0, "#", "");
    assert_text(0.0, "#.##", ".");
    assert_text(1.0, "0.#", "1.");
    assert_text(1.0, "#.", "1.");
    assert_text(0.5, "0.", "1.");
    assert_text(1.0, "0.#;-0.#", "1.");
    assert_text(-1.0, "0.#;-0.#", "-1.");
    assert_text(0.0, "#;-#", "");
    assert_text(1.0, "#,", "");
    assert_text(-0.4, "#;(#)", "()");
    assert_text(1.0, "0.#?", "1. ");
    assert_text(0.5, "#.0?", ".5 ");
    // A negative number shows its sign only when a nonzero digit shows.
    assert_text(-0.4, "#.#", "-.4");
    assert_text(-0.4, "0", "0");
    assert_text(-0.4, "#", "");
    assert_text(-0.04, "#.#", ".");
    assert_text(-0.001, "0.00", "0.00");
    assert_text(-0.001, "0.00 \"x\"", "0.00 x");
    assert_text(-0.001, "0.0?", "0.0 ");
    assert_text(-0.001, "#.#?", ". ");
    assert_text(-0.4, "0%", "-40%");
    assert_text(-0.004, "0%", "0%");
    assert_text(-1e-20, "0", "0");
    assert_text(-0.000_4, "0.00E+00", "-4.00E-04");
    // Scientific layouts that would need the rules of an unshown decimal point are unmeasured.
    assert_unsupported(1.0, "0.#E+0");
    assert_unsupported(0.0, "#.0E+0");
    assert_text(0.0, "0.0E+0", "0.0E+0");
}

#[test]
fn digits_round_the_fifteen_digit_decimal_half_away_from_zero() {
    assert_text(1.005, "0.00", "1.01");
    assert_text(-1.005, "0.00", "-1.01");
    assert_text(2.675, "0.00", "2.68");
    assert_text(1.15, "0.0", "1.2");
    assert_text(0.285, "0%", "29%");
    assert_text(0.285, "0.0%", "28.5%");
    assert_text(0.1 + 0.2, "0.00000000000000000", "0.30000000000000000");
    assert_text(1_234_567.0, "#,##0,", "1,235");
    assert_text(1_234_500.0, "0.0,,", "1.2");
    assert_text(1.005, "0.00E+00", "1.01E+00");
    assert_text(1005.0, "0.00E+0", "1.01E+3");
    assert_text(1.005e20, "0.00E+00", "1.01E+20");
    assert_text(12_345.0, "##0.0E+0", "12.3E+3");
    assert_text(999.999_999_999_999_9, "0.0E+0", "1.0E+3");
    // A tie past the fifteenth digit rounds toward zero (Mac Excel 16.114).
    assert_text(1_234_567_890_123_455.0, "0", "1234567890123450");
    assert_text(12_345_678_901_234.75, "0.0", "12345678901234.7");
    assert_text(1_234_567_890_123_456.0, "0", "1234567890123460");
    // Subnormal magnitudes keep their exponent.
    assert_text(f64::from_bits(1), "0.00E+00", "4.94E-324");
    assert_text(5e-310, "0.00E+00", "5.00E-310");
}

#[test]
fn large_numbers_show_fifteen_significant_digits_then_zeros() {
    assert_text(1e20, "0", "100000000000000000000");
    assert_text(1e20, "#,##0", "100,000,000,000,000,000,000");
    assert_text(2_f64.powi(60), "0", "1152921504606850000");
    assert_text(1e15 + 0.3, "0.0", "1000000000000000.0");
    assert_text(
        1.234e25,
        "#,##0.00",
        "12,340,000,000,000,000,000,000,000.00",
    );
    assert_text(
        0.000_123_456_789_012_345,
        "0.000000000000000000000000",
        "0.000123456789012345000000",
    );
    assert_text(1e16, "0.00;(0.00)", "10000000000000000.00");
    assert_text(-1e16, "0.00;(0.00)", "(10000000000000000.00)");
    assert_text(1e14, "#,##0.00_)", "100,000,000,000,000.00 ");
    assert_text(1.0, "0.0000000000000000E+0", "1.0000000000000000E+0");
    // A fraction past 2^53 units is beyond the engine.
    assert_unsupported(1e16, "0 ?/?");
}

#[test]
fn results_past_255_characters_or_127_decimals_are_rejected() {
    assert_eq!(text(1e254, "0").map(|text| text.len()), Ok(255));
    assert_eq!(text(1e252, "0.0").map(|text| text.len()), Ok(255));
    let decimals = |count: usize| format!("0.{}", "0".repeat(count));
    assert_eq!(text(1.0, &decimals(127)).map(|text| text.len()), Ok(129));
    let scientific = |count: usize| format!("{}E+0", decimals(count));
    assert_eq!(text(1.0, &scientific(127)).map(|text| text.len()), Ok(132));
    for (value, format) in [
        (1.0, scientific(128)),
        (1.0, format!("0;{}", decimals(128))),
        (1.0, "0 nn".to_owned()),
        (1.0, "nn".to_owned()),
        (-1e254, "0".to_owned()),
        (1e255, "0".to_owned()),
        (1.5e300, "0".to_owned()),
        (1e240, "#,##0".to_owned()),
        (1e253, "0.0".to_owned()),
        (1.0, decimals(128)),
        (1.0, format!("0.{}", "#".repeat(128))),
    ] {
        assert_eq!(text(value, &format), Err(ErrorKind::Value), "{value:e}");
    }
    assert_eq!(text(1e300, "0.0E+0").as_deref(), Ok("1.0E+300"));
    // Text counts UTF-16 code units, as Excel does.
    let emoji = |count: usize| "😀".repeat(count);
    assert_eq!(
        format_text(&emoji(127), "@", CONTEXT).map(|text| text.encode_utf16().count()),
        Ok(254)
    );
    assert_eq!(
        format_text(&emoji(128), "@", CONTEXT),
        Err(ErrorKind::Value)
    );
    assert_eq!(
        format_text("abc", &decimals(128), CONTEXT),
        Err(ErrorKind::Value)
    );
}

#[test]
fn binary_rounding_rounds_the_scaled_double() {
    let binary = FormatContext {
        decimal: false,
        ..CONTEXT
    };
    for (value, format, expected) in [
        (1.005, "0.00", "1.00"),
        (0.285, "0%", "28%"),
        (2_f64.powi(60), "0", "1152921504606846976"),
        (2.5, "0", "3"),
        (-0.001, "0.00", "0.00"),
        // Digits past those a double holds show its exact binary expansion.
        (
            1.0,
            "0.00000000000000000000000",
            "1.00000000000000000000000",
        ),
        (1_234_567.89, "0.0000000000", "1234567.8899999999"),
        (0.3, "0.0000000000000000000", "0.2999999999999999889"),
        (f64::from_bits(1), "0.00E+00", "4.94E-324"),
    ] {
        assert_eq!(
            format_number(value, format, binary).as_deref(),
            Ok(expected),
            "{value} with {format:?}"
        );
    }
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
fn unquoted_letters_follow_excel_16_114() {
    // Excel 16.114 for Mac, as Excel Online for curated:arithmetic:c019, shows the letters that are
    // not codes as written and `e` as the four-digit year (excel-measure RESULTS.md, sections N
    // and N2); Mac Excel 2021 stored "z3333o" for the first.
    assert_text(-0.0, "0.00;-0.00;zero", "z1900ro");
    assert_text(45_000.0, "zero", "z2023ro");
    assert_text(0.0, "x", "x");
    assert_text(-5.0, "0 \"x\";0 x", "5 x");
    assert_text(46_230.0, "yyyy r", "2026 r");
    assert_text(3.0, "General x", "3 x");
    assert_text(45_000.25, "yyyyy", "2023");
    assert_text(5.0, "0 x", "5 x");
    assert_text(5.0, "0 \"x\";0", "5 x");
    // Mac Excel 16.114 shows runs of letters that are not codes and letters of other scripts
    // beside digits as written.
    assert_text(5.0, "0 zz", "5 zz");
    assert_text(5.0, "0 aa", "5 aa");
    assert_text(5.0, "#,##0원", "5원");
    assert_text(5.0, "0.0 ㄱㄴ", "5.0 ㄱㄴ");
    assert_text(5.0, "0 α", "5 α");
    assert_text(5.0, "0;\"x\" d", "5");
    assert_text(5.0, "\"x\" d", "x 5");
    // Excel rejects n, N, B, and E as codes, and digits beside date, time, era, and weekday codes,
    // in whichever section they stand.
    for format in [
        "n", "N", "B", "E", "0n;0", "yyyyE", "0n", "0;0n", "0 d", "d 0", "# d", "0 D", "0 e",
        "0 g", "0 b", "0 bb", "0 aaa", "0;0 d", "ss.##", "[h] 0", "0 [h]",
    ] {
        assert_eq!(text(5.0, format), Err(ErrorKind::Value), "{format}");
    }
    assert_text(0.5, "ss.000", "00.000");
    // Accented Latin letters beside digits differ by letter (`0 ä` shows, `0 é` rejects).
    assert_unsupported(5.0, "0 ä");
    assert_unsupported(5.0, "0;0 é");
    // The era and Buddhist-year codes and A/P beside digits are unmeasured.
    assert_unsupported(5.0, "abc");
    assert_unsupported(0.0, "g");
    assert_unsupported(5.0, "0 a/p;0");
    assert_unsupported(1.0, "0 a/p");
}

#[test]
fn text_takes_the_text_section_as_in_mac_excel_16_114() {
    let formatted = |format: &str| format_text("abc", format, CONTEXT);
    for (format, expected) in [
        ("0", "abc"),
        ("0.00", "abc"),
        ("0.00E+00", "abc"),
        ("yyyy", "abc"),
        ("\"lit\"", "abc"),
        ("", "abc"),
        ("@", "abc"),
        ("@@", "abcabc"),
        ("0;0;0;\"x\"@", "xabc"),
        ("0;\"x\"@", "xabc"),
        ("0;0;\"x\"@", "xabc"),
        ("0;@", "abc"),
        ("0;0;0;\"lit\"", "lit"),
        ("0;0;0;", ""),
        ("[<5]0", "abc"),
        ("[>5]0;@", "abc"),
        ("[Red]@", "abc"),
        ("_(@_)", " abc "),
        ("*x@", "abc"),
        ("0원", "abc"),
        ("0/2", "abc"),
        ("0 a/p", "abc"),
        ("0.0.0", "abc"),
    ] {
        assert_eq!(formatted(format).as_deref(), Ok(expected), "{format:?}");
    }
    for format in [
        "General",
        "0;General",
        "General;0",
        "0;0;0;General",
        "General@",
        "@;0",
        "0;@;0",
        "\"x\"@;0;0;0",
        "0;0;0;0",
        "0;0;0;[<5]@",
        "0n",
        "TRUE",
        "0 d",
        "d 0",
        "0;0 d",
        "0 g",
        "0 aaa",
        "yyyy n",
    ] {
        assert_eq!(formatted(format), Err(ErrorKind::Value), "{format:?}");
    }
    // A cell shows text in `General` and has no fill width.
    let display = FormatContext {
        formula: false,
        ..CONTEXT
    };
    assert_eq!(format_text("abc", "General", display).as_deref(), Ok("abc"));
    assert_eq!(
        format_text("abc", "*x@", display),
        Err(ErrorKind::Unsupported)
    );
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
    assert_unsupported(5.0, "[Color57]0");
    assert_unsupported(5.0, "[Color0]0");
}

#[test]
fn excel_16_114_measurements_of_fills_palette_colors_fractions_and_seconds() {
    // excel-measure RESULTS.md, section B.
    assert_text(1.5, "*x0", "2");
    assert_text(1.5, "0*x", "2");
    assert_text(1.5, "_x0", " 2");
    assert_text(1.0, "[Color5]0", "1");
    assert_text(1.0, "[color56]0.0", "1.0");
    assert_text(1.999, "# ?/?", "2    ");
    assert_text(5.0, "# ???/???", "5        ");
    assert_text(-0.25, "# ?/?", "- 1/4");
    assert_text(-1.999, "# ?/?", "-2    ");
    let time = |seconds: f64| (16.0 * 3_600.0 + 36.0 * 60.0 + 3.0) / 86_400.0 + seconds / 86_400.0;
    assert_text(time(0.5), "h:mm:ss", "16:36:03");
    assert_text(time(0.49), "h:mm:ss", "16:36:03");
    assert_text(time(0.75), "h:mm:ss", "16:36:04");
    assert_text(time(0.75), "h:mm:ss.0", "16:36:03.8");
    // A zero fraction without a whole part or beside `#`, and a negative number without a whole
    // part, are unmeasured.
    assert_unsupported(0.001, "# ?/?");
    assert_unsupported(1.999, "# #/#");
    assert_unsupported(-0.25, "?/?");
}

#[test]
fn unestablished_combinations_are_unsupported() {
    // A third section after an unconditional second one.
    assert_unsupported(2.0, "[>3]0;0.0;0.00");
    assert_text(2.0, "[>3]0;[<1]0.0;0.00", "2.00");
    // Elapsed time beside a half-day marker.
    assert_unsupported(0.75, "[h]:mm AM/PM");
    assert_unsupported(0.75, "[mm]:ss A/P");
    // A slash that does not form a `?/?` fraction; to Excel `0/2` is a fixed denominator
    // (`TEXT(1,"0/2")` is `2/2`).
    assert_unsupported(0.25, "#%/#");
    assert_unsupported(2.5, "0/2;0");
    assert_unsupported(1.0, "0/2");
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
        ..CONTEXT
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
    assert_text(46_230.0, "yyyyy", "2026");
    assert_unsupported(46_230.0, "eee");
}

#[test]
fn year_codes_follow_the_measured_lengths() {
    // Excel 16.114 shows DATE(2024,1,15) with "y yyy ee" as "24 2024 2024" and with "e" as
    // "2024" (excel-measure RESULTS.md, section B).
    assert_text(45_306.0, "y yyy ee", "24 2024 2024");
    assert_text(45_306.0, "e", "2024");
    assert_text(45_306.0, "y", "24");
    assert_text(45_306.0, "yyy", "2024");
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
    assert_text(12_345_678_901.0, "General", "12345678901");
    assert_text(-1_234_567_890.0, "General", "-1234567890");
    assert_text(0.3 + 0.000_000_000_000_000_04, "General", "0.3");
    assert_text(0.001, "General", "0.001");
}

#[test]
fn general_leaves_numbers_excel_shortens_unsupported() {
    // Excel shows "1.23457E+11", "3.141592654", and "-1.23457E+10".
    assert_text(1e20, "General", "1E+20");
    assert_unsupported(123_456_789_012.0, "General");
    assert_unsupported(std::f64::consts::PI, "General");
    assert_unsupported(-12_345_678_901.0, "General");
    assert_unsupported(0.000_01, "General");
}

#[test]
fn unmeasured_constructs_stay_unsupported() {
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

#[test]
fn korean_regional_format_leaves_unmeasured_names_unsupported() {
    let korean = FormatContext {
        locale: Locale::KoKr,
        ..CONTEXT
    };
    assert_eq!(
        format_number(46_230.0, "yyyy-mm-dd", korean).as_deref(),
        Ok("2026-07-27")
    );
    assert_eq!(
        format_number(46_230.0, "[$-409]mmm", korean).as_deref(),
        Ok("Jul")
    );
    for format in ["mmm", "dddd", "h AM/PM"] {
        assert_eq!(
            format_number(46_230.5, format, korean),
            Err(ErrorKind::Unsupported),
            "{format}"
        );
    }
}
