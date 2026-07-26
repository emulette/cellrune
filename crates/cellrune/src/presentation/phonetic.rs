use std::ops::Range;

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

/// One phonetic run resolved into byte offsets over concrete base text.
///
/// Stored ranges are UTF-16 code-unit offsets because that is what XLSX records. Rust string
/// indexing is byte based, so a consumer needs the translation before it can slice the base text
/// or convert into its own annotation model. This type carries the translated range together with
/// the run it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPhoneticRun<'run, 'text> {
    // Held as two scalars rather than a `Range<usize>` so the type stays `Copy`.
    start_bytes: usize,
    end_bytes: usize,
    run: &'run PhoneticRun,
    base_text: &'text str,
}

impl<'run, 'text> ResolvedPhoneticRun<'run, 'text> {
    /// Returns the half-open byte range the run annotates within the base text it was resolved
    /// against.
    ///
    /// The range always falls on `char` boundaries of that text, so slicing with it never panics.
    pub const fn base_bytes(&self) -> Range<usize> {
        self.start_bytes..self.end_bytes
    }

    /// Returns the annotated slice of the base text this run was resolved against.
    ///
    /// The resolved value retains that exact base-text borrow, so a caller cannot accidentally
    /// pair the byte range with a different cell's string.
    pub fn base_slice(&self) -> &'text str {
        &self.base_text[self.base_bytes()]
    }

    /// Returns the displayed phonetic text.
    pub fn text(&self) -> &'run str {
        self.run.text()
    }

    /// Returns the source run, including its original UTF-16 range.
    pub const fn run(&self) -> &'run PhoneticRun {
        self.run
    }
}

/// Translates every run's UTF-16 range into byte offsets over `base_text`.
///
/// Runs are returned in source order. The base text is walked once regardless of run count.
pub(crate) fn resolve_runs<'run, 'text>(
    runs: &'run [PhoneticRun],
    base_text: &'text str,
) -> Result<Vec<ResolvedPhoneticRun<'run, 'text>>, ValidationError> {
    if runs.is_empty() {
        return Ok(Vec::new());
    }
    let (boundaries, base_utf16_len) = utf16_boundaries(base_text);
    runs.iter()
        .map(|run| {
            let range = run.base_range();
            // Checked before the boundary lookup so an out-of-range end reports the length it
            // exceeded rather than being reported as a split surrogate.
            if range.end_utf16() > base_utf16_len {
                return Err(ValidationError::PhoneticRangeOutOfBounds {
                    end: range.end_utf16(),
                    base_utf16_len,
                });
            }
            let start = byte_offset_at(&boundaries, range.start_utf16())?;
            let end = byte_offset_at(&boundaries, range.end_utf16())?;
            Ok(ResolvedPhoneticRun {
                start_bytes: start,
                end_bytes: end,
                run,
                base_text,
            })
        })
        .collect()
}

/// Returns `(utf16_offset, byte_offset)` pairs for every `char` boundary, and the total UTF-16
/// length. Both components of each pair increase monotonically, which is what lets the lookup
/// below binary search.
fn utf16_boundaries(text: &str) -> (Vec<(u32, usize)>, u32) {
    // Byte length, not `chars().count()`: every char occupies at least one byte, so this is an
    // upper bound on the entry count and reserves without a second pass over the text. Counting
    // chars exactly would make the "walked once" promise above false.
    let mut boundaries = Vec::with_capacity(text.len().saturating_add(1));
    boundaries.push((0_u32, 0_usize));
    let mut utf16 = 0_u32;
    for (byte_index, character) in text.char_indices() {
        utf16 = utf16.saturating_add(character.len_utf16() as u32);
        boundaries.push((utf16, byte_index + character.len_utf8()));
    }
    (boundaries, utf16)
}

