use cellrune_interop::{
    DateSystemDto, DisplayFormatRequestDto, DisplayFormatResultDto, LocaleDto, LocaleProfileDto,
    WritableCellValueDto, format_display_text,
};

fn request(value: WritableCellValueDto, format_code: &str) -> DisplayFormatRequestDto {
    DisplayFormatRequestDto {
        value,
        format_code: format_code.to_owned(),
        date_system: DateSystemDto::Excel1900,
        locale_profile: None,
    }
}

fn text(request: DisplayFormatRequestDto) -> String {
    format_display_text(request)
        .expect("display text renders")
        .text
}

#[test]
fn display_text_renders_values_with_number_format_codes() {
    assert_eq!(
        text(request(
            WritableCellValueDto::Number { value: 1234.567 },
            "#,##0.00;(#,##0.00)"
        )),
        "1,234.57"
    );
    assert_eq!(
        text(request(
            WritableCellValueDto::Text {
                value: "abc".to_owned()
            },
            "\"name: \"@"
        )),
        "name: abc"
    );
    assert_eq!(
        text(request(
            WritableCellValueDto::Logical { value: false },
            "0.00"
        )),
        "FALSE"
    );
    assert_eq!(
        text(DisplayFormatRequestDto {
            date_system: DateSystemDto::Excel1904,
            ..request(WritableCellValueDto::Number { value: 0.0 }, "yyyy-mm-dd")
        }),
        "1904-01-01"
    );

    let parsed: DisplayFormatRequestDto = serde_json::from_value(serde_json::json!({
        "value": {"kind": "number", "value": 46230.0},
        "format_code": "dddd",
    }))
    .expect("date system and locale profile are optional");
    assert_eq!(parsed.date_system, DateSystemDto::Excel1900);
    assert_eq!(
        serde_json::to_value(format_display_text(parsed).expect("weekday"))
            .expect("result serializes"),
        serde_json::to_value(DisplayFormatResultDto {
            text: "Monday".to_owned()
        })
        .expect("expected result serializes")
    );
}

#[test]
fn display_text_reports_unsupported_codes_and_invalid_values() {
    let fill = format_display_text(request(WritableCellValueDto::Number { value: 5.0 }, "*-0"))
        .expect_err("fills depend on the column width");
    assert_eq!(fill.code(), "display_format.unsupported");

    let korean_names = format_display_text(DisplayFormatRequestDto {
        locale_profile: Some(LocaleProfileDto {
            locale: LocaleDto::KoKr,
            display_language: None,
        }),
        ..request(WritableCellValueDto::Number { value: 46230.0 }, "mmmm")
    })
    .expect_err("Korean month names are unmeasured");
    assert_eq!(korean_names.code(), "display_format.unsupported");

    let unknown_error = format_display_text(request(
        WritableCellValueDto::Error {
            value: "#OOPS!".to_owned(),
        },
        "0",
    ))
    .expect_err("unknown error value");
    assert_eq!(unknown_error.code(), "interop.value.excel_error_invalid");
    assert!(
        serde_json::from_value::<DisplayFormatRequestDto>(serde_json::json!({
            "value": {"kind": "number", "value": 1.0},
            "format_code": "0",
            "date_system": "1904",
        }))
        .is_err()
    );
}
