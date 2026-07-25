use crate::{CellAddress, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormulaShiftError {
    MalformedQuotedSection,
    ReferenceOutOfBounds,
}

#[derive(Debug, Clone, Copy)]
struct CellReference {
    end: usize,
    column: u32,
    row: u32,
    column_absolute: bool,
    row_absolute: bool,
}

#[derive(Debug, Clone, Copy)]
struct AxisReference {
    end: usize,
    value: u32,
    absolute: bool,
}

pub(super) fn shift_formula(
    formula: &str,
    anchor: CellAddress,
    follower: CellAddress,
) -> Result<String, FormulaShiftError> {
    let row_delta = i64::from(follower.row().get()) - i64::from(anchor.row().get());
    let column_delta = i64::from(follower.column().get()) - i64::from(anchor.column().get());
    if row_delta == 0 && column_delta == 0 {
        return Ok(formula.to_owned());
    }

    let mut output = String::with_capacity(formula.len());
    let mut index = 0_usize;
    while index < formula.len() {
        let byte = formula.as_bytes()[index];
        if byte == b'"' || byte == b'\'' {
            let end = quoted_end(formula, index, byte)?;
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        if byte == b'[' {
            let end = bracket_end(formula, index)?;
            output.push_str(&formula[index..end]);
            index = end;
            continue;
        }
        if let Some((end, first, second)) = parse_column_range(formula, index) {
            write_axis(
                &mut output,
                first,
                column_delta,
                EXCEL_MAX_COLUMNS,
                column_label,
            )?;
            output.push(':');
            write_axis(
                &mut output,
                second,
                column_delta,
                EXCEL_MAX_COLUMNS,
                column_label,
            )?;
            index = end;
            continue;
        }
        if let Some((end, first, second)) = parse_row_range(formula, index) {
            write_axis(&mut output, first, row_delta, EXCEL_MAX_ROWS, |value| {
                value.to_string()
            })?;
            output.push(':');
            write_axis(&mut output, second, row_delta, EXCEL_MAX_ROWS, |value| {
                value.to_string()
            })?;
            index = end;
            continue;
        }
        if let Some(reference) = parse_cell_reference(formula, index) {
            if is_sheet_range_prefix(formula, reference.end) {
                output.push_str(&formula[index..reference.end]);
            } else {
                write_cell_reference(&mut output, reference, row_delta, column_delta)?;
            }
            index = reference.end;
            continue;
        }
        let character = formula[index..]
            .chars()
            .next()
            .ok_or(FormulaShiftError::MalformedQuotedSection)?;
        output.push(character);
        index += character.len_utf8();
    }
    Ok(output)
}

fn quoted_end(formula: &str, start: usize, quote: u8) -> Result<usize, FormulaShiftError> {
    let mut index = start + 1;
    while index < formula.len() {
        if formula.as_bytes()[index] == quote {
            if formula.as_bytes().get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return Ok(index + 1);
            }
        } else {
            let character = formula[index..]
                .chars()
                .next()
                .ok_or(FormulaShiftError::MalformedQuotedSection)?;
            index += character.len_utf8();
        }
    }
    Err(FormulaShiftError::MalformedQuotedSection)
}

fn bracket_end(formula: &str, start: usize) -> Result<usize, FormulaShiftError> {
    formula.as_bytes()[start + 1..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|offset| start + offset + 2)
        .ok_or(FormulaShiftError::MalformedQuotedSection)
}

fn parse_cell_reference(formula: &str, start: usize) -> Option<CellReference> {
    if !is_token_start(formula, start) {
        return None;
    }
    let bytes = formula.as_bytes();
    let mut index = start;
    let column_absolute = bytes.get(index) == Some(&b'$');
    if column_absolute {
        index += 1;
    }
    let column_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
        index += 1;
    }
    if index == column_start || index - column_start > 3 {
        return None;
    }
    let column = parse_column(&formula[column_start..index])?;
    let row_absolute = bytes.get(index) == Some(&b'$');
    if row_absolute {
        index += 1;
    }
    let row_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == row_start || !is_token_end(formula, index) {
        return None;
    }
    let row = formula[row_start..index].parse::<u32>().ok()?;
    if !(1..=EXCEL_MAX_ROWS).contains(&row) {
        return None;
    }
    Some(CellReference {
        end: index,
        column,
        row,
        column_absolute,
        row_absolute,
    })
}

fn parse_column_range(
    formula: &str,
    start: usize,
) -> Option<(usize, AxisReference, AxisReference)> {
    if !is_token_start(formula, start) {
        return None;
    }
    let first = parse_column_axis(formula, start)?;
    if formula.as_bytes().get(first.end) != Some(&b':') {
        return None;
    }
    let second = parse_column_axis(formula, first.end + 1)?;
    if !is_token_end(formula, second.end) {
        return None;
    }
    Some((second.end, first, second))
}

fn parse_column_axis(formula: &str, start: usize) -> Option<AxisReference> {
    let bytes = formula.as_bytes();
    let mut index = start;
    let absolute = bytes.get(index) == Some(&b'$');
    if absolute {
        index += 1;
    }
    let label_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
        index += 1;
    }
    if index == label_start || index - label_start > 3 {
        return None;
    }
    Some(AxisReference {
        end: index,
        value: parse_column(&formula[label_start..index])?,
        absolute,
    })
}

fn parse_row_range(formula: &str, start: usize) -> Option<(usize, AxisReference, AxisReference)> {
    if !is_token_start(formula, start) {
        return None;
    }
    let first = parse_row_axis(formula, start)?;
    if formula.as_bytes().get(first.end) != Some(&b':') {
        return None;
    }
    let second = parse_row_axis(formula, first.end + 1)?;
    if !is_token_end(formula, second.end) {
        return None;
    }
    Some((second.end, first, second))
}

