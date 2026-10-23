use super::super::ast::{Expr, column_label};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::CellId;
use super::super::value::{ErrorKind, Value};
use super::util::required_text;

const MAX_DISPLAYED_FORMULA_CHARACTERS: usize = 8_192;

pub(super) fn formula_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [reference] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let cell = match reference_cell(engine, context, reference) {
        Ok(cell) => cell,
        Err(kind) => return Value::Error(kind),
    };
    let Some(text) = engine.cell_formula_text(cell) else {
        return Value::Error(ErrorKind::NA);
    };
    if text.encode_utf16().count().saturating_add(1) > MAX_DISPLAYED_FORMULA_CHARACTERS {
        return Value::Error(ErrorKind::NA);
    }
    let mut output = String::with_capacity(text.len().saturating_add(1));
    output.push('=');
    output.push_str(text);
    engine.bounded_text(output)
}

pub(super) fn is_formula(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [reference] = args else {
        return Value::Error(ErrorKind::Value);
    };
    match reference_cell(engine, context, reference) {
        Ok(cell) => Value::Logical(engine.cell_has_formula(cell)),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn sheet(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let sheet_number =
        |sheet: usize| Value::Number(engine.workbook_tab_position(sheet) as f64 + 1.0);
    let Some(value) = args.first() else {
        return sheet_number(context.sheet());
    };
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.resolve_reference_value_expr(context, value) {
        Ok(reference) => match reference.single_rect() {
            Ok(rect) => sheet_number(rect.sheet),
            Err(kind) => Value::Error(kind),
        },
        Err(kind) if kind.is_engine_issue() => Value::Error(kind),
        Err(_) => match engine.eval_scalar(context, value) {
            Value::Text(name) => engine
                .workbook_tab_position_by_name(&name)
                .map_or(Value::Error(ErrorKind::NA), |position| {
                    Value::Number(position as f64 + 1.0)
                }),
            Value::Error(kind) => Value::Error(kind),
            _ => Value::Error(ErrorKind::NA),
        },
    }
}

pub(super) fn sheets(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let count = match args {
        [] => engine.workbook_tab_count(),
        [reference] => match engine.resolve_reference_value_expr(context, reference) {
            Ok(reference) => match reference.single_area_span() {
                Ok(span) => {
                    let (first, last) = span.sheet_bounds();
                    engine.workbook_tab_position(last) - engine.workbook_tab_position(first) + 1
                }
                Err(kind) if kind.is_engine_issue() => return Value::Error(kind),
                Err(_) => return Value::Error(ErrorKind::Ref),
            },
            Err(kind) if kind.is_engine_issue() => return Value::Error(kind),
            Err(_) => return Value::Error(ErrorKind::Ref),
        },
        _ => return Value::Error(ErrorKind::Value),
    };
    match u32::try_from(count) {
        Ok(count) => Value::Number(f64::from(count)),
        Err(_) => Value::Error(ErrorKind::Num),
    }
}

/// CELL(info_type, reference) for the info types Microsoft defines by the workbook alone:
/// `"address"` and `"contents"` of the upper-left cell, and `"col"`, `"row"`, and `"type"` of a
/// single cell. The other info types read formatting, column widths, or the file path, an omitted
/// reference reads the selection, and the address of another sheet names the file, so those are
/// unsupported, as are a multi-cell `"col"`, `"row"`, or `"type"` (Microsoft documents the active
/// cell), a blank `"contents"`, a formula with a text result for `"type"`, and info_type spellings
/// other than these lowercase ones.
pub(super) fn cell(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [info_type, reference] = args else {
        return Value::Error(ErrorKind::Unsupported);
    };
    let info_type = match required_text(engine, context, info_type) {
        Ok(info_type) => info_type,
        Err(kind) => return Value::Error(kind),
    };
    let rect = match engine
        .resolve_reference_value_expr(context, reference)
        .and_then(|reference| reference.single_rect())
    {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let first = (rect.sheet, rect.row_start, rect.col_start);
    let single = rect.is_single_cell();
    match info_type.as_str() {
        "address" if rect.sheet == context.sheet() => engine.bounded_text(format!(
            "${}${}",
            column_label(rect.col_start),
            rect.row_start
        )),
        "col" if single => Value::Number(f64::from(rect.col_start)),
        "row" if single => Value::Number(f64::from(rect.row_start)),
        "contents" => match engine.read_reference_cell(context, first) {
            Ok(Value::Blank) => Value::Error(ErrorKind::Unsupported),
            Ok(value) => value,
            Err(kind) => Value::Error(kind),
        },
        "type" if single => match engine.read_reference_cell(context, first) {
            Ok(value) if engine.cell_has_formula(first) => match value {
                Value::Blank | Value::Text(_) => Value::Error(ErrorKind::Unsupported),
                _ => Value::Text("v".to_owned()),
            },
            Ok(Value::Blank) => Value::Text("b".to_owned()),
            Ok(Value::Text(_)) => Value::Text("l".to_owned()),
            Ok(_) => Value::Text("v".to_owned()),
            Err(kind) => Value::Error(kind),
        },
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn reference_cell(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    reference: &Expr,
) -> Result<CellId, ErrorKind> {
    let rect = engine
        .resolve_reference_value_expr(context, reference)?
        .single_rect()?;
    Ok((rect.sheet, rect.row_start, rect.col_start))
}
