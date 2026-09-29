//! OOXML `ST_Xstring` escapes for cell text.
//!
//! XML 1.0 cannot carry most C0 control characters, so SpreadsheetML stores a UTF-16 code unit as
//! `_xHHHH_`. A literal `_xHHHH_` in the text is written with its leading underscore escaped as
//! `_x005F_`, which a left-to-right decoder turns back into the literal pattern.

use std::borrow::Cow;

use super::xml::is_xml_10_character;

const ESCAPE_LENGTH: usize = "_xHHHH_".len();
const UNDERSCORE_ESCAPE: &str = "_x005F_";

enum CodeUnit {
    Value(u16),
    Incomplete,
    Absent,
}

enum Scan {
    Character(char, usize),
    Literal,
    NeedMore,
}

/// Incrementally decodes the character data of one `ST_Xstring` element.
///
/// Character data may arrive in several chunks (text, entity references, CDATA), so an escape can
/// straddle a chunk boundary. The decoder holds back only the short tail that could still begin
/// an escape, which keeps callers' incremental byte budgets exact.
#[derive(Debug, Default)]
pub(super) struct XstringDecoder {
    pending: String,
}

impl XstringDecoder {
    /// Appends raw character data and returns the decoded text that later chunks cannot change.
    pub(super) fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let (decoded, consumed) = decode_prefix(&self.pending, false);
        self.pending.drain(..consumed);
        decoded
    }

    /// Ends the element and returns the decoded remainder.
    pub(super) fn finish(&mut self) -> String {
        let (decoded, _) = decode_prefix(&self.pending, true);
        self.pending.clear();
        decoded
    }
}

/// Decodes the complete character data of one `ST_Xstring` value.
pub(super) fn decode_xstring(text: &str) -> Cow<'_, str> {
    if !text.contains("_x") {
        return Cow::Borrowed(text);
    }
    Cow::Owned(decode_prefix(text, true).0)
}

/// Encodes text so that it is valid XML 1.0 character data and decodes back to `text`.
pub(super) fn encode_xstring(text: &str) -> Cow<'_, str> {
    let needs_encoding = text.char_indices().any(|(index, character)| {
        !is_xml_10_character(character)
            || (character == '_' && matches!(code_unit_at(text, index), CodeUnit::Value(_)))
    });
    if !needs_encoding {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len() + ESCAPE_LENGTH);
    for (index, character) in text.char_indices() {
        if !is_xml_10_character(character) {
            output.push_str(&format!("_x{:04X}_", u32::from(character)));
        } else if character == '_' && matches!(code_unit_at(text, index), CodeUnit::Value(_)) {
            output.push_str(UNDERSCORE_ESCAPE);
        } else {
            output.push(character);
        }
    }
    Cow::Owned(output)
}

/// Decodes `text` up to the first position whose meaning depends on data not yet seen, and
/// returns the decoded text with the number of bytes consumed.
fn decode_prefix(text: &str, at_end: bool) -> (String, usize) {
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    let mut search = 0;
    while let Some(offset) = text[search..].find('_') {
        let start = search + offset;
        match scan_escape(text, start, at_end) {
            Scan::Character(character, length) => {
                output.push_str(&text[copied..start]);
                output.push(character);
                copied = start + length;
                search = copied;
            }
            Scan::Literal => search = start + 1,
            Scan::NeedMore => {
                output.push_str(&text[copied..start]);
                return (output, start);
            }
        }
    }
    output.push_str(&text[copied..]);
    (output, text.len())
}

