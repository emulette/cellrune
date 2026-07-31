use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::coerce::{compare, to_logical};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::{Array, Rect};
use super::super::value::{ErrorKind, Value};
use super::super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
use super::util::{required_number, required_text};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "ADDRESS" => address(engine, context, args),
        "COLUMN" => column(engine, context, args),
        "HYPERLINK" => hyperlink(engine, context, args),
        "LOOKUP" => lookup(engine, context, args),
        "VLOOKUP" => table_lookup(engine, context, args, false),
        "HLOOKUP" => table_lookup(engine, context, args, true),
        "XLOOKUP" => xlookup(engine, context, args),
        "CHOOSE" => choose(engine, context, args),
        "ROWS" => dimension(engine, context, args, true),
        "COLUMNS" => dimension(engine, context, args, false),
        "ROW" => row(engine, context, args),
        "OFFSET" | "INDIRECT" => {
            let expression = Expr::Call {
                name: name.to_owned(),
                args: args.to_vec(),
            };
            match engine.resolve_rect_expr(context, &expression) {
                Ok(rect) if rect.is_single_cell() => engine
                    .read_reference_cell(context, (rect.sheet, rect.row_start, rect.col_start))
                    .unwrap_or_else(Value::Error),
                Ok(_) => Value::Error(ErrorKind::Unsupported),
                Err(kind) => Value::Error(kind),
            }
        }
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn address(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 5 {
        return Value::Error(ErrorKind::Value);
    }
    let row = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let column = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    if row < 1.0
        || row > f64::from(EXCEL_MAX_ROWS)
        || column < 1.0
        || column > f64::from(EXCEL_MAX_COLUMNS)
    {
        return Value::Error(ErrorKind::Value);
    }
    let absolute = match args.get(2) {
        None | Some(Expr::Missing) => 1_i32,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
    };
    if !(1..=4).contains(&absolute) {
        return Value::Error(ErrorKind::Value);
    }
    let a1 = match args.get(3) {
        None | Some(Expr::Missing) => true,
        Some(expr) => match to_logical(&engine.eval_scalar(context, expr)) {
            Ok(logical) => logical,
            Err(kind) => return Value::Error(kind),
        },
    };
    let sheet = match args.get(4) {
        None | Some(Expr::Missing) => None,
        Some(expr) => match required_text(engine, context, expr) {
            Ok(text) => Some(text),
            Err(kind) => return Value::Error(kind),
        },
    };
    let row = row as u32;
    let column = column as u32;
    let row_absolute = matches!(absolute, 1 | 2);
    let column_absolute = matches!(absolute, 1 | 3);
    let mut output = String::new();
    if let Some(sheet) = sheet {
        push_sheet_qualifier(&mut output, &sheet);
    }
    if a1 {
        if column_absolute {
            output.push('$');
        }
        output.push_str(&column_letters(column));
        if row_absolute {
            output.push('$');
        }
        output.push_str(&row.to_string());
    } else {
        output.push('R');
        if row_absolute {
            output.push_str(&row.to_string());
        } else {
            output.push('[');
            output.push_str(&row.to_string());
            output.push(']');
        }
        output.push('C');
        if column_absolute {
            output.push_str(&column.to_string());
        } else {
            output.push('[');
            output.push_str(&column.to_string());
            output.push(']');
        }
    }
    engine.bounded_text(output)
}

fn column(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Number(f64::from(context.column()));
    }
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => Value::Number(f64::from(rect.col_start)),
        Err(kind) => Value::Error(kind),
    }
}

fn hyperlink(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let location = match required_text(engine, context, &args[0]) {
        Ok(location) => location,
        Err(kind) => return Value::Error(kind),
    };
    match args.get(1) {
        None | Some(Expr::Missing) => engine.bounded_text(location),
        Some(friendly_name) => {
            let value = engine.eval_scalar(context, friendly_name);
            if let Value::Text(text) = &value
                && let Err(kind) = engine.ensure_text_bytes(text.len())
            {
                return Value::Error(kind);
            }
            value
        }
    }
}

fn lookup(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let lookup_value = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup_value {
        return Value::Error(kind);
    }
    let lookup_array = match engine.eval_array(context, &args[1]) {
        Ok(array) => array,
        Err(kind) => return Value::Error(kind),
    };
    match args.get(2) {
        Some(result) => {
            let result_array = match engine.eval_array(context, result) {
                Ok(array) => array,
                Err(kind) => return Value::Error(kind),
            };
            lookup_vector(engine, &lookup_value, &lookup_array, &result_array)
        }
        None => lookup_table(engine, &lookup_value, &lookup_array),
    }
}

