use super::kernel::LookupFunction;
use std::borrow::Cow;
use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::coerce::{compare, to_logical, to_number};
use super::super::criteria::CompiledWildcardPattern;
use super::super::eval::{Engine, EvalContext, RectCells};
use super::super::runtime::{Array, Rect};
use super::super::value::{ErrorKind, Value};
use super::super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
use super::criteria_runtime::CriteriaRuntime;
use super::descriptor::DynamicReferenceKind;
use super::elementwise::Pick;
use super::lookup_common::VectorView;
use super::lookup_index::{
    LookupArea, SearchDirection, indexed_approximate_lookup_offset, indexed_lookup_offset,
    indexed_xlookup_offset, record_linear_search,
};
use super::util::{required_number, required_text};
use super::xmatch::{MatchMode, SearchMode, find_match, parse_match_mode, parse_search_mode};

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
            value if lookup_candidate_matches_type(lookup, value)
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
            value if !lookup_candidate_matches_type(lookup, value) => continue,
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

fn lookup_candidate_matches_type(lookup: &Value, candidate: &Value) -> bool {
    matches!(
        (lookup, candidate),
        (Value::Number(_) | Value::Blank, Value::Number(_))
            | (Value::Text(_), Value::Text(_))
            | (Value::Logical(_), Value::Logical(_))
    )
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
    // A scalar caller receives the returned reference through implicit intersection, as Excel
    // applies it to a legacy formula, instead of the first cell of a multi-cell result.
    match xlookup_outcome(engine, context, args) {
        Ok(XLookupOutcome::Found(slice)) => engine
            .implicit_intersection_rect(context, slice)
            .and_then(|cell| {
                engine.read_reference_cell(context, (cell.sheet, cell.row_start, cell.col_start))
            })
            .unwrap_or_else(Value::Error),
        // An array result takes its first value, as a formula that is not an array formula takes
        // an array's.
        Ok(XLookupOutcome::FoundValues(values)) => values.data[0].clone(),
        Ok(XLookupOutcome::NotFound(value)) => value,
        Err(kind) => Value::Error(kind),
    }
}

/// `XLOOKUP` in an array context. A lookup value, match mode, or search mode that holds an array
/// looks up each element and returns one value for it; a result row or column per element is
/// unsupported, as Excel's nested arrays are not modeled.
pub(super) fn call_xlookup_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let lifted = super::elementwise::lifted_positions(engine, context, args, [0, 4, 5].into_iter());
    let lifting = super::elementwise::Lifting {
        lifted: &lifted,
        once: &[],
        only_several: true,
    };
    if !lifted.is_empty()
        && let Some(values) =
            super::elementwise::lift(engine, context, args, args, &lifting, |context, args, _| {
                match xlookup_outcome(engine, context, args) {
                    Ok(XLookupOutcome::Found(slice)) if slice.is_single_cell() => engine
                        .read_reference_cell(
                            context,
                            (slice.sheet, slice.row_start, slice.col_start),
                        )
                        .unwrap_or_else(Value::Error),
                    Ok(XLookupOutcome::FoundValues(values)) if values.is_scalar() => {
                        values.data[0].clone()
                    }
                    Ok(XLookupOutcome::Found(_) | XLookupOutcome::FoundValues(_)) => {
                        Value::Error(ErrorKind::Unsupported)
                    }
                    Ok(XLookupOutcome::NotFound(value)) => value,
                    Err(kind) => Value::Error(kind),
                }
            })?
    {
        return Ok(values);
    }
    match xlookup_outcome(engine, context, args)? {
        XLookupOutcome::Found(slice) => xlookup_return_array(engine, context, slice),
        XLookupOutcome::FoundValues(values) => Ok(values),
        XLookupOutcome::NotFound(value) => Ok(Array::scalar(value)),
    }
}

enum XLookupOutcome {
    Found(Rect),
    /// The row or column of a return array that is not a reference.
    FoundValues(Array),
    NotFound(Value),
}

/// The arrays `XLOOKUP` searches and returns from: references, or arrays of values when either is
/// not a reference.
enum XLookupArrays {
    Rects { lookup: Rect, result: Rect },
    Values { lookup: Array, result: Array },
}

