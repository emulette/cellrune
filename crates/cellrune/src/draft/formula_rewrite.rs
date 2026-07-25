pub(super) fn rename_sheet_references(formula: &str, old_name: &str, new_name: &str) -> String {
    let mut output = String::with_capacity(formula.len());
    let bytes = formula.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let end = quoted_end(formula, index, b'"').unwrap_or(formula.len());
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        if bytes[index] == b'[' {
            if let Some(end) = external_reference_end(formula, index) {
                output.push_str(&formula[index..end]);
                index = end;
                continue;
            }
            let end = bytes[index + 1..]
                .iter()
                .position(|byte| *byte == b']')
                .map_or(formula.len(), |offset| index + offset + 2);
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        let character = formula[index..].chars().next();
        let previous = formula[..index].chars().next_back();
        let can_start_prefix = character == Some('\'')
            || (character.is_some_and(is_sheet_token_character)
                && previous.is_none_or(|value| !is_sheet_token_character(value)));
        if can_start_prefix && let Some(prefix) = parse_sheet_prefix(formula, index) {
            let rewritten = match prefix.second.as_ref() {
                Some(second) => rewrite_sheet_range(
                    &prefix.first.name,
                    &second.name,
                    old_name,
                    new_name,
                    prefix.first.quoted || second.quoted,
                ),
                None => rewrite_single_sheet(&prefix.first, old_name, new_name),
            };
            if let Some(rewritten) = rewritten {
                output.push_str(&rewritten);
                output.push('!');
            } else {
                output.push_str(&formula[index..prefix.end]);
            }
            index = prefix.end;
            continue;
        }
        if bytes[index] == b'\'' {
            let end = quoted_end(formula, index, b'\'').unwrap_or(formula.len());
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        if let Some(character) = formula[index..].chars().next() {
            output.push(character);
            index += character.len_utf8();
        } else {
            break;
        }
    }
    output
}

struct SheetEndpoint {
    name: String,
    end: usize,
    quoted: bool,
}

struct SheetPrefix {
    first: SheetEndpoint,
    second: Option<SheetEndpoint>,
    end: usize,
}

fn parse_sheet_prefix(formula: &str, start: usize) -> Option<SheetPrefix> {
    let mut first = parse_sheet_endpoint(formula, start)?;
    let quoted_range = first
        .quoted
        .then(|| {
            first
                .name
                .split_once(':')
                .map(|(left, right)| (left.to_owned(), right.to_owned()))
        })
        .flatten();
    if let Some((first_name, second_name)) = quoted_range {
        let end = first.end;
        let second = SheetEndpoint {
            name: second_name,
            end,
            quoted: true,
        };
        first.name = first_name;
        return (formula.as_bytes().get(end) == Some(&b'!')).then_some(SheetPrefix {
            first,
            second: Some(second),
            end: end + 1,
        });
    }
    let (second, end) = if formula.as_bytes().get(first.end) == Some(&b':') {
        let second = parse_sheet_endpoint(formula, first.end + 1)?;
        (Some(second), None)
    } else {
        (None, Some(first.end))
    };
    let bang = end.unwrap_or_else(|| second.as_ref().expect("second endpoint").end);
    (formula.as_bytes().get(bang) == Some(&b'!')).then_some(SheetPrefix {
        first,
        second,
        end: bang + 1,
    })
}

fn parse_sheet_endpoint(formula: &str, start: usize) -> Option<SheetEndpoint> {
    if formula.as_bytes().get(start) == Some(&b'\'') {
        let end = quoted_end(formula, start, b'\'')?;
        return Some(SheetEndpoint {
            name: formula[start + 1..end - 1].replace("''", "'"),
            end,
            quoted: true,
        });
    }
    let mut end = start;
    for character in formula[start..].chars() {
        if !is_sheet_token_character(character) {
            break;
        }
        end += character.len_utf8();
    }
    (end > start).then(|| SheetEndpoint {
        name: formula[start..end].to_owned(),
        end,
        quoted: false,
    })
}

fn external_reference_end(formula: &str, start: usize) -> Option<usize> {
    let workbook_end = formula.as_bytes()[start + 1..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|offset| start + offset + 2)?;
    let prefix = parse_sheet_prefix(formula, workbook_end)?;
    Some(prefix.end)
}

fn quoted_end(formula: &str, start: usize, quote: u8) -> Option<usize> {
    let mut index = start + 1;
    while index < formula.len() {
        if formula.as_bytes()[index] == quote {
            if formula.as_bytes().get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return Some(index + 1);
            }
        } else {
            index += formula[index..].chars().next()?.len_utf8();
        }
    }
    None
}

