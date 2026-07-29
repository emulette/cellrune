fn column_number(label: &str) -> Option<u32> {
    let mut value = 0_u32;
    for byte in label.bytes() {
        value = value
            .checked_mul(26)?
            .checked_add(u32::from(byte.to_ascii_uppercase().checked_sub(b'A')? + 1))?;
    }
    (value <= 16_384).then_some(value)
}

fn column_label(mut value: u32) -> Result<String, String> {
    if !(1..=16_384).contains(&value) {
        return Err(format!(
            "shared-formula column is outside Excel's grid: {value}"
        ));
    }
    let mut reversed = Vec::new();
    while value > 0 {
        value -= 1;
        reversed.push((b'A' + (value % 26) as u8) as char);
        value /= 26;
    }
    Ok(reversed.into_iter().rev().collect())
}

fn parse_address(address: &str) -> Result<(u32, u32), String> {
    let split = address
        .find(|character: char| character.is_ascii_digit())
        .ok_or_else(|| format!("invalid A1 cell address: {address}"))?;
    let (column, row) = address.split_at(split);
    if column.is_empty()
        || column.len() > 3
        || !column.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(format!("invalid A1 cell address: {address}"));
    }
    let column =
        column_number(column).ok_or_else(|| format!("invalid A1 cell address: {address}"))?;
    let row = row
        .parse::<u32>()
        .map_err(|error| format!("invalid A1 cell address {address}: {error}"))?;
    if !(1..=1_048_576).contains(&row) {
        return Err(format!(
            "A1 cell address is outside Excel's grid: {address}"
        ));
    }
    Ok((column, row))
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
    Err("unterminated quote in shared formula".to_owned())
}

fn copy_bracketed(source: &str, start: usize) -> Result<usize, String> {
    let bytes = source.as_bytes();
    let mut depth = 0_i32;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index + 1);
                }
            }
            _ => {}
        }
        index += source[index..]
            .chars()
            .next()
            .expect("index remains in source")
            .len_utf8();
    }
    Err("unterminated bracketed token in shared formula".to_owned())
}

fn is_identifier(byte: Option<&u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
}

fn translate_plain(plain: &str, column_delta: i32, row_delta: i32) -> Result<String, String> {
    let bytes = plain.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let column_locked = bytes.get(index) == Some(&b'$');
        if column_locked {
            index += 1;
        }
        let column_start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphabetic())
            && index - column_start < 3
        {
            index += 1;
        }
        let column_end = index;
        let row_locked = bytes.get(index) == Some(&b'$');
        if row_locked {
            index += 1;
        }
        let row_start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        let looks_like_reference = column_end > column_start
            && row_start < index
            && !is_identifier(start.checked_sub(1).and_then(|at| bytes.get(at)))
            && !is_identifier(bytes.get(index))
            && bytes.get(index) != Some(&b'(')
            && bytes.get(index) != Some(&b'!');
        let original_column = column_number(&plain[column_start..column_end]);
        let original_row = plain[row_start..index].parse::<u32>().ok();
        let reference = match (looks_like_reference, original_column, original_row) {
            (true, Some(column), Some(row)) if (1..=1_048_576).contains(&row) => {
                Some((column, row))
            }
            _ => None,
        };
        if let Some((original_column, original_row)) = reference {
            let column = if column_locked {
                i32::try_from(original_column).expect("grid fits i32")
            } else {
                i32::try_from(original_column).expect("grid fits i32") + column_delta
            };
            let row = if row_locked {
                i32::try_from(original_row).expect("grid fits i32")
            } else {
                i32::try_from(original_row).expect("grid fits i32") + row_delta
            };
            if !(1..=16_384).contains(&column) || !(1..=1_048_576).contains(&row) {
                result.push_str("#REF!");
            } else {
                if column_locked {
                    result.push('$');
                }
                result.push_str(&column_label(column as u32)?);
                if row_locked {
                    result.push('$');
                }
                result.push_str(&row.to_string());
            }
        } else {
            result.push_str(&plain[start..index]);
        }
        if index == start {
            let character = plain[index..]
                .chars()
                .next()
                .expect("index remains in source");
            result.push(character);
            index += character.len_utf8();
        }
    }
    Ok(result)
}

pub(super) fn translate(
    formula: &str,
    anchor_address: &str,
    target_address: &str,
) -> Result<String, String> {
    let (anchor_column, anchor_row) = parse_address(anchor_address)?;
    let (target_column, target_row) = parse_address(target_address)?;
    let column_delta = target_column as i32 - anchor_column as i32;
    let row_delta = target_row as i32 - anchor_row as i32;
    let bytes = formula.as_bytes();
    let mut result = String::new();
    let mut plain_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let end = match bytes[index] {
            b'"' | b'\'' => Some(copy_quoted(formula, index, bytes[index])?),
            b'[' => Some(copy_bracketed(formula, index)?),
            _ => None,
        };
        if let Some(end) = end {
            result.push_str(&translate_plain(
                &formula[plain_start..index],
                column_delta,
                row_delta,
            )?);
            result.push_str(&formula[index..end]);
            index = end;
            plain_start = end;
        } else {
            index += formula[index..]
                .chars()
                .next()
                .expect("index remains in source")
                .len_utf8();
        }
    }
    result.push_str(&translate_plain(
        &formula[plain_start..],
        column_delta,
        row_delta,
    )?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::translate;

    #[test]
    fn shared_formulas_translate_only_relative_a1_tokens() {
        assert_eq!(
            translate("A1+$B1+C$1+$D$1", "F5", "H8").expect("translated"),
            "C4+$B4+E$1+$D$1"
        );
        assert_eq!(
            translate(
                "SUM('입력 시트'!A1,Table1[A1],\"A1\",LOG10(A1))",
                "F5",
                "F6"
            )
            .expect("translated"),
            "SUM('입력 시트'!A2,Table1[A1],\"A1\",LOG10(A2))"
        );
    }
}