fn parse_row_axis(formula: &str, start: usize) -> Option<AxisReference> {
    let bytes = formula.as_bytes();
    let mut index = start;
    let absolute = bytes.get(index) == Some(&b'$');
    if absolute {
        index += 1;
    }
    let digits_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == digits_start {
        return None;
    }
    let value = formula[digits_start..index].parse::<u32>().ok()?;
    if !(1..=EXCEL_MAX_ROWS).contains(&value) {
        return None;
    }
    Some(AxisReference {
        end: index,
        value,
        absolute,
    })
}

fn is_token_start(formula: &str, start: usize) -> bool {
    start == 0
        || formula
            .as_bytes()
            .get(start.wrapping_sub(1))
            .is_none_or(|byte| !is_identifier_byte(*byte) && *byte != b'$')
}

fn is_token_end(formula: &str, end: usize) -> bool {
    formula
        .as_bytes()
        .get(end)
        .is_none_or(|byte| !is_identifier_byte(*byte) && !matches!(*byte, b'!' | b'(' | b'['))
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'\\')
}

fn is_sheet_range_prefix(formula: &str, end: usize) -> bool {
    if formula.as_bytes().get(end) != Some(&b':') {
        return false;
    }
    let mut index = end + 1;
    if formula.as_bytes().get(index) == Some(&b'\'') {
        let Ok(quoted_end) = quoted_end(formula, index, b'\'') else {
            return false;
        };
        index = quoted_end;
    } else {
        while formula
            .as_bytes()
            .get(index)
            .is_some_and(|byte| is_identifier_byte(*byte))
        {
            index += 1;
        }
    }
    formula.as_bytes().get(index) == Some(&b'!')
}

fn parse_column(label: &str) -> Option<u32> {
    let mut column = 0_u32;
    for byte in label.bytes() {
        column = column
            .checked_mul(26)?
            .checked_add(u32::from(byte.to_ascii_uppercase() - b'A' + 1))?;
    }
    (1..=EXCEL_MAX_COLUMNS).contains(&column).then_some(column)
}

fn write_cell_reference(
    output: &mut String,
    reference: CellReference,
    row_delta: i64,
    column_delta: i64,
) -> Result<(), FormulaShiftError> {
    if reference.column_absolute {
        output.push('$');
    }
    let column = shifted(
        reference.column,
        column_delta,
        reference.column_absolute,
        EXCEL_MAX_COLUMNS,
    )?;
    output.push_str(&column_label(column));
    if reference.row_absolute {
        output.push('$');
    }
    let row = shifted(
        reference.row,
        row_delta,
        reference.row_absolute,
        EXCEL_MAX_ROWS,
    )?;
    output.push_str(&row.to_string());
    Ok(())
}

fn write_axis(
    output: &mut String,
    reference: AxisReference,
    delta: i64,
    maximum: u32,
    render: impl FnOnce(u32) -> String,
) -> Result<(), FormulaShiftError> {
    if reference.absolute {
        output.push('$');
    }
    let value = shifted(reference.value, delta, reference.absolute, maximum)?;
    output.push_str(&render(value));
    Ok(())
}

fn shifted(value: u32, delta: i64, absolute: bool, maximum: u32) -> Result<u32, FormulaShiftError> {
    if absolute {
        return Ok(value);
    }
    let shifted = i64::from(value) + delta;
    if !(1..=i64::from(maximum)).contains(&shifted) {
        return Err(FormulaShiftError::ReferenceOutOfBounds);
    }
    u32::try_from(shifted).map_err(|_| FormulaShiftError::ReferenceOutOfBounds)
}

fn column_label(mut column: u32) -> String {
    let mut bytes = Vec::with_capacity(3);
    while column > 0 {
        let offset = ((column - 1) % 26) as u8;
        bytes.push(b'A' + offset);
        column = (column - 1) / 26;
    }
    bytes.reverse();
    bytes.into_iter().map(char::from).collect()
}

#[cfg(test)]
mod tests {
    use super::{FormulaShiftError, shift_formula};
    use crate::CellAddress;

    #[test]
    fn shifts_relative_a1_references_without_touching_formula_literals_or_names() {
        let anchor = CellAddress::from_indices(2, 2).expect("anchor");
        let follower = CellAddress::from_indices(3, 3).expect("follower");
        let formula = concat!(
            "A1+$A1+A$1+$A$1+SUM(A:A,1:1,LOG10(10),",
            "Sheet1!A1,'A1'!B2,Sheet1:Sheet3!A1,\"A1\",Table1[A1])"
        );
        assert_eq!(
            shift_formula(formula, anchor, follower).expect("shift"),
            concat!(
                "B2+$A2+B$1+$A$1+SUM(B:B,2:2,LOG10(10),",
                "Sheet1!B2,'A1'!C3,Sheet1:Sheet3!B2,\"A1\",Table1[A1])"
            )
        );
    }

    #[test]
    fn rejects_relative_references_shifted_outside_excel_bounds() {
        let anchor = CellAddress::from_indices(2, 2).expect("anchor");
        let follower = CellAddress::from_indices(1, 1).expect("follower");
        assert_eq!(
            shift_formula("A1", anchor, follower),
            Err(FormulaShiftError::ReferenceOutOfBounds)
        );
        assert_eq!(
            shift_formula("$A$1", anchor, follower).expect("absolute reference"),
            "$A$1"
        );
    }
}
