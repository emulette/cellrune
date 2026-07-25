use crate::{FormulaText, SheetId, ValidationError};

/// Visibility scope of a workbook defined name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DefinedNameScope {
    /// The name is visible throughout the workbook.
    #[default]
    Workbook,
    /// The name is visible only within one sheet.
    Sheet(SheetId),
}

/// A validated workbook or sheet-local name formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedName {
    name: Box<str>,
    lookup_key: Box<str>,
    scope: DefinedNameScope,
    formula: FormulaText,
    hidden: bool,
}

impl DefinedName {
    /// Validates a defined name while preserving its raw formula text and scope.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the name is empty, too long, or contains a control
    /// character.
    pub fn new(
        name: impl Into<String>,
        scope: DefinedNameScope,
        formula: FormulaText,
        hidden: bool,
    ) -> Result<Self, ValidationError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ValidationError::DefinedNameEmpty);
        }
        let utf16_len = name.encode_utf16().count();
        if utf16_len > 255 {
            return Err(ValidationError::DefinedNameTooLong { utf16_len });
        }
        if let Some(character) = name.chars().find(|character| character.is_control()) {
            return Err(ValidationError::DefinedNameControlCharacter { character });
        }
        let lookup_key = case_insensitive_key(&name).into_boxed_str();
        Ok(Self {
            name: name.into_boxed_str(),
            lookup_key,
            scope,
            formula,
            hidden,
        })
    }

    /// Returns the original name spelling.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the workbook or sheet-local visibility scope.
    pub const fn scope(&self) -> DefinedNameScope {
        self.scope
    }

    /// Returns the raw name formula without a leading equals sign.
    pub const fn formula(&self) -> &FormulaText {
        &self.formula
    }

    /// Returns whether the producer hides this name from the normal UI.
    pub const fn hidden(&self) -> bool {
        self.hidden
    }

    pub(crate) fn lookup_key(&self) -> &str {
        &self.lookup_key
    }
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}
