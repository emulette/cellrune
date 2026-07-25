use crate::ValidationError;

/// A half-open range in zero-based UTF-16 code units over the base cell text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhoneticTextRange {
    start_utf16: u32,
    end_utf16: u32,
}

impl PhoneticTextRange {
    /// Validates and constructs a non-empty half-open UTF-16 range.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::PhoneticRangeEmpty`] when `start_utf16 >= end_utf16`.
    pub fn new(start_utf16: u32, end_utf16: u32) -> Result<Self, ValidationError> {
        if start_utf16 >= end_utf16 {
            return Err(ValidationError::PhoneticRangeEmpty {
                start: start_utf16,
                end: end_utf16,
            });
        }
        Ok(Self {
            start_utf16,
            end_utf16,
        })
    }

    /// Returns the inclusive zero-based UTF-16 start offset.
    pub const fn start_utf16(self) -> u32 {
        self.start_utf16
    }

    /// Returns the exclusive zero-based UTF-16 end offset.
    pub const fn end_utf16(self) -> u32 {
        self.end_utf16
    }
}

/// Character conversion requested for displayed phonetic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PhoneticType {
    /// Convert Japanese phonetics to half-width katakana.
    HalfWidthKatakana,
    /// Convert Japanese phonetics to full-width katakana.
    FullWidthKatakana,
    /// Convert Japanese phonetics to hiragana.
    Hiragana,
    /// Display the stored phonetic text without character-set conversion.
    NoConversion,
}

/// Horizontal alignment of phonetic text over its base text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PhoneticAlignment {
    /// Let the spreadsheet consumer choose its normal alignment.
    NoControl,
    /// Align phonetic text to the left.
    Left,
    /// Center phonetic text over its base range.
    Center,
    /// Distribute phonetic text over its base range.
    Distributed,
}

/// Display properties attached to a phonetic string item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhoneticProperties {
    font_id: u32,
    effective_font_id: u32,
    phonetic_type: Option<PhoneticType>,
    alignment: Option<PhoneticAlignment>,
}

impl PhoneticProperties {
    /// Constructs properties that reference one workbook font record.
    pub const fn new(font_id: u32) -> Self {
        Self {
            font_id,
            effective_font_id: font_id,
            phonetic_type: None,
            alignment: None,
        }
    }

    pub(crate) const fn from_source(font_id: u32, font_count: u32) -> Self {
        Self {
            font_id,
            effective_font_id: if font_id < font_count { font_id } else { 0 },
            phonetic_type: None,
            alignment: None,
        }
    }

    /// Returns the zero-based workbook font identifier.
    pub const fn font_id(&self) -> u32 {
        self.font_id
    }

    /// Returns the font identifier a consumer can safely resolve.
    ///
    /// A malformed source reference is preserved by [`Self::font_id`] while this accessor falls
    /// back to the workbook's default font record (`0`).
    pub const fn effective_font_id(&self) -> u32 {
        self.effective_font_id
    }

    /// Returns the requested phonetic character conversion, when explicitly declared.
    pub const fn phonetic_type(&self) -> Option<PhoneticType> {
        self.phonetic_type
    }

    /// Returns the requested phonetic alignment, when explicitly declared.
    pub const fn alignment(&self) -> Option<PhoneticAlignment> {
        self.alignment
    }

    /// Replaces the explicit phonetic character conversion.
    pub const fn with_phonetic_type(mut self, phonetic_type: PhoneticType) -> Self {
        self.phonetic_type = Some(phonetic_type);
        self
    }

    /// Replaces the explicit phonetic alignment.
    pub const fn with_alignment(mut self, alignment: PhoneticAlignment) -> Self {
        self.alignment = Some(alignment);
        self
    }
}

/// One phonetic string displayed over a range of literal base text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhoneticRun {
    base_range: PhoneticTextRange,
    text: Box<str>,
}

/// Visibility and display properties used when authoring phonetic text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PhoneticWriteOptions {
    visible: bool,
    properties: Option<PhoneticProperties>,
}

impl PhoneticWriteOptions {
    /// Constructs options with an explicit Cell `ph` visibility value.
    pub const fn new(visible: bool) -> Self {
        Self {
            visible,
            properties: None,
        }
    }

    /// Constructs options that display the authored phonetic guide.
    pub const fn show() -> Self {
        Self::new(true)
    }

    /// Constructs options that retain the annotation without displaying it by default.
    pub const fn hide() -> Self {
        Self::new(false)
    }

    /// Replaces the string-item display properties.
    pub const fn with_properties(mut self, properties: PhoneticProperties) -> Self {
        self.properties = Some(properties);
        self
    }

    /// Returns the explicit Cell visibility to serialize.
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// Returns the display properties to serialize, when supplied.
    pub const fn properties(&self) -> Option<&PhoneticProperties> {
        self.properties.as_ref()
    }
}

