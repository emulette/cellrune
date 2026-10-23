//! Stable codes the interop layer reports for its own failures; lower layers keep theirs.

/// A stable dotted error code owned by the interop layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteropCode {
    /// Targeted calculation payload is invalid.
    CalculationTargetInvalidPayload,
    /// Workbook session is closed.
    SessionClosed,
    /// Workbook session state is unavailable.
    SessionUnavailable,
    /// Workbook session state is unusable after a native panic.
    SessionPoisoned,
    /// Workbook does not contain the requested sheet.
    SheetNotFound,
    /// Page size exceeds the interop limit.
    PageLimitExceeded,
    /// Range page cannot contain its first position within the byte limit.
    PageByteLimitExceeded,
    /// Page offset exceeds the requested range.
    PageOffsetOutOfRange,
    /// Page offset must be a non-negative integer.
    PageOffsetInvalid,
    /// Page limit must be a non-negative integer within the unsigned 32-bit range.
    PageLimitInvalid,
    /// Calculate the current workbook revision before saving.
    CalculationRequired,
    /// Excel error value is not recognized.
    ValueExcelErrorInvalid,
    /// Workbook change failed transport validation.
    ChangeInvalid,
    /// Sheet identifier allocation failed.
    SheetIdExhausted,
    /// Calculation request identifier is exhausted.
    CalculationRequestIdExhausted,
    /// Edit batch payload is invalid.
    ChangePayloadInvalid,
    /// Revision or cursor must be an unsigned 64-bit integer.
    SessionRevisionOrCursorInvalid,
    /// Recalculation mode must be auto, incremental, or full.
    CalculationModeInvalid,
    /// Arithmetic semantics must be excel_near_zero or ieee_754.
    CalculationArithmeticSemanticsInvalid,
    /// Financial solver semantics must be excel_iteration_budget or extended_search.
    CalculationFinancialSolverSemanticsInvalid,
    /// Display format request is invalid.
    DisplayFormatRequestInvalid,
    /// Locale profile must be an object whose locale and display_language are en-US or ko-KR.
    CalculationLocaleProfileInvalid,
    /// Number must be an integer or floating-point value; booleans are not accepted.
    ValueNumberInvalid,
    /// Resource limit must be greater than zero.
    LimitInvalid,
    /// Resource limits must map known limit names to positive integers.
    LimitPayloadInvalid,
    /// Archive byte limit must be greater than zero.
    WorkbookArchiveLimitInvalid,
    /// Defined-name analysis returned an unknown sheet identity.
    DefinedNameSheetIdentityInvalid,
    /// Workbook preview is not published in this session.
    PreviewNotFound,
    /// Preview cursor does not belong to this preview.
    PreviewCursorInvalid,
    /// Preview response cannot contain one complete detail item within the byte limit.
    PreviewResponseLimitExceeded,
    /// Workbook preview identifier is exhausted.
    PreviewIdExhausted,
    /// Preview detail section is not recognized.
    PreviewSectionInvalid,
    /// Interop DTO serialization failed.
    DtoSerialization,
}

