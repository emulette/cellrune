//! Decodes OOXML `ST_Xstring` escapes in Excel-saved string observations.
//!
//! Observations keep the text exactly as Excel stored it, where a character XML 1.0 cannot carry
//! is written as the UTF-16 code unit `_xHHHH_` and a literal escape pattern is protected by an
//! escaped underscore, `_x005F_`. Decoding left to right recovers the cell text CellRune reports.

const ESCAPE_LENGTH: usize = "_xHHHH_".len();

pub(super) fn decode_xstring(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('_') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = match code_unit(rest) {
            Some(high @ 0xD800..=0xDBFF) => code_unit(&rest[ESCAPE_LENGTH..])
                .filter(|low| (0xDC00..=0xDFFF).contains(low))
                .and_then(|low| char::decode_utf16([high, low]).next()?.ok())
                .map(|character| (character, 2 * ESCAPE_LENGTH)),
            Some(unit) => {
                char::from_u32(u32::from(unit)).map(|character| (character, ESCAPE_LENGTH))
            }
            None => None,
        };
        if let Some((character, length)) = decoded {
            output.push(character);
            rest = &rest[length..];
        } else {
            output.push('_');
            rest = &rest[1..];
        }
    }
    output.push_str(rest);
    output
}

fn code_unit(text: &str) -> Option<u16> {
    let bytes = text.as_bytes().get(..ESCAPE_LENGTH)?;
    let digits = &bytes[2..6];
    if bytes[0] != b'_' || bytes[1] != b'x' || bytes[6] != b'_' {
        return None;
    }
    if !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::decode_xstring;

    #[test]
    fn decodes_excel_escapes_with_the_st_xstring_rule() {
        assert_eq!(decode_xstring("_x0002_"), "\u{2}");
        assert_eq!(decode_xstring("a_x000D__x000a_b"), "a\r\nb");
        assert_eq!(decode_xstring("_x005F_x0041_"), "_x0041_");
        assert_eq!(decode_xstring("_xD83D__xDE00_"), "\u{1F600}");
        for literal in ["plain", "_xD83D_", "_x+041_", "_x004_", "_X0041_", "_x0041"] {
            assert_eq!(decode_xstring(literal), literal);
        }
    }
}