fn lookup_vector(engine: &Engine<'_>, lookup: &Value, keys: &Array, results: &Array) -> Value {
    let Some((key_length, keys_vertical)) = vector_shape(keys) else {
        return Value::Error(ErrorKind::NA);
    };
    let Some((result_length, results_vertical)) = vector_shape(results) else {
        return Value::Error(ErrorKind::NA);
    };
    if key_length != result_length {
        return Value::Error(ErrorKind::NA);
    }
    if let Err(kind) = engine.ensure_function_iterations(u64::from(key_length) * 2) {
        return Value::Error(kind);
    }
    let matched = lookup_offset(lookup, key_length, |offset| {
        vector_value(keys, offset, keys_vertical)
    });
    matched.map_or(Value::Error(ErrorKind::NA), |offset| {
        vector_value(results, offset, results_vertical).clone()
    })
}

fn lookup_table(engine: &Engine<'_>, lookup: &Value, table: &Array) -> Value {
    let vertical = table.rows >= table.cols;
    let length = if vertical { table.rows } else { table.cols };
    if let Err(kind) = engine.ensure_function_iterations(u64::from(length) * 2) {
        return Value::Error(kind);
    }
    let matched = lookup_offset(lookup, length, |offset| {
        if vertical {
            table.at(offset, 0)
        } else {
            table.at(0, offset)
        }
    });
    matched.map_or(Value::Error(ErrorKind::NA), |offset| {
        if vertical {
            table.at(offset, table.cols - 1).clone()
        } else {
            table.at(table.rows - 1, offset).clone()
        }
    })
}

fn lookup_offset<'value>(
    lookup: &Value,
    length: u32,
    mut candidate: impl FnMut(u32) -> &'value Value,
) -> Option<u32> {
    let mut exact = None;
    for offset in 0..length {
        if matches!(
            candidate(offset),
            value if !matches!(value, Value::Error(_))
                && compare(value, lookup) == Ok(Ordering::Equal)
        ) {
            exact = Some(offset);
        }
    }
    if exact.is_some() {
        return exact;
    }
    let mut matched = None;
    for offset in 0..length {
        match candidate(offset) {
            Value::Error(_) => continue,
            value => match compare(value, lookup) {
                Ok(Ordering::Less) => matched = Some(offset),
                Ok(Ordering::Equal) => {
                    unreachable!("exact matches returned in the first pass")
                }
                Ok(Ordering::Greater) => break,
                Err(_) => continue,
            },
        }
    }
    matched
}

fn vector_shape(array: &Array) -> Option<(u32, bool)> {
    if array.cols == 1 {
        Some((array.rows, true))
    } else if array.rows == 1 {
        Some((array.cols, false))
    } else {
        None
    }
}

fn vector_value(array: &Array, offset: u32, vertical: bool) -> &Value {
    if vertical {
        array.at(offset, 0)
    } else {
        array.at(0, offset)
    }
}