impl InteropCode {
    /// Returns the stable dotted code.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CalculationTargetInvalidPayload => "interop.calculation.target.invalid_payload",
            Self::SessionClosed => "interop.session.closed",
            Self::SessionUnavailable => "interop.session.unavailable",
            Self::SessionPoisoned => "interop.session.poisoned",
            Self::SheetNotFound => "interop.sheet.not_found",
            Self::PageLimitExceeded => "interop.page.limit_exceeded",
            Self::PageByteLimitExceeded => "interop.page.byte_limit_exceeded",
            Self::PageOffsetOutOfRange => "interop.page.offset_out_of_range",
            Self::PageOffsetInvalid => "interop.page.offset_invalid",
            Self::PageLimitInvalid => "interop.page.limit_invalid",
            Self::CalculationRequired => "interop.calculation.required",
            Self::ValueExcelErrorInvalid => "interop.value.excel_error_invalid",
            Self::ChangeInvalid => "interop.change.invalid",
            Self::SheetIdExhausted => "interop.sheet.id_exhausted",
            Self::CalculationRequestIdExhausted => "interop.calculation.request_id_exhausted",
            Self::ChangePayloadInvalid => "interop.change.payload_invalid",
            Self::SessionRevisionOrCursorInvalid => "interop.session.revision_or_cursor_invalid",
            Self::CalculationModeInvalid => "interop.calculation.mode_invalid",
            Self::CalculationArithmeticSemanticsInvalid => {
                "interop.calculation.arithmetic_semantics_invalid"
            }
            Self::CalculationFinancialSolverSemanticsInvalid => {
                "interop.calculation.financial_solver_semantics_invalid"
            }
            Self::DisplayFormatRequestInvalid => "interop.display_format.request_invalid",
            Self::CalculationLocaleProfileInvalid => "interop.calculation.locale_profile_invalid",
            Self::ValueNumberInvalid => "interop.value.number_invalid",
            Self::LimitInvalid => "interop.limit.invalid",
            Self::LimitPayloadInvalid => "interop.limit.payload_invalid",
            Self::WorkbookArchiveLimitInvalid => "interop.workbook.archive_limit_invalid",
            Self::DefinedNameSheetIdentityInvalid => "interop.defined_name.sheet_identity_invalid",
            Self::PreviewNotFound => "interop.preview.not_found",
            Self::PreviewCursorInvalid => "interop.preview.cursor_invalid",
            Self::PreviewResponseLimitExceeded => "interop.preview.response_limit_exceeded",
            Self::PreviewIdExhausted => "interop.preview.id_exhausted",
            Self::PreviewSectionInvalid => "interop.preview.section_invalid",
            Self::DtoSerialization => "interop.dto.serialization",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::InteropCode;

    /// Lists the codes of every variant. Naming each variant in an exhaustive match makes this
    /// fail to compile when the enum gains one, so a new code cannot skip the registry.
    macro_rules! codes {
        ($enum:ident :: $method:ident; $($variant:ident),+ $(,)?) => {{
            fn exhaustive(code: $enum) {
                match code {
                    $($enum::$variant)|+ => {}
                }
            }
            let _: fn($enum) = exhaustive;
            vec![$($enum::$variant.$method()),+]
        }};
    }

    /// Returns the codes in the first column of the tables under the `## {section}` heading.
    fn documented(section: &str) -> BTreeSet<String> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ERROR_CODES.md");
        let document = std::fs::read_to_string(path).expect("docs/ERROR_CODES.md is readable");
        let heading = format!("## {section}");
        document
            .lines()
            .skip_while(|line| *line != heading)
            .skip(1)
            .take_while(|line| !line.starts_with("## "))
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|line| line.split_once('`').map(|(code, _)| code.to_owned()))
            .collect()
    }

    #[test]
    fn the_error_code_reference_lists_every_interop_code() {
        let codes = codes!(InteropCode::as_str;
            CalculationTargetInvalidPayload,
            SessionClosed,
            SessionUnavailable,
            SessionPoisoned,
            SheetNotFound,
            PageLimitExceeded,
            PageByteLimitExceeded,
            PageOffsetOutOfRange,
            PageOffsetInvalid,
            PageLimitInvalid,
            CalculationRequired,
            ValueExcelErrorInvalid,
            ChangeInvalid,
            SheetIdExhausted,
            CalculationRequestIdExhausted,
            ChangePayloadInvalid,
            SessionRevisionOrCursorInvalid,
            CalculationModeInvalid,
            CalculationArithmeticSemanticsInvalid,
            CalculationFinancialSolverSemanticsInvalid,
            DisplayFormatRequestInvalid,
            CalculationLocaleProfileInvalid,
            ValueNumberInvalid,
            LimitInvalid,
            LimitPayloadInvalid,
            WorkbookArchiveLimitInvalid,
            DefinedNameSheetIdentityInvalid,
            PreviewNotFound,
            PreviewCursorInvalid,
            PreviewResponseLimitExceeded,
            PreviewIdExhausted,
            PreviewSectionInvalid,
            DtoSerialization,
        )
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        let documented = documented("Interop");
        let undocumented = codes.difference(&documented).collect::<Vec<_>>();
        let stale = documented.difference(&codes).collect::<Vec<_>>();
        assert!(
            undocumented.is_empty() && stale.is_empty(),
            "docs/ERROR_CODES.md must list exactly these codes; undocumented: {undocumented:?}, \
             not defined: {stale:?}"
        );
    }
}
