#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegexNewline {
    Cr,
    Lf,
    CrLf,
    Any,
    AnyCrLf,
    Nul,
}

impl RegexNewline {
    pub(super) const fn comment_terminator(self) -> &'static str {
        match self {
            Self::Cr => "\r",
            Self::Lf | Self::Any | Self::AnyCrLf => "\n",
            Self::CrLf => "\r\n",
            Self::Nul => "\0",
        }
    }
}

pub(super) struct StartOptions<'a> {
    pub(super) preserved: String,
    pub(super) remainder: &'a str,
    pub(super) match_limit: Option<u64>,
    pub(super) depth_limit: Option<u64>,
    pub(super) heap_limit: Option<u64>,
    pub(super) newline: RegexNewline,
}

pub(super) fn start_options(mut pattern: &str) -> StartOptions<'_> {
    let mut preserved = String::new();
    let mut match_limit: Option<u64> = None;
    let mut depth_limit: Option<u64> = None;
    let mut heap_limit: Option<u64> = None;
    let mut newline = RegexNewline::Lf;
    while let Some(tail) = pattern.strip_prefix("(*") {
        let Some(end) = tail.find(')') else {
            break;
        };
        let option = &tail[..end];
        let recognized = if is_preserved_start_option(option) {
            preserved.push_str(&pattern[..end + 3]);
            newline = match option {
                "CR" => RegexNewline::Cr,
                "LF" => RegexNewline::Lf,
                "CRLF" => RegexNewline::CrLf,
                "ANY" => RegexNewline::Any,
                "ANYCRLF" => RegexNewline::AnyCrLf,
                "NUL" => RegexNewline::Nul,
                _ => newline,
            };
            true
        } else if let Some(value) = numeric_start_option(option, "LIMIT_MATCH=") {
            match_limit = Some(match_limit.map_or(value, |current| current.min(value)));
            true
        } else if let Some(value) = numeric_start_option(option, "LIMIT_DEPTH=")
            .or_else(|| numeric_start_option(option, "LIMIT_RECURSION="))
        {
            depth_limit = Some(depth_limit.map_or(value, |current| current.min(value)));
            true
        } else if let Some(value) = numeric_start_option(option, "LIMIT_HEAP=") {
            heap_limit = Some(heap_limit.map_or(value, |current| current.min(value)));
            true
        } else {
            false
        };
        if !recognized {
            break;
        }
        pattern = &tail[end + 1..];
    }
    StartOptions {
        preserved,
        remainder: pattern,
        match_limit,
        depth_limit,
        heap_limit,
        newline,
    }
}

fn numeric_start_option(option: &str, prefix: &str) -> Option<u64> {
    let digits = option.strip_prefix(prefix)?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse::<u64>().ok())
        .flatten()
        .filter(|value| *value <= u64::from(u32::MAX))
}

fn is_preserved_start_option(option: &str) -> bool {
    matches!(
        option,
        "UTF"
            | "UTF8"
            | "UCP"
            | "NOTEMPTY"
            | "NOTEMPTY_ATSTART"
            | "NO_AUTO_POSSESS"
            | "NO_DOTSTAR_ANCHOR"
            | "NO_JIT"
            | "NO_START_OPT"
            | "CASELESS_RESTRICT"
            | "TURKISH_CASING"
            | "CR"
            | "LF"
            | "CRLF"
            | "ANY"
            | "NUL"
            | "ANYCRLF"
            | "BSR_ANYCRLF"
            | "BSR_UNICODE"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn start_options_are_preserved_ahead_of_injected_limits() {
        let options = super::start_options(
            r"(*UTF)(*NO_AUTO_POSSESS)(*LIMIT_MATCH=7)(*LIMIT_DEPTH=8)(*LIMIT_HEAP=9)a",
        );
        assert_eq!(options.preserved, "(*UTF)(*NO_AUTO_POSSESS)");
        assert_eq!(options.remainder, "a");
        assert_eq!(options.match_limit, Some(7));
        assert_eq!(options.depth_limit, Some(8));
        assert_eq!(options.heap_limit, Some(9));
    }
}