impl PhoneticRun {
    /// Validates and constructs one phonetic run.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the text is empty or contains an XML 1.0 character that
    /// cannot be represented in an XLSX string.
    pub fn new(
        base_range: PhoneticTextRange,
        text: impl Into<Box<str>>,
    ) -> Result<Self, ValidationError> {
        let text = text.into();
        validate_phonetic_text(&text)?;
        Ok(Self { base_range, text })
    }

    /// Returns the half-open base-text range.
    pub const fn base_range(&self) -> PhoneticTextRange {
        self.base_range
    }

    /// Returns the displayed phonetic text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PhoneticAnnotation {
    runs: Vec<PhoneticRun>,
    properties: Option<PhoneticProperties>,
}

impl PhoneticAnnotation {
    pub(crate) fn new(runs: Vec<PhoneticRun>, properties: Option<PhoneticProperties>) -> Self {
        Self { runs, properties }
    }

    pub(crate) fn runs(&self) -> &[PhoneticRun] {
        &self.runs
    }

    pub(crate) const fn properties(&self) -> Option<&PhoneticProperties> {
        self.properties.as_ref()
    }
}

pub(crate) fn validate_authoring_runs(
    base_text: &str,
    runs: &[PhoneticRun],
) -> Result<(), ValidationError> {
    if validate_source_runs(base_text, runs)? {
        return Err(ValidationError::PhoneticRunsOutOfOrder);
    }
    Ok(())
}

pub(crate) fn validate_source_runs(
    base_text: &str,
    runs: &[PhoneticRun],
) -> Result<bool, ValidationError> {
    let base_utf16_len = u32::try_from(base_text.encode_utf16().count()).unwrap_or(u32::MAX);
    let mut previous_end = 0_u32;
    let mut overlaps_or_reorders = false;
    for run in runs {
        let range = run.base_range();
        if range.end_utf16() > base_utf16_len {
            return Err(ValidationError::PhoneticRangeOutOfBounds {
                end: range.end_utf16(),
                base_utf16_len,
            });
        }
        validate_utf16_boundary(base_text, range.start_utf16())?;
        validate_utf16_boundary(base_text, range.end_utf16())?;
        if range.start_utf16() < previous_end {
            overlaps_or_reorders = true;
        }
        previous_end = range.end_utf16();
    }
    Ok(overlaps_or_reorders)
}

fn validate_utf16_boundary(text: &str, offset: u32) -> Result<(), ValidationError> {
    if offset == 0 {
        return Ok(());
    }
    let mut current = 0_u32;
    for character in text.chars() {
        current = current.saturating_add(character.len_utf16() as u32);
        if current == offset {
            return Ok(());
        }
        if current > offset {
            break;
        }
    }
    Err(ValidationError::PhoneticRangeSplitsSurrogate { offset })
}

fn validate_phonetic_text(text: &str) -> Result<(), ValidationError> {
    if text.is_empty() {
        return Err(ValidationError::PhoneticTextEmpty);
    }
    if let Some(character) = text.chars().find(|character| !is_xml_10_char(*character)) {
        return Err(ValidationError::PhoneticTextInvalidCharacter { character });
    }
    Ok(())
}

fn is_xml_10_char(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

#[cfg(test)]
mod tests {
    use super::{
        PhoneticRun, PhoneticTextRange, validate_authoring_runs, validate_source_runs,
        validate_utf16_boundary,
    };
    use crate::ValidationError;

    #[test]
    fn authoring_ranges_use_utf16_boundaries() {
        let run =
            PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "え").expect("run");
        assert_eq!(validate_authoring_runs("😀A", &[run]), Ok(()));

        let split =
            PhoneticRun::new(PhoneticTextRange::new(0, 1).expect("range"), "え").expect("run");
        assert_eq!(
            validate_authoring_runs("😀A", &[split]),
            Err(ValidationError::PhoneticRangeSplitsSurrogate { offset: 1 })
        );
    }

    #[test]
    fn utf16_boundary_validation_rejects_past_end_offsets() {
        assert_eq!(validate_utf16_boundary("ab", 0), Ok(()));
        assert_eq!(validate_utf16_boundary("ab", 1), Ok(()));
        assert_eq!(validate_utf16_boundary("ab", 2), Ok(()));
        assert_eq!(
            validate_utf16_boundary("ab", 3),
            Err(ValidationError::PhoneticRangeSplitsSurrogate { offset: 3 })
        );
    }

    #[test]
    fn source_ranges_reject_an_end_past_the_utf16_length() {
        let run = PhoneticRun::new(PhoneticTextRange::new(1, 3).expect("range"), "x").expect("run");
        assert_eq!(
            validate_source_runs("ab", &[run]),
            Err(ValidationError::PhoneticRangeOutOfBounds {
                end: 3,
                base_utf16_len: 2,
            })
        );
    }
}
