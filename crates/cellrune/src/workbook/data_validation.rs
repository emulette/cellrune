use crate::{CellRange, ValidationError};

/// Excel keeps at most this many references in a rule's `sqref`.
const MAX_RANGES: usize = 32_767;
/// Excel's length limits for a rule's titles and messages, in UTF-16 code units.
const MAX_PROMPT_TITLE: usize = 32;
const MAX_PROMPT: usize = 255;
const MAX_ERROR_TITLE: usize = 32;
const MAX_ERROR: usize = 224;

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

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
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

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
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

    /// Parses an OOXML token.
    pub fn from_token(token: &str) -> Option<Self> {
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

impl DataValidationRule {
    /// Reads the attributes of a `dataValidation` element by their unqualified names, giving an
    /// omitted one its OOXML default. Returns `None` for a token OOXML does not define.
    pub(crate) fn from_attributes<'a>(attribute: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        let token = |name, default| attribute(name).unwrap_or(default);
        let flag = |name| match attribute(name) {
            None | Some("0" | "false") => Some(false),
            Some("1" | "true") => Some(true),
            Some(_) => None,
        };
        let text = |name| attribute(name).map(Box::from);
        Some(Self {
            validation_type: DataValidationType::from_token(token("type", "none"))?,
            operator: DataValidationOperator::from_token(token("operator", "between"))?,
            error_style: DataValidationErrorStyle::from_token(token("errorStyle", "stop"))?,
            allow_blank: flag("allowBlank")?,
            in_cell_dropdown: !flag("showDropDown")?,
            show_input_message: flag("showInputMessage")?,
            show_error_message: flag("showErrorMessage")?,
            error_title: text("errorTitle"),
            error: text("error"),
            prompt_title: text("promptTitle"),
            prompt: text("prompt"),
        })
    }
}

/// Parses an `sqref` list of space-separated cell and range references.
pub(crate) fn parse_sqref(sqref: &str) -> Option<Vec<CellRange>> {
    let ranges = sqref
        .split_ascii_whitespace()
        .map(CellRange::from_sqref_item)
        .collect::<Option<Vec<_>>>()?;
    (!ranges.is_empty()).then_some(ranges)
}

/// Whether XML 1.0 carries `character`: a tab, line feed, carriage return, or any character from
/// space on except U+FFFE and U+FFFF.
const fn is_xml_char(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\r')
        || (character >= ' ' && character != '\u{FFFE}' && character != '\u{FFFF}')
}

/// One worksheet data validation rule and the ranges it applies to.
///
/// Formulas keep their stored text, without a leading `=`, and a sheet rename rewrites their
/// sheet qualifiers as it does in the saved part. [`DataValidation::new`] and the `with_`
/// methods build a rule for [`crate::WorkbookChange::SetDataValidation`]; a rule starts with the
/// values OOXML gives an attribute the source omits, which the reader also returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataValidation {
    ranges: Vec<CellRange>,
    rule: DataValidationRule,
    formula1: Option<Box<str>>,
    formula2: Option<Box<str>>,
}

impl DataValidation {
    /// Builds a rule of `validation_type` over `ranges`: operator `Between`, error style `Stop`,
    /// blank cells rejected, the in-cell dropdown shown, no messages, and no formulas.
    pub fn new(ranges: Vec<CellRange>, validation_type: DataValidationType) -> Self {
        Self {
            ranges,
            rule: DataValidationRule {
                validation_type,
                operator: DataValidationOperator::Between,
                error_style: DataValidationErrorStyle::Stop,
                allow_blank: false,
                in_cell_dropdown: true,
                show_input_message: false,
                show_error_message: false,
                error_title: None,
                error: None,
                prompt_title: None,
                prompt: None,
            },
            formula1: None,
            formula2: None,
        }
    }

    /// Sets the comparison operator.
    #[must_use]
    pub fn with_operator(mut self, operator: DataValidationOperator) -> Self {
        self.rule.operator = operator;
        self
    }

    /// Sets the first formula, without a leading `=`: a list source (a range, a defined name, or
    /// a quoted comma-separated list), a custom condition, or the first bound.
    #[must_use]
    pub fn with_formula1(mut self, formula: impl Into<Box<str>>) -> Self {
        self.formula1 = Some(formula.into());
        self
    }

    /// Sets the second bound of a `Between` or `NotBetween` rule, without a leading `=`.
    #[must_use]
    pub fn with_formula2(mut self, formula: impl Into<Box<str>>) -> Self {
        self.formula2 = Some(formula.into());
        self
    }

    /// Sets whether a blank cell passes the rule.
    #[must_use]
    pub fn with_allow_blank(mut self, allow_blank: bool) -> Self {
        self.rule.allow_blank = allow_blank;
        self
    }