fn column_letters(mut column: u32) -> String {
    let mut letters = Vec::new();
    while column > 0 {
        column -= 1;
        letters.push((b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    letters.into_iter().rev().collect()
}

fn push_sheet_qualifier(output: &mut String, sheet: &str) {
    let requires_quotes = sheet.is_empty()
        || sheet
            .chars()
            .any(|character| !character.is_alphanumeric() && !"_.[]".contains(character));
    if requires_quotes {
        output.push('\'');
        output.push_str(&sheet.replace('\'', "''"));
        output.push('\'');
    } else {
        output.push_str(sheet);
    }
    output.push('!');
}

fn xlookup(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 6 {
        return Value::Error(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let lookup_rect = match engine.resolve_rect_expr(context, &args[1]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let return_rect = match engine.resolve_rect_expr(context, &args[2]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let match_mode = match args.get(4) {
        None | Some(Expr::Missing) => 0,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
    };
    let search_mode = match args.get(5) {
        None | Some(Expr::Missing) => 1,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
    };
    if match_mode != 0 || !matches!(search_mode, 1 | -1) {
        return Value::Error(ErrorKind::Unsupported);
    }

    let vertical = lookup_rect.width() == 1;
    let horizontal = lookup_rect.height() == 1;
    if !vertical && !horizontal {
        return Value::Error(ErrorKind::Value);
    }
    let length = if vertical {
        let row_end = engine.clamped_row_end(&lookup_rect);
        if row_end < lookup_rect.row_start {
            0
        } else {
            u64::from(row_end - lookup_rect.row_start) + 1
        }
    } else {
        lookup_rect.width()
    };
    let return_length = if vertical && return_rect.width() == 1 {
        let row_end = engine.clamped_row_end(&return_rect);
        if row_end < return_rect.row_start {
            0
        } else {
            u64::from(row_end - return_rect.row_start) + 1
        }
    } else if horizontal && return_rect.height() == 1 {
        return_rect.width()
    } else {
        return Value::Error(ErrorKind::Value);
    };
    if length != return_length {
        return Value::Error(ErrorKind::Value);
    }
    if let Err(kind) = engine.ensure_array_cells(length) {
        return Value::Error(kind);
    }

    for step in 0..length as u32 {
        let offset = if search_mode == 1 {
            step
        } else {
            length as u32 - step - 1
        };
        let candidate_cell = if vertical {
            (
                lookup_rect.sheet,
                lookup_rect.row_start + offset,
                lookup_rect.col_start,
            )
        } else {
            (
                lookup_rect.sheet,
                lookup_rect.row_start,
                lookup_rect.col_start + offset,
            )
        };
        let candidate = match engine.read_reference_cell(context, candidate_cell) {
            Ok(value) => value,
            Err(kind) => return Value::Error(kind),
        };
        match compare(&candidate, &lookup) {
            Ok(Ordering::Equal) => {
                let result_cell = if vertical {
                    (
                        return_rect.sheet,
                        return_rect.row_start + offset,
                        return_rect.col_start,
                    )
                } else {
                    (
                        return_rect.sheet,
                        return_rect.row_start,
                        return_rect.col_start + offset,
                    )
                };
                return engine
                    .read_reference_cell(context, result_cell)
                    .unwrap_or_else(Value::Error);
            }
            Ok(Ordering::Less | Ordering::Greater) => {}
            Err(kind) => return Value::Error(kind),
        }
    }

    match args.get(3) {
        None | Some(Expr::Missing) => Value::Error(ErrorKind::NA),
        Some(if_not_found) => engine.eval_scalar(context, if_not_found),
    }
}

fn table_lookup(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    horizontal: bool,
) -> Value {
    if args.len() < 3 || args.len() > 4 {
        return Value::Error(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let rect = match engine.resolve_rect_expr(context, &args[1]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let result_index = match required_number(engine, context, &args[2]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let result_limit = if horizontal {
        rect.height()
    } else {
        rect.width()
    };
    if result_index < 1.0 || result_index as u64 > result_limit {
        return Value::Error(ErrorKind::Ref);
    }
    let approximate = match args.get(3) {
        None | Some(Expr::Missing) => true,
        Some(expr) => match to_logical(&engine.eval_scalar(context, expr)) {
            Ok(logical) => logical,
            Err(kind) => return Value::Error(kind),
        },
    };
    match find_lookup_offset(engine, context, &lookup, rect, horizontal, approximate) {
        Ok(offset) => {
            let row = if horizontal {
                rect.row_start + result_index as u32 - 1
            } else {
                rect.row_start + offset
            };
            let column = if horizontal {
                rect.col_start + offset
            } else {
                rect.col_start + result_index as u32 - 1
            };
            engine
                .read_reference_cell(context, (rect.sheet, row, column))
                .unwrap_or_else(Value::Error)
        }
        Err(kind) => Value::Error(kind),
    }
}

fn find_lookup_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lookup: &Value,
    rect: Rect,
    horizontal: bool,
    approximate: bool,
) -> Result<u32, ErrorKind> {
    let length = if horizontal {
        rect.width()
    } else if engine.clamped_row_end(&rect) < rect.row_start {
        0
    } else {
        u64::from(engine.clamped_row_end(&rect) - rect.row_start) + 1
    };
    engine.ensure_array_cells(length)?;

    for offset in 0..length as u32 {
        let value = lookup_axis_value(engine, context, rect, horizontal, offset)?;
        if compare(&value, lookup)? == Ordering::Equal {
            return Ok(offset);
        }
    }
    if !approximate {
        return Err(ErrorKind::NA);
    }

    let mut candidate = None;
    for offset in 0..length as u32 {
        let value = lookup_axis_value(engine, context, rect, horizontal, offset)?;
        match compare(&value, lookup)? {
            Ordering::Equal => unreachable!("exact matches returned in the first pass"),
            Ordering::Less => candidate = Some(offset),
            Ordering::Greater => break,
        }
    }
    candidate.ok_or(ErrorKind::NA)
}

fn lookup_axis_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    rect: Rect,
    horizontal: bool,
    offset: u32,
) -> Result<Value, ErrorKind> {
    let cell = if horizontal {
        (rect.sheet, rect.row_start, rect.col_start + offset)
    } else {
        (rect.sheet, rect.row_start + offset, rect.col_start)
    };
    engine.read_reference_cell(context, cell)
}

fn choose(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 {
        return Value::Error(ErrorKind::Value);
    }
    let index = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    if index < 1.0 || index as usize >= args.len() {
        return Value::Error(ErrorKind::Value);
    }
    engine.eval_scalar(context, &args[index as usize])
}

fn dimension(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], rows: bool) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => Value::Number(if rows {
            rect.height() as f64
        } else {
            rect.width() as f64
        }),
        Err(kind) => Value::Error(kind),
    }
}

fn row(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Number(context.row() as f64);
    }
    if args.len() > 1 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => Value::Number(rect.row_start as f64),
        Err(kind) => Value::Error(kind),
    }
}
