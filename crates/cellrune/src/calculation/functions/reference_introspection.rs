use super::super::ast::{Expr, column_label};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::CellId;
use super::super::value::{ErrorKind, Value};
use super::cell_format_info::format_code;
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

/// CELL(info_type, reference) for the info types defined by the workbook alone, as Microsoft
/// documents them and Excel 16.114 for Mac returns them: `"address"` of the upper-left cell as
/// `$A$1` text when the reference is on the formula's sheet, `"contents"` (the upper-left cell's
/// value, 0 for an empty cell), `"row"` of the upper-left cell, for a single cell `"col"` and
/// `"type"` (`"b"` for an empty cell, `"l"` for text, including a formula's text result, and `"v"`
/// otherwise), and for the upper-left cell's number format `"format"` (its documented code, such
/// as `"F2"` or `"D4"`), `"color"` (1 for a code that shows negative values in color, else 0), and
/// `"parentheses"` (0, since no documented code shows positive values in parentheses). The info
/// type is matched without regard to ASCII case.
///
/// The other info types read the column width, alignment, protection, or file path, which are not
/// calculation inputs, an omitted or empty reference reads the selection at calculation time, and
/// Excel prefixes the address of another sheet with the file name, so those are unsupported. So
/// are a multi-cell `"col"` or `"type"`, which Excel was not measured on, the `"type"` of a formula
/// with a blank result, and the format of an empty position, whose row or column style is not a
/// calculation input, or of a format Microsoft does not document a code for.
pub(super) fn cell(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [info_type, reference] = args else {
        return Value::Error(ErrorKind::Unsupported);
    };
    if matches!(reference, Expr::Missing) {
        return Value::Error(ErrorKind::Unsupported);
    }
    let info_type = match required_text(engine, context, info_type) {
        Ok(info_type) => info_type.to_ascii_lowercase(),
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
        "row" => Value::Number(f64::from(rect.row_start)),
        "contents" => match engine.read_reference_cell(context, first) {
            Ok(Value::Blank) => Value::Number(0.0),
            Ok(value) => value,
            Err(kind) => Value::Error(kind),
        },
        "type" if single => match engine.read_reference_cell(context, first) {
            Ok(Value::Blank) if engine.cell_has_formula(first) => {
                Value::Error(ErrorKind::Unsupported)
            }
            Ok(Value::Blank) => Value::Text("b".to_owned()),
            Ok(Value::Text(_)) => Value::Text("l".to_owned()),
            Ok(_) => Value::Text("v".to_owned()),
            Err(kind) => Value::Error(kind),
        },
        "format" | "color" | "parentheses" => {
            let Some(code) = engine.cell_number_format(first).and_then(format_code) else {
                return Value::Error(ErrorKind::Unsupported);
            };
            match info_type.as_str() {
                "format" => Value::Text(code.to_owned()),
                "color" => Value::Number(if code.ends_with('-') { 1.0 } else { 0.0 }),
                _ => Value::Number(0.0),
            }
        }
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