    /// Sets whether a list rule shows its in-cell dropdown.
    #[must_use]
    pub fn with_in_cell_dropdown(mut self, in_cell_dropdown: bool) -> Self {
        self.rule.in_cell_dropdown = in_cell_dropdown;
        self
    }

    /// Sets whether the input message shows when a cell is selected, and its title and text.
    #[must_use]
    pub fn with_input_message(
        mut self,
        show: bool,
        title: Option<&str>,
        message: Option<&str>,
    ) -> Self {
        self.rule.show_input_message = show;
        self.rule.prompt_title = title.map(Box::from);
        self.rule.prompt = message.map(Box::from);
        self
    }

    /// Sets whether the error alert shows for an invalid entry, what the app then does, and the
    /// alert's title and text.
    #[must_use]
    pub fn with_error_alert(
        mut self,
        show: bool,
        style: DataValidationErrorStyle,
        title: Option<&str>,
        message: Option<&str>,
    ) -> Self {
        self.rule.show_error_message = show;
        self.rule.error_style = style;
        self.rule.error_title = title.map(Box::from);
        self.rule.error = message.map(Box::from);
        self
    }

    /// Checks that the rule can be written: it applies to a range, and its formulas, titles, and
    /// messages hold only characters XML 1.0 carries. A rule read from a file passes, whatever
    /// the tool that wrote it allowed.
    pub(crate) fn check_writable(&self, sheet_id: u32) -> Result<(), ValidationError> {
        if self.ranges.is_empty() {
            return Err(ValidationError::DataValidationRangesInvalid { sheet_id, count: 0 });
        }
        for (attribute, text) in [
            ("formula1", self.formula1.as_deref()),
            ("formula2", self.formula2.as_deref()),
            ("promptTitle", self.rule.prompt_title.as_deref()),
            ("prompt", self.rule.prompt.as_deref()),
            ("errorTitle", self.rule.error_title.as_deref()),
            ("error", self.rule.error.as_deref()),
        ] {
            if text.is_some_and(|text| text.chars().any(|character| !is_xml_char(character))) {
                return Err(ValidationError::DataValidationTextInvalid {
                    sheet_id,
                    attribute,
                });
            }
        }
        Ok(())
    }

    /// Checks a new rule as Excel keeps it: it can be written, applies to at most 32,767 ranges,
    /// has the formulas its type and operator need, formulas that parse, and titles and messages
    /// within Excel's lengths.
    pub(crate) fn check(&self, sheet_id: u32) -> Result<(), ValidationError> {
        self.check_writable(sheet_id)?;
        if self.ranges.len() > MAX_RANGES {
            return Err(ValidationError::DataValidationRangesInvalid {
                sheet_id,
                count: self.ranges.len(),
            });
        }
        let (needs_formula1, needs_formula2) = match self.rule.validation_type {
            DataValidationType::AnyValue => (false, false),
            DataValidationType::List | DataValidationType::Custom => (true, false),
            DataValidationType::Whole
            | DataValidationType::Decimal
            | DataValidationType::Date
            | DataValidationType::Time
            | DataValidationType::TextLength => (
                true,
                matches!(
                    self.rule.operator,
                    DataValidationOperator::Between | DataValidationOperator::NotBetween
                ),
            ),
        };
        for (slot, formula, needed) in [
            (1, &self.formula1, needs_formula1),
            (2, &self.formula2, needs_formula2),
        ] {
            match formula {
                None if needed => {
                    return Err(ValidationError::DataValidationFormulaMissing { sheet_id, slot });
                }
                Some(text) if !crate::calculation::formula_parses(text) => {
                    return Err(ValidationError::DataValidationFormulaInvalid { sheet_id, slot });
                }
                _ => {}
            }
        }
        for (attribute, text, limit) in [
            ("promptTitle", &self.rule.prompt_title, MAX_PROMPT_TITLE),
            ("prompt", &self.rule.prompt, MAX_PROMPT),
            ("errorTitle", &self.rule.error_title, MAX_ERROR_TITLE),
            ("error", &self.rule.error, MAX_ERROR),
        ] {
            if text
                .as_deref()
                .is_some_and(|text| text.encode_utf16().count() > limit)
            {
                return Err(ValidationError::DataValidationTextTooLong {
                    sheet_id,
                    attribute,
                    limit,
                });
            }
        }
        Ok(())
    }

    /// Returns whether `other` is the same rule, whatever ranges each applies to.
    pub(crate) fn same_rule(&self, other: &Self) -> bool {
        self.rule == other.rule
            && self.formula1 == other.formula1
            && self.formula2 == other.formula2
    }

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

    pub(crate) fn set_ranges(&mut self, ranges: Vec<CellRange>) {
        self.ranges = ranges;
    }
}
