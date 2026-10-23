use crate::CellRange;

/// The kind of value a data validation rule accepts, the OOXML `type` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataValidationType {
    /// Any value (`none`); the rule only carries an input message.
    AnyValue,
    /// A whole number compared with the rule's formulas (`whole`).
    Whole,
    /// A decimal number compared with the rule's formulas (`decimal`).
    Decimal,
    /// One of the values `formula1` lists or references (`list`).
    List,
    /// A date serial compared with the rule's formulas (`date`).
    Date,
    /// A time fraction compared with the rule's formulas (`time`).
    Time,
    /// A text whose length is compared with the rule's formulas (`textLength`).
    TextLength,
    /// A value for which `formula1` evaluates to TRUE (`custom`).
    Custom,
}

impl DataValidationType {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnyValue => "none",
            Self::Whole => "whole",
            Self::Decimal => "decimal",
            Self::List => "list",
            Self::Date => "date",
            Self::Time => "time",
            Self::TextLength => "textLength",
            Self::Custom => "custom",
        }
    }

    pub(crate) fn from_xlsx(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::AnyValue,
            "whole" => Self::Whole,
            "decimal" => Self::Decimal,
            "list" => Self::List,
            "date" => Self::Date,
            "time" => Self::Time,
            "textLength" => Self::TextLength,
            "custom" => Self::Custom,
            _ => return None,
        })
    }
}

/// How a whole, decimal, date, time, or text-length rule compares a value with its formulas,
/// the OOXML `operator` attribute. Other rule types ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataValidationOperator {
    /// Between `formula1` and `formula2`, inclusive (`between`).
    Between,
    /// Outside `formula1` through `formula2` (`notBetween`).
    NotBetween,
    /// Equal to `formula1` (`equal`).
    Equal,
    /// Not equal to `formula1` (`notEqual`).
    NotEqual,
    /// Less than `formula1` (`lessThan`).
    LessThan,
    /// Less than or equal to `formula1` (`lessThanOrEqual`).
    LessThanOrEqual,
    /// Greater than `formula1` (`greaterThan`).
    GreaterThan,
    /// Greater than or equal to `formula1` (`greaterThanOrEqual`).
    GreaterThanOrEqual,
}

impl DataValidationOperator {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Between => "between",
            Self::NotBetween => "notBetween",
            Self::Equal => "equal",
            Self::NotEqual => "notEqual",
            Self::LessThan => "lessThan",
            Self::LessThanOrEqual => "lessThanOrEqual",
            Self::GreaterThan => "greaterThan",
            Self::GreaterThanOrEqual => "greaterThanOrEqual",
        }
    }

    pub(crate) fn from_xlsx(token: &str) -> Option<Self> {
        Some(match token {
            "between" => Self::Between,
            "notBetween" => Self::NotBetween,
            "equal" => Self::Equal,
            "notEqual" => Self::NotEqual,
            "lessThan" => Self::LessThan,
            "lessThanOrEqual" => Self::LessThanOrEqual,
            "greaterThan" => Self::GreaterThan,
            "greaterThanOrEqual" => Self::GreaterThanOrEqual,
            _ => return None,
        })
    }
}

/// What the spreadsheet app does with an invalid entry, the OOXML `errorStyle` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataValidationErrorStyle {
    /// Reject the entry (`stop`).
    Stop,
    /// Warn and let the user keep the entry (`warning`).
    Warning,
    /// Inform and keep the entry (`information`).
    Information,
}

impl DataValidationErrorStyle {
    /// Returns the OOXML token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Warning => "warning",
            Self::Information => "information",
        }
    }

    pub(crate) fn from_xlsx(token: &str) -> Option<Self> {
        Some(match token {
            "stop" => Self::Stop,
            "warning" => Self::Warning,
            "information" => Self::Information,
            _ => return None,
        })
    }
}

/// The attributes of a data validation rule, as the reader parses them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataValidationRule {
    pub(crate) validation_type: DataValidationType,
    pub(crate) operator: DataValidationOperator,
    pub(crate) error_style: DataValidationErrorStyle,
    pub(crate) allow_blank: bool,
    pub(crate) in_cell_dropdown: bool,
    pub(crate) show_input_message: bool,
    pub(crate) show_error_message: bool,
    pub(crate) error_title: Option<Box<str>>,
    pub(crate) error: Option<Box<str>>,
    pub(crate) prompt_title: Option<Box<str>>,
    pub(crate) prompt: Option<Box<str>>,
}

/// One worksheet data validation rule and the ranges it applies to.
///
/// The model is read-only: the rule's XML stays in the worksheet part, which a document-backed
/// writer preserves. Formulas keep their stored text, without a leading `=`, and a sheet rename
/// rewrites their sheet qualifiers as it does in the saved part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataValidation {
    ranges: Vec<CellRange>,
    rule: DataValidationRule,
    formula1: Option<Box<str>>,
    formula2: Option<Box<str>>,
}

impl DataValidation {
    pub(crate) const fn from_xlsx(
        ranges: Vec<CellRange>,
        rule: DataValidationRule,
        formula1: Option<Box<str>>,
        formula2: Option<Box<str>>,
    ) -> Self {
        Self {
            ranges,
            rule,
            formula1,
            formula2,
        }
    }

    /// Returns the ranges the rule applies to, in `sqref` order.
    pub fn ranges(&self) -> &[CellRange] {
        &self.ranges
    }

    /// Returns the kind of value the rule accepts.
    pub const fn validation_type(&self) -> DataValidationType {
        self.rule.validation_type
    }

    /// Returns the comparison operator; `Between` when the source omits it.
    pub const fn operator(&self) -> DataValidationOperator {
        self.rule.operator
    }

    /// Returns the first formula: the list source, the custom condition, or the first bound.
    pub fn formula1(&self) -> Option<&str> {
        self.formula1.as_deref()
    }

    /// Returns the second bound of a `Between` or `NotBetween` rule.
    pub fn formula2(&self) -> Option<&str> {
        self.formula2.as_deref()
    }

    /// Returns whether a blank cell passes the rule.
    pub const fn allow_blank(&self) -> bool {
        self.rule.allow_blank
    }

    /// Returns whether a list rule shows its in-cell dropdown.
    ///
    /// OOXML stores the inverse: `showDropDown="1"` hides the dropdown.
    pub const fn in_cell_dropdown(&self) -> bool {
        self.rule.in_cell_dropdown
    }

    /// Returns whether the input message is shown when a cell is selected.
    pub const fn show_input_message(&self) -> bool {
        self.rule.show_input_message
    }

    /// Returns whether the error alert is shown for an invalid entry.
    pub const fn show_error_message(&self) -> bool {
        self.rule.show_error_message
    }

    /// Returns what the app does with an invalid entry.
    pub const fn error_style(&self) -> DataValidationErrorStyle {
        self.rule.error_style
    }

    /// Returns the error alert title.
    pub fn error_title(&self) -> Option<&str> {
        self.rule.error_title.as_deref()
    }

    /// Returns the error alert text.
    pub fn error_message(&self) -> Option<&str> {
        self.rule.error.as_deref()
    }

    /// Returns the input message title.
    pub fn prompt_title(&self) -> Option<&str> {
        self.rule.prompt_title.as_deref()
    }

    /// Returns the input message text.
    pub fn prompt_message(&self) -> Option<&str> {
        self.rule.prompt.as_deref()
    }

    pub(crate) fn formulas_mut(&mut self) -> [&mut Option<Box<str>>; 2] {
        [&mut self.formula1, &mut self.formula2]
    }
}
