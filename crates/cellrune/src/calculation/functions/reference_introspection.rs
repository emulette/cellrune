use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::CellId;
use super::super::value::{ErrorKind, Value};

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
    let Some(value) = args.first() else {
        return Value::Number(context.sheet() as f64 + 1.0);
    };
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.resolve_reference_value_expr(context, value) {
        Ok(reference) => match reference.single_rect() {
            Ok(rect) => Value::Number(rect.sheet as f64 + 1.0),
            Err(kind) => Value::Error(kind),
        },
        Err(kind) if kind.is_engine_issue() => Value::Error(kind),
        Err(_) => match engine.eval_scalar(context, value) {
            Value::Text(name) => engine
                .workbook_sheet_index(&name)
                .map_or(Value::Error(ErrorKind::NA), |index| {
                    Value::Number(index as f64 + 1.0)
                }),
            Value::Error(kind) => Value::Error(kind),
            _ => Value::Error(ErrorKind::NA),
        },
    }
}

pub(super) fn sheets(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let count = match args {
        [] => engine.workbook_sheet_count(),
        [reference] => match engine.resolve_reference_value_expr(context, reference) {
            Ok(reference) => match reference.single_area_span() {
                Ok(span) => span.sheet_count(),
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
