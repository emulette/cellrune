//! Built-in locale conventions. Calculation follows the [`LocaleProfile`] in its options and never
//! reads the host locale.

/// A locale built into CellRune.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Locale {
    /// English (United States), the conventions CellRune follows when no profile is supplied.
    #[default]
    EnUs,
    /// Korean (Korea).
    KoKr,
}

/// The locale conventions a calculation follows.
///
/// `locale` stands for Excel's regional format: the `DOLLAR` currency symbol and the decimals it
/// shows when the formula omits them, and the month, weekday, and `AM`/`PM` names of `TEXT`.
/// `display_language` stands for the language of the labels Excel generates, such as the
/// `GROUPBY` and `PIVOTBY` total row; it follows `locale` unless set. Both built-in locales use
/// `.` as the decimal separator and `,` as the thousands separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LocaleProfile {
    locale: Locale,
    display_language: Locale,
}

impl LocaleProfile {
    /// Follows `locale` for both the regional format and the generated labels.
    pub const fn new(locale: Locale) -> Self {
        Self {
            locale,
            display_language: locale,
        }
    }

    /// Selects the language of generated labels separately from the regional format, as an Excel
    /// with an English interface and Korean regional settings may show `Total` beside `₩`.
    pub const fn with_display_language(mut self, display_language: Locale) -> Self {
        self.display_language = display_language;
        self
    }

    /// Returns the regional-format locale.
    pub const fn locale(self) -> Locale {
        self.locale
    }

    /// Returns the language of generated labels.
    pub const fn display_language(self) -> Locale {
        self.display_language
    }
}

impl Locale {
    /// The currency symbol `DOLLAR` places before the amount.
    pub(in crate::calculation) const fn currency_symbol(self) -> &'static str {
        match self {
            Self::EnUs => "$",
            Self::KoKr => "₩",
        }
    }

    /// The decimals `DOLLAR` shows when the formula omits them: Excel with Korean regional
    /// settings writes `DOLLAR(1)` as `₩1`.
    pub(in crate::calculation) const fn currency_decimals(self) -> i32 {
        match self {
            Self::EnUs => 2,
            Self::KoKr => 0,
        }
    }

    /// The label of a `GROUPBY` or `PIVOTBY` total.
    pub(in crate::calculation) const fn total_label(self) -> &'static str {
        match self {
            Self::EnUs => "Total",
            Self::KoKr => "합계",
        }
    }

    /// The label of a grand total over several grouping levels, or `None` where it has not been
    /// established.
    pub(in crate::calculation) const fn grand_total_label(self) -> Option<&'static str> {
        match self {
            Self::EnUs => Some("Grand Total"),
            Self::KoKr => None,
        }
    }
}