fn is_sheet_token_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '.' | '\\')
}

fn render_sheet_name(name: &str) -> String {
    if name.chars().all(is_sheet_token_character) {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn rewrite_single_sheet(
    endpoint: &SheetEndpoint,
    old_name: &str,
    new_name: &str,
) -> Option<String> {
    if endpoint.name.contains(['[', ']'])
        || case_insensitive_key(&endpoint.name) != case_insensitive_key(old_name)
    {
        return None;
    }
    if endpoint.quoted {
        Some(format!("'{}'", new_name.replace('\'', "''")))
    } else {
        Some(render_sheet_name(new_name))
    }
}

fn rewrite_sheet_range(
    first: &str,
    second: &str,
    old_name: &str,
    new_name: &str,
    preserve_quotes: bool,
) -> Option<String> {
    if first.contains(['[', ']']) || second.contains(['[', ']']) {
        return None;
    }
    let old_key = case_insensitive_key(old_name);
    let mut changed = false;
    let rewritten_first = if case_insensitive_key(first) == old_key {
        changed = true;
        new_name
    } else {
        first
    };
    let rewritten_second = if case_insensitive_key(second) == old_key {
        changed = true;
        new_name
    } else {
        second
    };
    if !changed {
        return None;
    }
    let range = format!("{rewritten_first}:{rewritten_second}");
    if preserve_quotes
        || !rewritten_first.chars().all(is_sheet_token_character)
        || !rewritten_second.chars().all(is_sheet_token_character)
    {
        Some(format!("'{}'", range.replace('\'', "''")))
    } else {
        Some(range)
    }
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::rename_sheet_references;

    #[test]
    fn sheet_reference_renaming_preserves_literals_and_quotes_new_names() {
        let formula = r#"Old!A1+'Old'!B2+"Old!C3"+Other!A1+[Book.xlsx]Old!A1"#;
        assert_eq!(
            rename_sheet_references(formula, "Old", "New Name"),
            r#"'New Name'!A1+'New Name'!B2+"Old!C3"+Other!A1+[Book.xlsx]Old!A1"#
        );
    }

    #[test]
    fn sheet_reference_renaming_updates_both_three_d_range_endpoints() {
        assert_eq!(
            rename_sheet_references("SUM(Old:Sheet3!A1)", "Old", "First"),
            "SUM(First:Sheet3!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM(Sheet1:Old!A1)", "Old", "Last"),
            "SUM(Sheet1:Last!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM('Old:Sheet 3'!A1)", "Old", "First"),
            "SUM('First:Sheet 3'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM('Sheet 1:Old'!A1)", "Old", "Last"),
            "SUM('Sheet 1:Last'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM(Old:'Sheet 3'!A1)", "Old", "First"),
            "SUM('First:Sheet 3'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM('Sheet 1':Old!A1)", "Old", "Last"),
            "SUM('Sheet 1:Last'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM('Old':Sheet3!A1)", "Old", "First"),
            "SUM('First:Sheet3'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM(Sheet1:'Old'!A1)", "Old", "Last"),
            "SUM('Sheet1:Last'!A1)"
        );
    }

    #[test]
    fn sheet_reference_renaming_quotes_three_d_ranges_when_required() {
        assert_eq!(
            rename_sheet_references("SUM(Old:Sheet3!A1)", "Old", "New Name"),
            "SUM('New Name:Sheet3'!A1)"
        );
        assert_eq!(
            rename_sheet_references("SUM(Sheet1:Old!A1)", "Old", "New's"),
            "SUM('Sheet1:New''s'!A1)"
        );
    }

    #[test]
    fn sheet_reference_renaming_supports_unquoted_unicode_names() {
        assert_eq!(
            rename_sheet_references("합계!A1+Sheet2:합계!B2", "합계", "결과"),
            "결과!A1+Sheet2:결과!B2"
        );
    }

    #[test]
    fn sheet_reference_renaming_preserves_external_three_d_references() {
        let formula = r#"[Book.xlsx]Old:Sheet3!A1+[Book.xlsx]Sheet1:Old!A1+'[Book.xlsx]Old:Sheet3'!A1+[Book.xlsx]Old:'Sheet 3'!A1"#;
        assert_eq!(rename_sheet_references(formula, "Old", "New"), formula);
    }
}