fn xlookup_outcome(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<XLookupOutcome, ErrorKind> {
    if args.len() < 3 || args.len() > 6 {
        return Err(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Err(kind);
    }
    let arrays = match (
        engine.resolve_rect_expr(context, &args[1]),
        engine.resolve_rect_expr(context, &args[2]),
    ) {
        (Ok(lookup), Ok(result)) => XLookupArrays::Rects { lookup, result },
        (Err(kind), _) | (_, Err(kind)) if kind.is_engine_issue() => return Err(kind),
        _ => XLookupArrays::Values {
            lookup: engine.eval_array(context, &args[1])?,
            result: engine.eval_array(context, &args[2])?,
        },
    };
    let match_mode = parse_match_mode(engine, context, args.get(4))?;
    let search_mode = parse_search_mode(engine, context, args.get(5))?;
    let (lookup_rect, return_rect) = match arrays {
        XLookupArrays::Rects { lookup, result } => (lookup, result),
        XLookupArrays::Values {
            lookup: lookup_values,
            result,
        } => {
            let search = XLookupSearch {
                lookup: &lookup,
                match_mode,
                search_mode,
            };
            return xlookup_values(engine, context, args, &search, &lookup_values, &result);
        }
    };
    let orientation = xlookup_orientation(lookup_rect, return_rect)?;
    let length = if orientation == XLookupOrientation::Vertical {
        engine.operation_row_count([&lookup_rect, &return_rect])
    } else {
        lookup_rect.width()
    };
    let length = u32::try_from(length).map_err(|_| ErrorKind::Num)?;
    engine.ensure_array_cells(length.into())?;
    let direction = match (match_mode, search_mode) {
        (MatchMode::Exact, SearchMode::Forward) => Some(SearchDirection::Forward),
        (MatchMode::Exact, SearchMode::Reverse) => Some(SearchDirection::Reverse),
        _ => None,
    };
    let indexable = direction.and_then(|direction| {
        let horizontal = orientation == XLookupOrientation::Horizontal;
        Some((
            LookupArea::new(lookup_rect, length.into(), horizontal)?,
            direction,
        ))
    });
    let indexed = indexable.and_then(|(area, direction)| {
        indexed_xlookup_offset(engine, context, area, &lookup, direction)
    });
    let match_offset = indexed.unwrap_or_else(|| {
        let found = find_match(
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
        if let Some((area, direction)) = indexable {
            record_linear_search(engine, area, direction, &found);
        }
        found
    });
    match match_offset {
        Ok(offset) => Ok(XLookupOutcome::Found(xlookup_return_slice(
            return_rect,
            orientation,
            offset,
        ))),
        Err(ErrorKind::NA) => Ok(XLookupOutcome::NotFound(match args.get(3) {
            None | Some(Expr::Missing) => Value::Error(ErrorKind::NA),
            Some(if_not_found) => engine.eval_scalar(context, if_not_found),
        })),
        Err(kind) => Err(kind),
    }
}

/// The value `XLOOKUP` looks for, and how.
struct XLookupSearch<'search> {
    lookup: &'search Value,
    match_mode: MatchMode,
    search_mode: SearchMode,
}

/// `XLOOKUP` over arrays of values, with the shape rules of references.
fn xlookup_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    search: &XLookupSearch<'_>,
    lookup_values: &Array,
    result: &Array,
) -> Result<XLookupOutcome, ErrorKind> {
    let shape = |array: &Array| Rect {
        sheet: 0,
        row_start: 1,
        col_start: 1,
        row_end: array.rows,
        col_end: array.cols,
        whole_rows: false,
    };
    let orientation = xlookup_orientation(shape(lookup_values), shape(result))?;
    let length = lookup_values.data.len() as u32;
    let found = find_match(
        engine,
        context,
        search.lookup,
        length,
        search.match_mode,
        search.search_mode,
        |offset| Ok(Cow::Borrowed(&lookup_values.data[offset as usize])),
    );
    match found {
        Ok(offset) => Ok(XLookupOutcome::FoundValues(match orientation {
            XLookupOrientation::Vertical => Array {
                rows: 1,
                cols: result.cols,
                data: (0..result.cols)
                    .map(|col| result.at(offset, col).clone())
                    .collect(),
            },
            XLookupOrientation::Horizontal => Array {
                rows: result.rows,
                cols: 1,
                data: (0..result.rows)
                    .map(|row| result.at(row, offset).clone())
                    .collect(),
            },
        })),
        Err(ErrorKind::NA) => Ok(XLookupOutcome::NotFound(match args.get(3) {
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

fn xlookup_return_slice(result: Rect, orientation: XLookupOrientation, offset: u32) -> Rect {
    match orientation {
        XLookupOrientation::Vertical => Rect {
            row_start: result.row_start + offset,
            row_end: result.row_start + offset,
            whole_rows: false,
            ..result
        },
        XLookupOrientation::Horizontal => Rect {
            col_start: result.col_start + offset,
            col_end: result.col_start + offset,
            ..result
        },
    }
}

fn xlookup_return_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    slice: Rect,
) -> Result<Array, ErrorKind> {
    let rows = u32::try_from(slice.height()).map_err(|_| ErrorKind::Num)?;
    let cols = u32::try_from(slice.width()).map_err(|_| ErrorKind::Num)?;
    let cells = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    let mut stored = engine.rect_cells(&slice, slice.row_end);
    for index in 0..cells {
        if index % 256 == 0 {
            super::array_common::poll_cancellation(context)?;
        }
        let row =
            slice.row_start + u32::try_from(index / u64::from(cols)).map_err(|_| ErrorKind::Num)?;
        let col =
            slice.col_start + u32::try_from(index % u64::from(cols)).map_err(|_| ErrorKind::Num)?;
        data.push(engine.read_stored_reference_cell(
            context,
            (slice.sheet, row, col),
            stored.at(row, col),
        )?);
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
    // A reference keeps its lookup index; an array constant or a computed array is searched as it
    // is (`VLOOKUP(2,{1,"a";2,"b"},2,FALSE)` is "b", Mac Excel 16.114).
    let table = match engine.resolve_rect_expr(context, &args[1]) {
        Ok(rect) => LookupTable::Rect(rect),
        Err(kind) if kind.is_engine_issue() => return Value::Error(kind),
        Err(_) => match engine.eval_array(context, &args[1]) {
            Ok(array) => LookupTable::Array(array),
            Err(kind) => return Value::Error(kind),
        },
    };
    let result_index = match required_number(engine, context, &args[2]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let result_limit = match (&table, horizontal) {
        (LookupTable::Rect(rect), true) => rect.height(),
        (LookupTable::Rect(rect), false) => rect.width(),
        (LookupTable::Array(array), true) => u64::from(array.rows),
        (LookupTable::Array(array), false) => u64::from(array.cols),
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
    let result_index = result_index as u32 - 1;
    match table {
        LookupTable::Rect(rect) => {
            match find_lookup_offset(engine, context, &lookup, rect, horizontal, approximate) {
                Ok(offset) => {
                    let (row, column) = if horizontal {
                        (rect.row_start + result_index, rect.col_start + offset)
                    } else {
                        (rect.row_start + offset, rect.col_start + result_index)
                    };
                    engine
                        .read_reference_cell(context, (rect.sheet, row, column))
                        .unwrap_or_else(Value::Error)
                }
                Err(kind) => Value::Error(kind),
            }
        }
        LookupTable::Array(array) => {
            let length = if horizontal { array.cols } else { array.rows };
            let key = |offset: u32| {
                if horizontal {
                    array.at(0, offset).clone()
                } else {
                    array.at(offset, 0).clone()
                }
            };
            let found = if approximate {
                scan_equal_lookup(&lookup, length, |offset| Ok(key(offset))).or_else(|kind| {
                    match kind {
                        ErrorKind::NA => {
                            scan_approximate_lookup(&lookup, length, |offset| Ok(key(offset))).0
                        }
                        kind => Err(kind),
                    }
                })
            } else {
                let mut criteria_runtime = CriteriaRuntime::new(engine, context);
                exact_lookup_pattern(&mut criteria_runtime, &lookup).and_then(|pattern| {
                    scan_exact_lookup(
                        &mut criteria_runtime,
                        pattern.as_ref(),
                        &lookup,
                        length,
                        |offset| Ok(key(offset)),
                    )
                })
            };
            match found {
                Ok(offset) if horizontal => array.at(result_index, offset).clone(),
                Ok(offset) => array.at(offset, result_index).clone(),
                Err(kind) => Value::Error(kind),
            }
        }
    }
}

/// The table `VLOOKUP` and `HLOOKUP` search: a reference, or an array of values.
enum LookupTable {
    Rect(Rect),
    Array(Array),
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

    if !approximate {
        return find_exact_lookup_offset(engine, context, lookup, rect, horizontal, length);
    }
    // Approximate searches pass over error cells as well, as Excel does. The first pass finds an
    // equal cell, which an exact index answers when the lookup is not wildcard text.
    let area = LookupArea::new(rect, length, horizontal);
    match area.and_then(|area| indexed_lookup_offset(engine, context, area, lookup)) {
        Some(Err(ErrorKind::NA)) => {}
        Some(found) => return found,
        None => {
            let mut stored = engine.rect_cells(&rect, rect.row_end);
            let found = scan_equal_lookup(lookup, length as u32, |offset| {
                lookup_axis_value(engine, context, rect, horizontal, offset, &mut stored)
            });
            if let Some(area) = area {
                record_linear_search(engine, area, SearchDirection::Forward, &found);
            }
            if !matches!(found, Err(ErrorKind::NA)) {
                return found;
            }
        }
    }

    if let Some(area) = area
        && let Some(found) = indexed_approximate_lookup_offset(engine, context, area, lookup)
    {
        return found;
    }
    let mut stored = engine.rect_cells(&rect, rect.row_end);
    let (candidate, stop) = scan_approximate_lookup(lookup, length as u32, |offset| {
        lookup_axis_value(engine, context, rect, horizontal, offset, &mut stored)
    });
    if let Some(area) = area
        && !matches!(candidate, Err(kind) if kind != ErrorKind::NA)
    {
        let reads = stop.ok_or(ErrorKind::NA);
        record_linear_search(engine, area, SearchDirection::Forward, &reads);
    }
    candidate
}

/// The first value equal to `lookup` among `length` values, passing over errors.
fn scan_equal_lookup(
    lookup: &Value,
    length: u32,
    mut value_at: impl FnMut(u32) -> Result<Value, ErrorKind>,
) -> Result<u32, ErrorKind> {
    for offset in 0..length {
        let value = value_at(offset)?;
        if !matches!(value, Value::Error(_)) && compare(&value, lookup)? == Ordering::Equal {
            return Ok(offset);
        }
    }
    Err(ErrorKind::NA)
}

/// The last value below `lookup` before a value above it, passing over errors, as an approximate
/// `VLOOKUP` finds once no value equals it; also the offset where the scan stopped.
fn scan_approximate_lookup(
    lookup: &Value,
    length: u32,
    mut value_at: impl FnMut(u32) -> Result<Value, ErrorKind>,
) -> (Result<u32, ErrorKind>, Option<u32>) {
    let mut candidate = None;
    for offset in 0..length {
        let value = match value_at(offset) {
            Ok(value) => value,
            Err(kind) => return (Err(kind), None),
        };
        if matches!(value, Value::Error(_)) {
            continue;
        }
        match compare(&value, lookup) {
            Ok(Ordering::Equal) => unreachable!("exact matches returned in the first pass"),
            Ok(Ordering::Less) => candidate = Some(offset),
            Ok(Ordering::Greater) => return (candidate.ok_or(ErrorKind::NA), Some(offset)),
            Err(kind) => return (Err(kind), None),
        }
    }
    (candidate.ok_or(ErrorKind::NA), None)
}

/// Finds an exact VLOOKUP or HLOOKUP match. Error cells never equal the lookup value, and a text
/// lookup value containing `?`, `*`, or `~` is a wildcard pattern, as in MATCH.
fn find_exact_lookup_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lookup: &Value,
    rect: Rect,
    horizontal: bool,
    length: u64,
) -> Result<u32, ErrorKind> {
    let area = LookupArea::new(rect, length, horizontal);
    if let Some(area) = area
        && let Some(found) = indexed_lookup_offset(engine, context, area, lookup)
    {
        return found;
    }
    let mut criteria_runtime = CriteriaRuntime::new(engine, context);
    let pattern = exact_lookup_pattern(&mut criteria_runtime, lookup)?;
    let mut stored = engine.rect_cells(&rect, rect.row_end);
    let found = scan_exact_lookup(
        &mut criteria_runtime,
        pattern.as_ref(),
        lookup,
        length as u32,
        |offset| lookup_axis_value(engine, context, rect, horizontal, offset, &mut stored),
    );
    if let Some(area) = area
        && pattern.is_none()
    {
        record_linear_search(engine, area, SearchDirection::Forward, &found);
    }
    found
}

/// The wildcard pattern a text lookup value containing `?`, `*`, or `~` stands for in an exact
/// lookup, as in MATCH.
fn exact_lookup_pattern(
    criteria_runtime: &mut CriteriaRuntime<'_, '_, '_>,
    lookup: &Value,
) -> Result<Option<CompiledWildcardPattern>, ErrorKind> {
    match lookup {
        Value::Text(text) if text.contains(['?', '*', '~']) => {
            criteria_runtime.compile_wildcard(text).map(Some)
        }
        _ => Ok(None),
    }
}

/// The first of `length` values an exact lookup matches: never an error, and text matching
/// `pattern` when the lookup value is a wildcard pattern.
fn scan_exact_lookup(
    criteria_runtime: &mut CriteriaRuntime<'_, '_, '_>,
    pattern: Option<&CompiledWildcardPattern>,
    lookup: &Value,
    length: u32,
    mut value_at: impl FnMut(u32) -> Result<Value, ErrorKind>,
) -> Result<u32, ErrorKind> {
    for offset in 0..length {
        let value = value_at(offset)?;
        let matched = match (pattern, &value) {
            (_, Value::Error(_)) => false,
            (Some(pattern), Value::Text(text)) => {
                criteria_runtime.wildcard_matches(pattern, text)?
            }
            (Some(_), _) => false,
            (None, _) => compare(&value, lookup)? == Ordering::Equal,
        };
        if matched {
            return Ok(offset);
        }
    }
    Err(ErrorKind::NA)
}

/// Reads the cell `offset` along the lookup axis, through `stored`, a walk over `rect`.
fn lookup_axis_value<'workbook>(
    engine: &Engine<'workbook>,
    context: EvalContext<'_>,
    rect: Rect,
    horizontal: bool,
    offset: u32,
    stored: &mut RectCells<'workbook>,
) -> Result<Value, ErrorKind> {
    let cell = if horizontal {
        (rect.sheet, rect.row_start, rect.col_start + offset)
    } else {
        (rect.sheet, rect.row_start + offset, rect.col_start)
    };
    engine.read_stored_reference_cell(context, cell, stored.at(cell.1, cell.2))
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

/// `CHOOSE` in an array context: an index that holds an array picks per element, and the picked
/// values are arrays (`CHOOSE(2,{1,2},{3,4})` is `{3,4}`, Mac Excel 16.114).
pub(super) fn choose_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 {
        return Err(ErrorKind::Value);
    }
    let index = match engine.eval_array(context, &args[0]) {
        Ok(index) => index,
        Err(kind) if kind.is_engine_issue() => return Err(kind),
        Err(kind) => Array::scalar(Value::Error(kind)),
    };
    let numeric_text = engine.numeric_text();
    super::elementwise::select(engine, context, args, &[index], |elements| match to_number(
        elements[0],
        numeric_text,
    ) {
        Ok(index) if index >= 1.0 && (index.trunc() as usize) < args.len() => {
            Pick::Argument(index.trunc() as usize)
        }
        Ok(_) => Pick::Value(Value::Error(ErrorKind::Value)),
        Err(kind) => Pick::Value(Value::Error(kind)),
    })
}

fn dimension(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], rows: bool) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let (height, width) = match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => (rect.height(), rect.width()),
        Err(kind) if kind.is_engine_issue() => return Value::Error(kind),
        // An array or a single value has its own dimensions: `ROWS({1;2;3})` is 3 and `ROWS(1)` is
        // 1 (Mac Excel 16.114).
        Err(_) => match engine.eval_array(context, &args[0]) {
            Ok(array) => (u64::from(array.rows), u64::from(array.cols)),
            Err(kind) => return Value::Error(kind),
        },
    };
    Value::Number(if rows { height } else { width } as f64)
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
