//! The number-format codes `CELL("format")` reports.

use crate::NumberFormat;

/// The `CELL("format")` codes of the number formats Microsoft's CELL documentation lists, keyed
/// by the format code with ASCII letters lowercased. `yyyy-mm-dd` is `G` in Excel 16.114 for Mac.
const DOCUMENTED_CODES: [(&str, &str); 31] = [
    ("general", "G"),
    ("0", "F0"),
    ("#,##0", ",0"),
    ("0.00", "F2"),
    ("#,##0.00", ",2"),
    ("$#,##0_);($#,##0)", "C0"),
    ("$#,##0_);[red]($#,##0)", "C0-"),
    ("$#,##0.00_);($#,##0.00)", "C2"),
    ("$#,##0.00_);[red]($#,##0.00)", "C2-"),
    ("0%", "P0"),
    ("0.00%", "P2"),
    ("0.00e+00", "S2"),
    ("# ?/?", "G"),
    ("# ??/??", "G"),
    ("m/d/yy", "D4"),
    ("m/d/yy h:mm", "D4"),
    ("mm/dd/yy", "D4"),
    ("mm-dd-yy", "D4"),
    ("d-mmm-yy", "D1"),
    ("dd-mmm-yy", "D1"),
    ("d-mmm", "D2"),
    ("dd-mmm", "D2"),
    ("mmm-yy", "D3"),
    ("mm/dd", "D5"),
    ("h:mm am/pm", "D7"),
    ("h:mm:ss am/pm", "D6"),
    ("h:mm", "D9"),
    ("h:mm:ss", "D8"),
    ("yyyy-mm-dd", "G"),
    ("m/d/yyyy", "D4"),
    ("m/d/yyyy h:mm", "D4"),
];

/// Returns the `CELL("format")` code of `format`, or `None` for a format Microsoft does not
/// document a code for.
///
/// Built-in formats 0 through 22 are the documented ones; 5 through 8 are the United States
/// currency formats Excel stores by ID. A custom code is matched after its `"$"` literals and
/// backslash escapes are read as the characters they show, so `"$"#,##0_);\("$"#,##0\)` is
/// `C0`.
pub(super) fn format_code(format: &NumberFormat) -> Option<&'static str> {
    let built_in = match format.id() {
        0 | 12 | 13 => Some("G"),
        1 => Some("F0"),
        2 => Some("F2"),
        3 => Some(",0"),
        4 => Some(",2"),
        5 => Some("C0"),
        6 => Some("C0-"),
        7 => Some("C2"),
        8 => Some("C2-"),
        9 => Some("P0"),
        10 => Some("P2"),
        11 => Some("S2"),
        14 | 22 => Some("D4"),
        15 => Some("D1"),
        16 => Some("D2"),
        17 => Some("D3"),
        18 => Some("D7"),
        19 => Some("D6"),
        20 => Some("D9"),
        21 => Some("D8"),
        _ => None,
    };
    if format.id() < 164 {
        return built_in;
    }
    let shown = shown_code(format.code()?);
    DOCUMENTED_CODES
        .iter()
        .find(|(code, _)| *code == shown)
        .map(|(_, info)| *info)
}

/// The code as its characters show, with ASCII letters lowercased.
fn shown_code(code: &str) -> String {
    let unquoted = code.replace("\"$\"", "$");
    let mut shown = String::with_capacity(unquoted.len());
    let mut characters = unquoted.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => shown.extend(characters.next()),
            _ => shown.push(character.to_ascii_lowercase()),
        }
    }
    shown
}