fn byte_offset_at(boundaries: &[(u32, usize)], offset: u32) -> Result<usize, ValidationError> {
    boundaries
        .binary_search_by_key(&offset, |(utf16, _)| *utf16)
        .map(|index| boundaries[index].1)
        // A miss means the offset lands inside a surrogate pair: it is within the text but is not
        // a char boundary, so no byte offset represents it.
        .map_err(|_| ValidationError::PhoneticRangeSplitsSurrogate { offset })
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
        PhoneticRun, PhoneticTextRange, resolve_runs, validate_authoring_runs,
        validate_source_runs, validate_utf16_boundary,
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
    fn resolution_translates_utf16_ranges_into_byte_ranges() {
        // "😀" is one char, two UTF-16 code units, four UTF-8 bytes: the offsets diverge in both
        // directions, which is the whole reason this API exists.
        let base = "😀ab";
        let runs = vec![
            PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "え").expect("run"),
            PhoneticRun::new(PhoneticTextRange::new(2, 4).expect("range"), "び").expect("run"),
        ];
        let resolved = resolve_runs(&runs, base).expect("resolves");
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].base_bytes(), 0..4);
        assert_eq!(resolved[0].base_slice(), "😀");
        assert_eq!(resolved[0].text(), "え");
        assert_eq!(resolved[1].base_bytes(), 4..6);
        assert_eq!(resolved[1].base_slice(), "ab");
        assert_eq!(resolved[1].run().base_range().start_utf16(), 2);
    }

    #[test]
    fn resolution_rejects_a_boundary_inside_a_surrogate_pair() {
        let runs = vec![
            PhoneticRun::new(PhoneticTextRange::new(0, 1).expect("range"), "え").expect("run"),
        ];
        assert_eq!(
            resolve_runs(&runs, "😀A"),
            Err(ValidationError::PhoneticRangeSplitsSurrogate { offset: 1 })
        );
    }

    #[test]
    fn resolution_rejects_an_end_past_the_utf16_length() {
        let runs =
            vec![PhoneticRun::new(PhoneticTextRange::new(1, 3).expect("range"), "x").expect("run")];
        assert_eq!(
            resolve_runs(&runs, "ab"),
            Err(ValidationError::PhoneticRangeOutOfBounds {
                end: 3,
                base_utf16_len: 2,
            })
        );
    }

    #[test]
    fn resolution_reports_out_of_bounds_before_a_split_for_the_same_run() {
        // The end is both past the length and mid-surrogate. Reporting the length it exceeded is
        // the actionable message; "splits a surrogate" would send the caller looking at encoding.
        let runs =
            vec![PhoneticRun::new(PhoneticTextRange::new(0, 5).expect("range"), "x").expect("run")];
        assert_eq!(
            resolve_runs(&runs, "😀"),
            Err(ValidationError::PhoneticRangeOutOfBounds {
                end: 5,
                base_utf16_len: 2,
            })
        );
    }

    #[test]
    fn resolution_accepts_the_full_span_and_an_empty_run_list() {
        let runs = vec![
            PhoneticRun::new(PhoneticTextRange::new(0, 2).expect("range"), "ぜ").expect("run"),
        ];
        let resolved = resolve_runs(&runs, "😀").expect("resolves");
        assert_eq!(resolved[0].base_bytes(), 0..4);
        assert_eq!(resolve_runs(&[], "anything"), Ok(Vec::new()));
    }

    #[test]
    fn resolution_handles_unordered_and_overlapping_source_runs() {
        // The reader tolerates these (validate_source_runs only reports them), so resolution must
        // not assume sorted input.
        let runs = vec![
            PhoneticRun::new(PhoneticTextRange::new(2, 4).expect("range"), "b").expect("run"),
            PhoneticRun::new(PhoneticTextRange::new(0, 3).expect("range"), "a").expect("run"),
        ];
        let resolved = resolve_runs(&runs, "abcd").expect("resolves");
        assert_eq!(resolved[0].base_bytes(), 2..4);
        assert_eq!(resolved[1].base_bytes(), 0..3);
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
