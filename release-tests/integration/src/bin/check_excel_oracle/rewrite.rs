use std::collections::BTreeSet;

use cellrune_integration_tests::oracle::HostFormulaRewrite;

const REWRITE_KINDS: [&str; 7] = [
    "simple_sheet_quotes",
    "xlws_prefix",
    "xlfn_prefix",
    "iso_ceiling_prefix",
    "single_wrapper",
    "negative_zero",
    "online_implicit_intersection",
];

pub(super) fn validate_declarations(
    declarations: &[HostFormulaRewrite],
    profile_ids: &BTreeSet<&str>,
) -> Result<(), String> {
    let mut kinds = BTreeSet::new();
    for declaration in declarations {
        if !REWRITE_KINDS.contains(&declaration.kind.as_str())
            || !kinds.insert(declaration.kind.as_str())
            || declaration.profile_ids.is_empty()
        {
            return Err("invalid or duplicate host formula rewrite declaration".to_owned());
        }
        let declared = declaration
            .profile_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if declared.len() != declaration.profile_ids.len() || !declared.is_subset(profile_ids) {
            return Err("host formula rewrite names an invalid profile set".to_owned());
        }
    }
    Ok(())
}

pub(super) fn accepted_rewrites(
    built: &str,
    saved: &str,
    declarations: &[HostFormulaRewrite],
    profile_id: &str,
) -> Result<Vec<String>, String> {
    if built == saved {
        return Ok(Vec::new());
    }
    let mut normalized_built = built.to_owned();
    let mut normalized_saved = saved.to_owned();
    let mut applied = Vec::new();
    for declaration in declarations
        .iter()
        .filter(|declaration| declaration.profile_ids.iter().any(|id| id == profile_id))
    {
        let next_built = apply(&normalized_built, &declaration.kind)?;
        let next_saved = apply(&normalized_saved, &declaration.kind)?;
        if (next_built != normalized_built) != (next_saved != normalized_saved) {
            applied.push(declaration.kind.clone());
        }
        normalized_built = next_built;
        normalized_saved = next_saved;
    }
    if normalized_built == normalized_saved {
        Ok(applied)
    } else {
        Err(format!(
            "saved formula is not an allowed rewrite; built={built} saved={saved}"
        ))
    }
}

fn apply(formula: &str, kind: &str) -> Result<String, String> {
    match kind {
        "simple_sheet_quotes" => Ok(normalize_sheet_quotes(formula)?),
        "xlws_prefix" => Ok(map_outside_protected_tokens(formula, |plain| {
            plain.replace("_xlfn._xlws.", "_xlfn.")
        })?),
        "xlfn_prefix" => Ok(map_outside_protected_tokens(formula, |plain| {
            plain.replace("_xlfn.", "")
        })?),
        "iso_ceiling_prefix" => Ok(map_outside_protected_tokens(formula, |plain| {
            plain.replace("_xlfn.ISO.CEILING", "ISO.CEILING")
        })?),
        "single_wrapper" => remove_single_wrappers(formula),
        "negative_zero" => Ok(map_outside_protected_tokens(
            formula,
            normalize_negative_zero,
        )?),
        "online_implicit_intersection" => {
            let unwrapped = remove_single_wrappers(formula)?;
            map_outside_protected_tokens(&unwrapped, normalize_formatting_whitespace)
        }
        _ => Err(format!("unknown host formula rewrite kind {kind}")),
    }
}

fn copy_quoted(source: &str, start: usize, quote: u8) -> Result<usize, String> {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] != quote {
            index += source[index..]
                .chars()
                .next()
                .expect("index remains in source")
                .len_utf8();
        } else if bytes.get(index + 1) == Some(&quote) {
            index += 2;
        } else {
            return Ok(index + 1);
        }
    }
    Err("unterminated quote in formula rewrite".to_owned())
}

fn bracketed_token_end(source: &str, start: usize) -> Result<usize, String> {
    source[start + 1..]
        .find(']')
        .map(|relative| start + relative + 2)
        .ok_or_else(|| "unterminated structured-reference token in formula rewrite".to_owned())
}

