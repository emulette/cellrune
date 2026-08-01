use super::regex_options::RegexNewline;

pub(super) fn is_plain_literal(pattern: &str) -> bool {
    !pattern.bytes().any(|byte| {
        matches!(
            byte,
            b'\\'
                | b'.'
                | b'^'
                | b'$'
                | b'|'
                | b'?'
                | b'*'
                | b'+'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegexExtendedMode {
    Off,
    Extended,
    ExtendedMore,
}

impl RegexExtendedMode {
    const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineExtendedMode {
    Scoped {
        mode: RegexExtendedMode,
        remainder: usize,
    },
    Unscoped {
        mode: RegexExtendedMode,
        remainder: usize,
    },
}

fn inline_extended_mode(
    bytes: &[u8],
    opening: usize,
    inherited: RegexExtendedMode,
) -> Option<InlineExtendedMode> {
    let mut cursor = opening.checked_add(2)?;
    let mut mode = inherited;
    let mut disabling = false;
    let mut recognized = false;
    let mut sets_extended_more = false;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'-' if !disabling => disabling = true,
            b'^' if !recognized && !disabling => {
                mode = RegexExtendedMode::Off;
                recognized = true;
            }
            b'x' => {
                let doubled = bytes.get(cursor + 1) == Some(&b'x');
                if doubled {
                    cursor += 1;
                }
                mode = if disabling {
                    RegexExtendedMode::Off
                } else if doubled {
                    sets_extended_more = true;
                    RegexExtendedMode::ExtendedMore
                } else if sets_extended_more {
                    RegexExtendedMode::ExtendedMore
                } else {
                    RegexExtendedMode::Extended
                };
                recognized = true;
            }
            b'a' => {
                recognized = true;
                if cursor + 1 < bytes.len()
                    && matches!(bytes[cursor + 1], b'D' | b'P' | b'S' | b'T' | b'W')
                {
                    cursor += 1;
                }
            }
            b'i' | b'J' | b'm' | b'n' | b'r' | b's' | b'U' => {
                recognized = true;
            }
            b':' if recognized => {
                return Some(InlineExtendedMode::Scoped {
                    mode,
                    remainder: cursor + 1,
                });
            }
            b')' if recognized => {
                return Some(InlineExtendedMode::Unscoped {
                    mode,
                    remainder: cursor + 1,
                });
            }
            _ => return None,
        }
        cursor += 1;
    }
    None
}

fn verb_argument_end(bytes: &[u8], opening: usize) -> Option<usize> {
    const LITERAL_ARGUMENT_PREFIXES: [&[u8]; 9] = [
        b"(*:",
        b"(*MARK:",
        b"(*ACCEPT:",
        b"(*F:",
        b"(*FAIL:",
        b"(*COMMIT:",
        b"(*PRUNE:",
        b"(*SKIP:",
        b"(*THEN:",
    ];
    LITERAL_ARGUMENT_PREFIXES
        .iter()
        .any(|prefix| bytes[opening..].starts_with(prefix))
        .then(|| {
            bytes[opening..]
                .iter()
                .position(|byte| *byte == b')')
                .map(|offset| opening + offset + 1)
        })
        .flatten()
}

fn string_callout_end(bytes: &[u8], opening: usize) -> Option<usize> {
    if !bytes[opening..].starts_with(b"(?C") {
        return None;
    }
    let delimiter = *bytes.get(opening + 3)?;
    let closing = match delimiter {
        b'`' | b'\'' | b'"' | b'^' | b'%' | b'#' | b'$' => delimiter,
        b'{' => b'}',
        _ => return None,
    };
    let mut cursor = opening + 4;
    while cursor < bytes.len() {
        if bytes[cursor] != closing {
            cursor += 1;
            continue;
        }
        if bytes.get(cursor + 1) == Some(&closing) {
            cursor += 2;
            continue;
        }
        return (bytes.get(cursor + 1) == Some(&b')')).then_some(cursor + 2);
    }
    None
}

fn is_root_recursion_at(bytes: &[u8], opening: usize) -> bool {
    if bytes[opening..].starts_with(b"(?R)") {
        return true;
    }
    if bytes[opening..].starts_with(b"(?") {
        let mut cursor = opening + 2;
        while bytes.get(cursor) == Some(&b'0') {
            cursor += 1;
        }
        if cursor > opening + 2 && bytes.get(cursor) == Some(&b')') {
            return true;
        }
    }
    for (prefix, closing) in [(br"\g<".as_slice(), b'>'), (br"\g'".as_slice(), b'\'')] {
        if bytes[opening..].starts_with(prefix) {
            let mut cursor = opening + prefix.len();
            while bytes.get(cursor) == Some(&b'0') {
                cursor += 1;
            }
            if cursor > opening + prefix.len() && bytes.get(cursor) == Some(&closing) {
                return true;
            }
        }
    }
    false
}

fn skip_ignored_class_prefix(bytes: &[u8], mut cursor: usize, mode: RegexExtendedMode) -> usize {
    loop {
        if mode == RegexExtendedMode::ExtendedMore
            && matches!(bytes.get(cursor), Some(b' ' | b'\t'))
        {
            cursor += 1;
        } else if bytes[cursor..].starts_with(br"\Q\E") {
            cursor += 4;
        } else if bytes[cursor..].starts_with(br"\E") {
            cursor += 2;
        } else {
            return cursor;
        }
    }
}

fn character_class_end(bytes: &[u8], opening: usize, mode: RegexExtendedMode) -> usize {
    let mut cursor = opening + 1;
    cursor = skip_ignored_class_prefix(bytes, cursor, mode);
    if bytes.get(cursor) == Some(&b'^') {
        cursor += 1;
        cursor = skip_ignored_class_prefix(bytes, cursor, mode);
    }
    if bytes.get(cursor) == Some(&b']') {
        cursor += 1;
    }
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            if bytes[cursor..].starts_with(br"\Q") {
                cursor += 2;
                while cursor < bytes.len() && !bytes[cursor..].starts_with(br"\E") {
                    cursor += 1;
                }
                cursor = cursor.saturating_add(2).min(bytes.len());
            } else {
                cursor = cursor.saturating_add(2).min(bytes.len());
            }
            continue;
        }
        if bytes[cursor] == b'[' && matches!(bytes.get(cursor + 1), Some(b':' | b'.' | b'=')) {
            let delimiter = bytes[cursor + 1];
            let mut nested_end = cursor + 2;
            while nested_end < bytes.len() && bytes[nested_end] != b']' {
                nested_end += 1;
            }
            if nested_end < bytes.len()
                && nested_end > cursor + 2
                && bytes[nested_end - 1] == delimiter
            {
                cursor = nested_end + 1;
            } else {
                cursor += 1;
            }
            continue;
        }
        if bytes[cursor] == b']' {
            return cursor + 1;
        }
        cursor += 1;
    }
    bytes.len()
}

fn extended_comment_end(bytes: &[u8], opening: usize, newline: RegexNewline) -> usize {
    let mut cursor = opening + 1;
    while cursor < bytes.len() {
        let is_newline = match newline {
            RegexNewline::Cr => bytes[cursor] == b'\r',
            RegexNewline::Lf => bytes[cursor] == b'\n',
            RegexNewline::CrLf => bytes[cursor..].starts_with(b"\r\n"),
            RegexNewline::Any => {
                matches!(bytes[cursor], b'\r' | b'\n' | 0x0b | 0x0c)
                    || bytes[cursor..].starts_with("\u{0085}".as_bytes())
                    || bytes[cursor..].starts_with("\u{2028}".as_bytes())
                    || bytes[cursor..].starts_with("\u{2029}".as_bytes())
            }
            RegexNewline::AnyCrLf => matches!(bytes[cursor], b'\r' | b'\n'),
            RegexNewline::Nul => bytes[cursor] == 0,
        };
        if is_newline {
            return cursor;
        }
        cursor += 1;
    }
    bytes.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PatternAnalysis {
    pub(super) has_root_recursion: bool,
    pub(super) ends_in_extended_comment: bool,
}

pub(super) fn analyze_pattern(pattern: &str, newline: RegexNewline) -> PatternAnalysis {
    let bytes = pattern.as_bytes();
    let mut index = 0;
    let mut in_quoted_literal = false;
    let mut extended_modes = vec![RegexExtendedMode::Off];
    let mut has_root_recursion = false;
    let mut ends_in_extended_comment = false;
    while index < bytes.len() {
        if in_quoted_literal {
            if bytes[index..].starts_with(br"\E") {
                in_quoted_literal = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'\\' {
            if bytes[index..].starts_with(br"\Q") {
                in_quoted_literal = true;
            } else if is_root_recursion_at(bytes, index) {
                has_root_recursion = true;
            }
            index = index.saturating_add(2);
            continue;
        }
        if bytes[index] == b'[' {
            index = character_class_end(
                bytes,
                index,
                extended_modes
                    .last()
                    .copied()
                    .unwrap_or(RegexExtendedMode::Off),
            );
            continue;
        }
        if extended_modes
            .last()
            .copied()
            .unwrap_or(RegexExtendedMode::Off)
            .is_enabled()
            && bytes[index] == b'#'
        {
            index = extended_comment_end(bytes, index, newline);
            ends_in_extended_comment = index == bytes.len();
            continue;
        }
        if bytes[index..].starts_with(b"(?#") {
            index += 3;
            while index < bytes.len() && bytes[index] != b')' {
                index += 1;
            }
            index = index.saturating_add(1);
            continue;
        }
        if let Some(remainder) = verb_argument_end(bytes, index) {
            index = remainder;
            continue;
        }
        if let Some(remainder) = string_callout_end(bytes, index) {
            index = remainder;
            continue;
        }
        if is_root_recursion_at(bytes, index) {
            has_root_recursion = true;
        }
        if bytes[index] == b'(' {
            let inherited = extended_modes
                .last()
                .copied()
                .unwrap_or(RegexExtendedMode::Off);
            match inline_extended_mode(bytes, index, inherited) {
                Some(InlineExtendedMode::Scoped { mode, remainder }) => {
                    extended_modes.push(mode);
                    index = remainder;
                }
                Some(InlineExtendedMode::Unscoped { mode, remainder }) => {
                    if let Some(current) = extended_modes.last_mut() {
                        *current = mode;
                    }
                    index = remainder;
                }
                None => {
                    extended_modes.push(inherited);
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b')' {
            if extended_modes.len() > 1 {
                extended_modes.pop();
            }
            index += 1;
            continue;
        }
        index += 1;
    }
    PatternAnalysis {
        has_root_recursion,
        ends_in_extended_comment,
    }
}

#[cfg(test)]
mod tests {
    use super::super::regex_options::start_options;

    #[test]
    fn root_recursion_detection_ignores_literal_contexts() {
        let cases = [
            (r"|a(?R)", true),
            (r"|a(?0)", true),
            (r"|a(?000)", true),
            (r"|a\g<0>", true),
            (r"|a\g<00>", true),
            (r"|a\g'0'", true),
            (r"|a\g'000'", true),
            (r"\Q(?R)\E", false),
            (r"[(?R)]", false),
            (r"[\g<0>]", false),
            (r"[[:alpha:](?R)]", false),
            (r"[[:alpha:]](?R)", true),
            (r"|a[[:foo]](?R)", true),
            (r"|a[[:foo]](?R):]", true),
            (r"[\Q](?R)\E]", false),
            (r"[](?R)]|", false),
            (r"[^](?R)]|", false),
            (r"(?xx:[  ](?R)]|)", false),
            (r"(?xxx:[  ](?R)]|)", false),
            (r"(?xx)(?x:[  ](?R)]|)", true),
            (r"[\Q\E](?R)]|", false),
            (r"[^\E](?R)]|", false),
            (r"(?# (?R))a", false),
            (r"(?#\)|a(?R)", true),
            ("(?x)# (?R)\n(?:|a)", false),
            ("(?x:# (?0)\n(?:|a))", false),
            ("(?x)# comment\n(?-x:a(?R))", true),
            ("(?x)# comment\n(?-aTx:#(?R)\n)", true),
            (r"(*MARK:(?R)", false),
            (r"(*MARK:(?0)", false),
            (r"(*ACCEPT:(?R)", false),
            (r"(*F:(?0)", false),
            (r"(*FAIL:(?R)", false),
            (r"(*SKIP:(?R)", false),
            (r#"(?C"(?R)")"#, false),
            (r"(?C{(?0)})", false),
            (r#"(?C"quoted""(?R)")"#, false),
        ];
        for (pattern, expected) in cases {
            let options = start_options(pattern);
            assert_eq!(
                super::analyze_pattern(pattern, options.newline).has_root_recursion,
                expected,
                "unexpected root-recursion classification for {pattern:?}",
            );
        }
    }

    #[test]
    fn extended_comments_follow_the_selected_pcre2_newline_convention() {
        let cases = [
            ("(?x)# comment\r(?R)|", false),
            ("(*CR)(?x)# comment\r(?R)|", true),
            ("(*CRLF)(?x)# comment\r(?R)|", false),
            ("(*CRLF)(?x)# comment\r\n(?R)|", true),
            ("(*ANYCRLF)(?x)# comment\r(?R)|", true),
            ("(*ANYCRLF)(?x)# comment\u{000b}(?R)|", false),
            ("(*ANY)(?x)# comment\u{000b}(?R)|", true),
            ("(*ANY)(?x)# comment\u{0085}(?R)|", true),
            ("(*ANY)(?x)# comment\u{2028}(?R)|", true),
            ("(*ANY)(?x)# comment\u{2029}(?R)|", true),
            ("(*NUL)(?x)# comment\0(?R)|", true),
        ];
        for (pattern, expected) in cases {
            let options = start_options(pattern);
            assert_eq!(
                super::analyze_pattern(pattern, options.newline).has_root_recursion,
                expected,
                "unexpected newline handling for {pattern:?}",
            );
        }
    }

    #[test]
    fn plain_literal_detection_excludes_regex_operators() {
        assert!(super::is_plain_literal("a thousand literal bytes 123"));
        assert!(super::is_plain_literal("한글 literal"));
        assert!(!super::is_plain_literal(r"a\.b"));
        assert!(!super::is_plain_literal("a.b"));
        assert!(!super::is_plain_literal("(?:abc)"));
    }
}
