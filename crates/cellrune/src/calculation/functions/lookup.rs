use super::kernel::LookupFunction;
use std::borrow::Cow;
use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::coerce::{compare, to_logical};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::{Array, Rect};
use super::super::value::{ErrorKind, Value};
use super::super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
use super::descriptor::DynamicReferenceKind;
use super::lookup_common::VectorView;
use super::util::{required_number, required_text};
use super::xmatch::{find_match, parse_match_mode, parse_search_mode};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: LookupFunction,
    args: &[Expr],
) -> Value {
    match function {
        LookupFunction::Address => address(engine, context, args),
        LookupFunction::Column => column(engine, context, args),
        LookupFunction::Hyperlink => hyperlink(engine, context, args),
        LookupFunction::Lookup => lookup(engine, context, args),
        LookupFunction::VLookup => table_lookup(engine, context, args, false),
        LookupFunction::HLookup => table_lookup(engine, context, args, true),
        LookupFunction::XLookup => xlookup(engine, context, args),
        LookupFunction::Choose => choose(engine, context, args),
        LookupFunction::Rows => dimension(engine, context, args, true),
        LookupFunction::Columns => dimension(engine, context, args, false),
        LookupFunction::Row => row(engine, context, args),
        LookupFunction::Sheet => super::reference_introspection::sheet(engine, context, args),
        LookupFunction::Sheets => super::reference_introspection::sheets(engine, context, args),
        LookupFunction::XMatch => super::xmatch::xmatch(engine, context, args),
        LookupFunction::Offset | LookupFunction::Indirect => {
            let kind = match function {
                LookupFunction::Offset => DynamicReferenceKind::Offset,
                LookupFunction::Indirect => DynamicReferenceKind::Indirect,
                _ => unreachable!("only reference-returning lookup functions enter this branch"),
            };
            match engine.resolve_dynamic_rect(context, kind, args) {
                Ok(rect) if rect.is_single_cell() => engine
                    .read_reference_cell(context, (rect.sheet, rect.row_start, rect.col_start))
                    .unwrap_or_else(Value::Error),
                Ok(_) => Value::Error(ErrorKind::Unsupported),
                Err(kind) => Value::Error(kind),
            }
        }
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
    let Some(keys) = VectorView::new(keys) else {
        return Value::Error(ErrorKind::NA);
    };
    let Some(results) = VectorView::new(results) else {
        return Value::Error(ErrorKind::NA);
    };
    if keys.len() != results.len() {
        return Value::Error(ErrorKind::NA);
    }
    if let Err(kind) = engine.ensure_function_iterations(u64::from(keys.len()) * 2) {
        return Value::Error(kind);
    }
    let matched = lookup_offset(lookup, keys.len(), |offset| keys.at(offset));
    matched.map_or(Value::Error(ErrorKind::NA), |offset| {
        results.at(offset).clone()
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
    match call_xlookup_array(engine, context, args) {
        Ok(result) => result
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn call_xlookup_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 3 || args.len() > 6 {
        return Err(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Err(kind);
    }
    let lookup_rect = engine.resolve_rect_expr(context, &args[1])?;
    let return_rect = engine.resolve_rect_expr(context, &args[2])?;
    let match_mode = parse_match_mode(engine, context, args.get(4))?;
    let search_mode = parse_search_mode(engine, context, args.get(5))?;
    let orientation = xlookup_orientation(lookup_rect, return_rect)?;
    let length = if orientation == XLookupOrientation::Vertical {
        engine.operation_row_count([&lookup_rect, &return_rect])
    } else {
        lookup_rect.width()
    };
    let length = u32::try_from(length).map_err(|_| ErrorKind::Num)?;
    engine.ensure_array_cells(length.into())?;
    let match_offset = find_match(
        engine,
        context,
        &lookup,
        length,
        match_mode,
        search_mode,
        |offset| {
            let cell = lookup_axis_cell(lookup_rect, orientation, offset);
            engine.read_reference_cell(context, cell).map(Cow::Owned)
        },
    );
    match match_offset {
        Ok(offset) => xlookup_return_array(engine, context, return_rect, orientation, offset),
        Err(ErrorKind::NA) => Ok(Array::scalar(match args.get(3) {
            None | Some(Expr::Missing) => Value::Error(ErrorKind::NA),
            Some(if_not_found) => engine.eval_scalar(context, if_not_found),
        })),
        Err(kind) => Err(kind),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XLookupOrientation {
    Vertical,
    Horizontal,
}

fn xlookup_orientation(lookup: Rect, result: Rect) -> Result<XLookupOrientation, ErrorKind> {
    let vertical = lookup.width() == 1 && lookup.height() == result.height();
    let horizontal = lookup.height() == 1 && lookup.width() == result.width();
    match (vertical, horizontal) {
        (true, false) => Ok(XLookupOrientation::Vertical),
        (false, true) => Ok(XLookupOrientation::Horizontal),
        (true, true) => Ok(XLookupOrientation::Vertical),
        _ => Err(ErrorKind::Value),
    }
}

fn lookup_axis_cell(rect: Rect, orientation: XLookupOrientation, offset: u32) -> (usize, u32, u32) {
    match orientation {
        XLookupOrientation::Vertical => (rect.sheet, rect.row_start + offset, rect.col_start),
        XLookupOrientation::Horizontal => (rect.sheet, rect.row_start, rect.col_start + offset),
    }
}

fn xlookup_return_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    result: Rect,
    orientation: XLookupOrientation,
    offset: u32,
) -> Result<Array, ErrorKind> {
    let (rows, cols) = match orientation {
        XLookupOrientation::Vertical => (
            1,
            u32::try_from(result.width()).map_err(|_| ErrorKind::Num)?,
        ),
        XLookupOrientation::Horizontal => (
            u32::try_from(result.height()).map_err(|_| ErrorKind::Num)?,
            1,
        ),
    };
    let cells = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for index in 0..cells {
        if index % 256 == 0 {
            super::array_common::poll_cancellation(context)?;
        }
        let cell = match orientation {
            XLookupOrientation::Vertical => (
                result.sheet,
                result.row_start + offset,
                result.col_start + u32::try_from(index).map_err(|_| ErrorKind::Num)?,
            ),
            XLookupOrientation::Horizontal => (
                result.sheet,
                result.row_start + u32::try_from(index).map_err(|_| ErrorKind::Num)?,
                result.col_start + offset,
            ),
        };
        data.push(engine.read_reference_cell(context, cell)?);
    }
    Ok(Array { rows, cols, data })
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
    } else {
        engine.operation_row_count([&rect])
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