fn map_outside_protected_tokens(
    source: &str,
    transform: impl Fn(&str) -> String,
) -> Result<String, String> {
    let bytes = source.as_bytes();
    let mut result = String::new();
    let mut plain_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if !matches!(bytes[index], b'"' | b'\'' | b'[') {
            index += 1;
            continue;
        }
        result.push_str(&transform(&source[plain_start..index]));
        let end = if bytes[index] == b'[' {
            bracketed_token_end(source, index)?
        } else {
            copy_quoted(source, index, bytes[index])?
        };
        result.push_str(&source[index..end]);
        index = end;
        plain_start = end;
    }
    result.push_str(&transform(&source[plain_start..]));
    Ok(result)
}

fn is_whitespace_delimiter(character: Option<char>) -> bool {
    character.is_some_and(|value| ",;{}+-*/^&=<>%:".contains(value))
}

fn normalize_formatting_whitespace(plain: &str) -> String {
    let mut result = String::new();
    let mut characters = plain.chars().peekable();
    while let Some(character) = characters.next() {
        if !character.is_whitespace() {
            result.push(character);
            continue;
        }
        while characters.peek().is_some_and(|next| next.is_whitespace()) {
            characters.next();
        }
        let previous = result.chars().next_back();
        let next = characters.peek().copied();
        if !is_whitespace_delimiter(previous) && !is_whitespace_delimiter(next) {
            result.push(' ');
        }
    }
    result
}

fn normalize_sheet_quotes(source: &str) -> Result<String, String> {
    let bytes = source.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let end = copy_quoted(source, index, b'"')?;
            result.push_str(&source[index..end]);
            index = end;
            continue;
        }
        if bytes[index] == b'\'' {
            let end = copy_quoted(source, index, b'\'')?;
            let value = &source[index + 1..end - 1];
            let simple = value.split(':').all(|part| {
                !part.is_empty()
                    && part.as_bytes()[0].is_ascii_alphabetic()
                    && part
                        .bytes()
                        .all(|byte| byte == b'_' || byte == b'.' || byte.is_ascii_alphanumeric())
            }) && bytes.get(end) == Some(&b'!');
            if simple {
                result.push_str(value);
            } else {
                result.push_str(&source[index..end]);
            }
            index = end;
            continue;
        }
        let character = source[index..]
            .chars()
            .next()
            .expect("index remains in source");
        result.push(character);
        index += character.len_utf8();
    }
    Ok(result)
}

fn normalize_negative_zero(plain: &str) -> String {
    let bytes = plain.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'-'
            && bytes.get(index + 1) == Some(&b'0')
            && !index
                .checked_sub(1)
                .and_then(|position| bytes.get(position))
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
            && !bytes
                .get(index + 2)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            result.push('0');
            index += 2;
        } else {
            let character = plain[index..]
                .chars()
                .next()
                .expect("index remains in source");
            result.push(character);
            index += character.len_utf8();
        }
    }
    result
}

fn call_close(source: &str, open: usize) -> Result<(usize, usize), String> {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut arguments = 1_usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => {
                index = copy_quoted(source, index, bytes[index])?;
                continue;
            }
            b'(' | b'{' => depth += 1,
            b')' | b'}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "unbalanced formula call".to_owned())?;
                if depth == 0 {
                    return Ok((index, arguments));
                }
            }
            b',' if depth == 1 => arguments += 1,
            _ => {}
        }
        index += source[index..]
            .chars()
            .next()
            .expect("index remains in source")
            .len_utf8();
    }
    Err("unterminated formula call".to_owned())
}