fn scan_escape(text: &str, start: usize, at_end: bool) -> Scan {
    match code_unit_at(text, start) {
        CodeUnit::Absent => Scan::Literal,
        CodeUnit::Incomplete if at_end => Scan::Literal,
        CodeUnit::Incomplete => Scan::NeedMore,
        CodeUnit::Value(high @ 0xD800..=0xDBFF) => {
            match code_unit_at(text, start + ESCAPE_LENGTH) {
                CodeUnit::Value(low @ 0xDC00..=0xDFFF) => char::decode_utf16([high, low])
                    .next()
                    .and_then(Result::ok)
                    .map_or(Scan::Literal, |character| {
                        Scan::Character(character, 2 * ESCAPE_LENGTH)
                    }),
                CodeUnit::Incomplete if !at_end => Scan::NeedMore,
                _ => Scan::Literal,
            }
        }
        CodeUnit::Value(unit) => char::from_u32(u32::from(unit))
            .map_or(Scan::Literal, |character| {
                Scan::Character(character, ESCAPE_LENGTH)
            }),
    }
}

/// Reads an `_xHHHH_` code unit starting at byte `start`; text that ends before the pattern can be
/// ruled out, including text that ends exactly at `start`, is incomplete.
fn code_unit_at(text: &str, start: usize) -> CodeUnit {
    let Some(bytes) = text.as_bytes().get(start..) else {
        return CodeUnit::Absent;
    };
    let available = bytes.len().min(ESCAPE_LENGTH);
    let matches_pattern = bytes[..available]
        .iter()
        .enumerate()
        .all(|(position, byte)| match position {
            0 | 6 => *byte == b'_',
            1 => *byte == b'x',
            _ => byte.is_ascii_hexdigit(),
        });
    if !matches_pattern {
        return CodeUnit::Absent;
    }
    if available < ESCAPE_LENGTH {
        return CodeUnit::Incomplete;
    }
    std::str::from_utf8(&bytes[2..6])
        .ok()
        .and_then(|digits| u16::from_str_radix(digits, 16).ok())
        .map_or(CodeUnit::Absent, CodeUnit::Value)
}

#[cfg(test)]
mod tests {
    use super::{XstringDecoder, decode_xstring, encode_xstring, is_xml_10_character};

    #[test]
    fn decodes_code_units_surrogate_pairs_and_escaped_underscores() {
        assert_eq!(decode_xstring("a_x0002_b"), "a\u{2}b");
        assert_eq!(decode_xstring("_x000d__x000A_"), "\r\n");
        assert_eq!(decode_xstring("_xD83D__xDE00_"), "\u{1F600}");
        assert_eq!(decode_xstring("_x005F_x0041_"), "_x0041_");
        assert_eq!(decode_xstring("_x005F_"), "_");
        for literal in [
            "_xD83D_", "_xDE00_", "_x004_", "_x00G1_", "x0041_", "_X0041_", "_x0041",
        ] {
            assert_eq!(decode_xstring(literal), literal);
        }
    }

    #[test]
    fn streaming_decoder_matches_whole_text_decoding_at_every_split() {
        let raw = "a_x0002__x005F_x0041__xD83D__xDE00_b_x00";
        let expected = decode_xstring(raw).into_owned();
        for split in 0..=raw.len() {
            let mut decoder = XstringDecoder::default();
            let mut decoded = decoder.push(&raw[..split]);
            decoded.push_str(&decoder.push(&raw[split..]));
            decoded.push_str(&decoder.finish());
            assert_eq!(decoded, expected, "split at {split}");
        }
    }

    #[test]
    fn encoding_round_trips_forbidden_characters_and_literal_escapes() {
        assert_eq!(encode_xstring("plain_text"), "plain_text");
        assert_eq!(encode_xstring("a\u{2}b"), "a_x0002_b");
        assert_eq!(encode_xstring("\u{FFFE}"), "_xFFFE_");
        assert_eq!(encode_xstring("_x0041_"), "_x005F_x0041_");
        for text in [
            "a\u{0}\u{8}\u{B}\u{C}\u{1F}b",
            "_x0041_",
            "_x005F_",
            "_x0041__x0042_",
            "__\u{1}x0041_",
            "\u{1}_x0041_\u{FFFF}",
            "tab\tnew\nline\r",
        ] {
            let encoded = encode_xstring(text);
            assert!(encoded.chars().all(is_xml_10_character), "{encoded:?}");
            assert_eq!(decode_xstring(&encoded), text, "{encoded:?}");
        }
    }
}