fn remove_single_wrappers(source: &str) -> Result<String, String> {
    const MARKER: &str = "_xlfn.SINGLE(";
    let bytes = source.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            let end = copy_quoted(source, index, bytes[index])?;
            result.push_str(&source[index..end]);
            index = end;
            continue;
        }
        if bytes[index] == b'[' {
            let end = bracketed_token_end(source, index)?;
            result.push_str(&source[index..end]);
            index = end;
            continue;
        }
        if source[index..].starts_with(MARKER) {
            let open = index + MARKER.len() - 1;
            let (close, arguments) = call_close(source, open)?;
            if arguments == 1 {
                result.push_str(&remove_single_wrappers(&source[open + 1..close])?);
                index = close + 1;
                continue;
            }
        }
        let character = source[index..]
            .chars()
            .next()
            .expect("index remains in source");
        result.push(character);
        index += character.len_utf8();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use cellrune_integration_tests::oracle::HostFormulaRewrite;

    use super::{accepted_rewrites, validate_declarations};

    #[test]
    fn exact_declared_rewrites_are_accepted_but_formula_changes_are_not() {
        let profiles = BTreeSet::from(["online", "mac"]);
        let declarations = vec![
            HostFormulaRewrite {
                kind: "simple_sheet_quotes".to_owned(),
                profile_ids: vec!["online".to_owned(), "mac".to_owned()],
            },
            HostFormulaRewrite {
                kind: "xlws_prefix".to_owned(),
                profile_ids: vec!["online".to_owned(), "mac".to_owned()],
            },
        ];
        validate_declarations(&declarations, &profiles).expect("valid declarations");
        assert_eq!(
            accepted_rewrites(
                "_xlfn._xlws.SORTBY('Data'!A1:A2,'Data'!B1:B2)",
                "_xlfn.SORTBY(Data!A1:A2,Data!B1:B2)",
                &declarations,
                "mac",
            )
            .expect("accepted rewrite"),
            ["simple_sheet_quotes", "xlws_prefix"]
        );
        assert!(accepted_rewrites("SUM(A1:A2)", "1+1", &declarations, "online").is_err());
    }

    #[test]
    fn semantic_intersection_spaces_and_protected_tokens_are_not_rewritten() {
        let online = HostFormulaRewrite {
            kind: "online_implicit_intersection".to_owned(),
            profile_ids: vec!["online".to_owned()],
        };
        assert!(
            accepted_rewrites(
                "SUM(A1:A3 B1:B3)",
                "SUM(A1:A3B1:B3)",
                std::slice::from_ref(&online),
                "online",
            )
            .is_err()
        );
        assert!(
            accepted_rewrites(
                "SUM(A1 'Data'!A1)",
                "SUM(A1'Data'!A1)",
                std::slice::from_ref(&online),
                "online",
            )
            .is_err()
        );
        assert!(
            accepted_rewrites(
                "SUM(A1:A3 (A2:A4))",
                "SUM(A1:A3(A2:A4))",
                std::slice::from_ref(&online),
                "online",
            )
            .is_err()
        );
        assert!(
            accepted_rewrites(
                "SUM(OFFSET(A1,0,0,3,1) A2:A3)",
                "SUM(OFFSET(A1,0,0,3,1)A2:A3)",
                std::slice::from_ref(&online),
                "online",
            )
            .is_err()
        );
        assert_eq!(
            accepted_rewrites(
                "_xlfn.LAMBDA(_xlpm.x,IF(_xlfn.ISOMITTED(_xlpm.x),1,2))()",
                "_xlfn.SINGLE(_xlfn.LAMBDA(_xlpm.x, IF(_xlfn.SINGLE(_xlfn.ISOMITTED(_xlpm.x)), 1, 2))())",
                &[online],
                "online",
            )
            .expect("formatting and SINGLE wrappers are accepted"),
            ["online_implicit_intersection"]
        );

        let prefix = HostFormulaRewrite {
            kind: "xlfn_prefix".to_owned(),
            profile_ids: vec!["online".to_owned()],
        };
        assert!(
            accepted_rewrites(
                "SUM(Table1[_xlfn.VALUE])",
                "SUM(Table1[VALUE])",
                &[prefix],
                "online",
            )
            .is_err()
        );
    }
}
