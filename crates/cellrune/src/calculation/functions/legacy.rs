use super::super::ast::Expr;
use super::super::coerce::{logical_text, to_logical, to_number, to_text};
use super::super::criteria::CompiledCriteria;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::operators::{broadcast_shape, element_at};
use super::super::runtime::{Array, Rect};
use super::super::textfmt::{format_number, format_text};
use super::super::value::{ErrorKind, Value};
use super::criteria_index::indexed_matches;
use super::criteria_runtime::{ConditionalScanBound, CriteriaRuntime};
use super::kernel::{LegacyArrayFunction, LegacyFunction};
use super::lookup_index::{
    LookupArea, SearchDirection, indexed_approximate_match_offset, indexed_match_offset,
    record_linear_search,
};
use super::util::ExcelSum;

pub(super) fn call_legacy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: LegacyFunction,
    args: &[Expr],
) -> Value {
    match function {
        LegacyFunction::If => kernel_if(engine, context, args),
        LegacyFunction::And => kernel_and(engine, context, args),
        LegacyFunction::IfError => kernel_iferror(engine, context, args),
        LegacyFunction::Lower => kernel_lower(engine, context, args),
        LegacyFunction::Text => kernel_text(engine, context, args),
        LegacyFunction::CountIf => kernel_countifs(engine, context, args, true),
        LegacyFunction::CountIfs => kernel_countifs(engine, context, args, false),
        LegacyFunction::SumProduct => kernel_sumproduct(engine, context, args),
        LegacyFunction::Index => kernel_index(engine, context, args),
        LegacyFunction::Match => kernel_match(engine, context, args),
        LegacyFunction::DummyFunction if args.len() == 1 => Value::Error(ErrorKind::Name),
        LegacyFunction::DummyFunction => Value::Error(ErrorKind::Value),
    }
}

pub(super) fn call_legacy_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: LegacyArrayFunction,
    args: &[Expr],
) -> Option<Result<Array, ErrorKind>> {
    match function {
        LegacyArrayFunction::If => Some(if_array(engine, context, args)),
        LegacyArrayFunction::IfError => Some(if_error_array(engine, context, args, false)),
        LegacyArrayFunction::IfNa => Some(if_error_array(engine, context, args, true)),
        LegacyArrayFunction::Index => Some(index_array(engine, context, args)),
    }
}

fn if_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let condition = engine.eval_array(context, &args[0])?;
    let mut needs_true = false;
    let mut needs_false = false;
    for value in &condition.data {
        match to_logical(value) {
            Ok(true) => needs_true = true,
            Ok(false) => needs_false = true,
            Err(_) => {}
        }
    }
    let when_true = if needs_true {
        Some(match args.get(1) {
            Some(Expr::Missing) => Array::scalar(Value::Number(0.0)),
            Some(expr) => engine.eval_array(context, expr)?,
            None => return Err(ErrorKind::Value),
        })
    } else {
        None
    };
    let when_false = if needs_false {
        Some(match args.get(2) {
            Some(Expr::Missing) => Array::scalar(Value::Number(0.0)),
            Some(expr) => engine.eval_array(context, expr)?,
            None => Array::scalar(Value::Logical(false)),
        })
    } else {
        None
    };
    let mut shape = condition.clone();
    for branch in [&when_true, &when_false].into_iter().flatten() {
        shape = shape_array(&shape, branch)?;
    }
    let rows = shape.rows;
    let cols = shape.cols;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    let capacity = usize::try_from(cells)
        .map_err(|_| ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    let mut data = Vec::with_capacity(capacity);
    for row in 0..rows {
        for column in 0..cols {
            let selected = match to_logical(element_at(&condition, row, column)) {
                Ok(true) => when_true
                    .as_ref()
                    .map(|branch| element_at(branch, row, column).clone())
                    .unwrap_or(Value::Error(ErrorKind::Calc)),
                Ok(false) => when_false
                    .as_ref()
                    .map(|branch| element_at(branch, row, column).clone())
                    .unwrap_or(Value::Error(ErrorKind::Calc)),
                Err(kind) => Value::Error(kind),
            };
            data.push(selected);
        }
    }
    Ok(Array { rows, cols, data })
}

/// `IFERROR`, or `IFNA` with `only_na`, over arrays: each element of the value that is an error,
/// or `#N/A` for `IFNA`, takes the fallback's element, so `{=SUM(IFERROR(1/A1:A3,0))}` adds the
/// reciprocals that exist. The fallback is evaluated for an array value, which takes its shape,
/// and for a single value only when that value is caught; an error evaluating it is its value.
fn if_error_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    only_na: bool,
) -> Result<Array, ErrorKind> {
    let [value, fallback] = args else {
        return Err(ErrorKind::Value);
    };
    let caught = |value: &Value| match value {
        Value::Error(kind) => !kind.is_engine_issue() && (!only_na || *kind == ErrorKind::NA),
        _ => false,
    };
    let value = match engine.eval_array(context, value) {
        Ok(value) => value,
        Err(kind) if kind.is_engine_issue() => return Err(kind),
        Err(kind) => Array::scalar(Value::Error(kind)),
    };
    if let Some(Value::Error(kind)) = value
        .data
        .iter()
        .find(|element| matches!(element, Value::Error(kind) if kind.is_engine_issue()))
    {
        return Err(*kind);
    }
    // An array value broadcasts against the fallback even when it holds no error (Mac Excel 16.114
    // `IFERROR({1,2},{7;8})` is 2x2), while a single value that is not caught stands alone.
    if value.is_scalar() && !value.data.iter().any(caught) {
        return Ok(value);
    }
    let fallback = match engine.eval_array(context, fallback) {
        Ok(fallback) => fallback,
        Err(kind) if kind.is_engine_issue() => return Err(kind),
        Err(kind) => Array::scalar(Value::Error(kind)),
    };
    let (rows, cols) = broadcast_shape(&value, &fallback)?;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for column in 0..cols {
            let element = element_at(&value, row, column);
            data.push(if caught(element) {
                element_at(&fallback, row, column).clone()
            } else {
                element.clone()
            });
        }
    }
    Ok(Array { rows, cols, data })
}

fn shape_array(left: &Array, right: &Array) -> Result<Array, ErrorKind> {
    let (rows, cols) = broadcast_shape(left, right)?;
    Ok(Array {
        rows,
        cols,
        data: Vec::new(),
    })
}

fn reference_rect(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<Rect, ErrorKind> {
    engine.resolve_rect_expr(context, expr)
}

fn multi_cell_rect(engine: &Engine<'_>, context: EvalContext<'_>, expr: &Expr) -> Option<Rect> {
    reference_rect(engine, context, expr)
        .ok()
        .filter(|rect| !rect.is_single_cell())
}

fn kernel_if(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let condition = engine.eval_scalar(context, &args[0]);
    match to_logical(&condition) {
        Ok(true) => eval_if_branch(engine, context, args.get(1), Value::Logical(false)),
        Ok(false) => eval_if_branch(engine, context, args.get(2), Value::Logical(false)),
        Err(kind) => Value::Error(kind),
    }
}

fn eval_if_branch(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    branch: Option<&Expr>,
    omitted: Value,
) -> Value {
    match branch {
        Some(Expr::Missing) => Value::Number(0.0),
        Some(branch) => engine.eval_scalar(context, branch),
        None => omitted,
    }
}

fn kernel_and(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let mut participants = 0_u64;
    let mut all_true = true;
    let mut visited_cells = 0_u64;
    for arg in args {
        if let Some(rect) = multi_cell_rect(engine, context, arg) {
            let rows = engine.operation_row_count([&rect]);
            if rows > 0 {
                let cells = rows.checked_mul(rect.width());
                visited_cells = match cells.and_then(|cells| visited_cells.checked_add(cells)) {
                    Some(total) => total,
                    None => {
                        return Value::Error(ErrorKind::ResourceLimit(
                            CalculationLimitKind::ArrayCells,
                        ));
                    }
                };
                if let Err(kind) = engine.ensure_array_cells(visited_cells) {
                    return Value::Error(kind);
                }
            }
            let mut stored =
                engine.rect_cells(&rect, (rect.row_start + rows as u32).saturating_sub(1));
            for row_offset in 0..rows as u32 {
                let row = rect.row_start + row_offset;
                for column in rect.col_start..=rect.col_end {
                    match engine.read_stored_reference_cell(
                        context,
                        (rect.sheet, row, column),
                        stored.at(row, column),
                    ) {
                        Err(kind) => return Value::Error(kind),
                        Ok(Value::Error(kind)) => return Value::Error(kind),
                        Ok(Value::Number(number)) => {
                            participants += 1;
                            all_true &= number != 0.0;
                        }
                        Ok(Value::Logical(logical)) => {
                            participants += 1;
                            all_true &= logical;
                        }
                        Ok(Value::Text(_) | Value::Blank) => {}
                    }
                }
            }
            continue;
        }
        let is_reference = reference_rect(engine, context, arg).is_ok();
        let value = if !is_reference && super::elementwise::may_hold_array(engine, context, arg) {
            match engine.eval_array(context, arg) {
                // An array's elements take part as a range's cells do: `AND({TRUE,FALSE})` is
                // `FALSE` (Mac Excel 16.114).
                Ok(array) if !array.is_scalar() => {
                    for value in array.data {
                        match value {
                            Value::Error(kind) => return Value::Error(kind),
                            Value::Number(number) => {
                                participants += 1;
                                all_true &= number != 0.0;
                            }
                            Value::Logical(logical) => {
                                participants += 1;
                                all_true &= logical;
                            }
                            Value::Text(_) | Value::Blank => {}
                        }
                    }
                    continue;
                }
                Ok(array) => array.data.into_iter().next().unwrap_or(Value::Blank),
                Err(kind) => return Value::Error(kind),
            }
        } else {
            engine.eval_scalar(context, arg)
        };
        if is_reference {
            // Text in a referenced cell takes no part, `TRUE` included, as in a range: with A1
            // holding the text `x`, `AND(A1,TRUE)` is `TRUE` and `AND(A1)` is `#VALUE!` (Mac Excel
            // 16.114).
            if matches!(&value, Value::Blank | Value::Text(_)) {
                continue;
            }
            match to_logical(&value) {
                Ok(logical) => {
                    participants += 1;
                    all_true &= logical;
                }
                Err(kind) => return Value::Error(kind),
            }
            continue;
        }
        match to_logical_aggregate_argument(&value) {
            Ok(Some(logical)) => {
                participants += 1;
                all_true &= logical;
            }
            Ok(None) => {}
            Err(kind) => return Value::Error(kind),
        }
    }
    if participants == 0 {
        return Value::Error(ErrorKind::Value);
    }
    Value::Logical(all_true)
}

/// Reads a direct argument of `AND`, `OR`, or `XOR`: text other than `TRUE` and `FALSE` takes no
/// part, whether typed or computed, so `AND("0",1)` and `AND("x",1)` are `TRUE` (Mac Excel 16.114).
fn to_logical_aggregate_argument(value: &Value) -> Result<Option<bool>, ErrorKind> {
    match value {
        Value::Text(text) => Ok(logical_text(text)),
        _ => to_logical(value).map(Some),
    }
}

fn kernel_iferror(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let value = engine.eval_scalar(context, &args[0]);
    match value {
        Value::Error(kind) if kind.is_engine_issue() => Value::Error(kind),
        Value::Error(_) => engine.eval_scalar(context, &args[1]),
        other => other,
    }
}

fn kernel_lower(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let value = engine.eval_scalar(context, &args[0]);
    match to_text(&value) {
        Ok(text) => engine.bounded_text(text.to_lowercase()),
        Err(kind) => Value::Error(kind),
    }
}

/// `TEXT`: text that reads as a number formats as that number, and other text and logical values
/// format as text, `TRUE` and `FALSE` included (`TEXT("abc","0")` is `abc` and `TEXT(TRUE,"0")`
/// is `TRUE`, Mac Excel 16.114).
fn kernel_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let value = engine.eval_scalar(context, &args[0]);
    // The value's error comes before the format's: `TEXT(1/0,NA())` is `#DIV/0!` (Mac Excel 16.114).
    if let Value::Error(kind) = value {
        return Value::Error(kind);
    }
    let format = match to_text(&engine.eval_scalar(context, &args[1])) {
        Ok(format) => format,
        Err(kind) => return Value::Error(kind),
    };
    let format_context = engine.format_context();
    let numeric_text = engine.numeric_text();
    let formatted = match &value {
        Value::Text(text) => match numeric_text.number(text) {
            Some(number) => format_number(number, &format, format_context),
            // Showing text Excel reads as a number unchanged would be a wrong answer.
            None if numeric_text.may_read_otherwise(text) => Err(ErrorKind::Unsupported),
            None => format_text(text, &format, format_context),
        },
        Value::Logical(logical) => format_text(
            if *logical { "TRUE" } else { "FALSE" },
            &format,
            format_context,
        ),
        _ => to_number(&value, engine.numeric_text())
            .and_then(|number| format_number(number, &format, format_context)),
    };
    match formatted {
        Ok(text) => engine.bounded_text(text),
        Err(kind) => Value::Error(kind),
    }
}

struct CriteriaPairs<'formula> {
    rects: Vec<Rect>,
    criteria_exprs: Vec<&'formula Expr>,
}

fn split_pairs<'formula>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &'formula [Expr],
    single_pair: bool,
) -> Result<CriteriaPairs<'formula>, ErrorKind> {
    let expected_len_ok = if single_pair {
        args.len() == 2
    } else {
        args.len() >= 2 && args.len().is_multiple_of(2)
    };
    if !expected_len_ok {
        return Err(ErrorKind::Value);
    }
    let mut rects = Vec::new();
    let mut criteria_exprs = Vec::new();
    for pair in args.chunks(2) {
        let rect = reference_rect(engine, context, &pair[0])?;
        rects.push(rect);
        criteria_exprs.push(&pair[1]);
    }
    let first = rects[0];
    for rect in &rects {
        if rect.height() != first.height() || rect.width() != first.width() {
            return Err(ErrorKind::Value);
        }
    }
    Ok(CriteriaPairs {
        rects,
        criteria_exprs,
    })
}

fn count_matches(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    rects: &[Rect],
    criteria: &[CompiledCriteria],
    matcher: &mut CriteriaRuntime<'_, '_, '_>,
) -> Result<f64, ErrorKind> {
    let height = rects[0].height();
    let width = rects[0].width();
    let iter_rows = engine.operation_row_count(rects);
    let bound = ConditionalScanBound::new(engine, iter_rows, width, rects.len());
    let count = scan_matches(engine, context, rects, criteria, iter_rows, matcher)
        .map_err(|kind| bound.error(kind))?;
    let virtual_cells = (height - iter_rows) * width;
    if virtual_cells > 0 && criteria.iter().all(CompiledCriteria::matches_blank) {
        return Ok((count + virtual_cells) as f64);
    }
    Ok(count as f64)
}

fn scan_matches(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    rects: &[Rect],
    criteria: &[CompiledCriteria],
    iter_rows: u64,
    matcher: &mut CriteriaRuntime<'_, '_, '_>,
) -> Result<u64, ErrorKind> {
    let width = rects[0].width();
    if let Some(matches) = indexed_matches(
        engine,
        context,
        rects.iter().zip(criteria),
        iter_rows,
        width,
    ) && let Some(charged) = matches.charge(engine, context, 0)
    {
        return charged.map(|()| matches.offsets.len() as u64);
    }
    let mut count = 0_u64;
    let mut stored = rects
        .iter()
        .map(|rect| engine.rect_cells(rect, (rect.row_start + iter_rows as u32).saturating_sub(1)))
        .collect::<Vec<_>>();
    for relative_row in 0..iter_rows {
        for relative_col in 0..width {
            let mut matched = true;
            for ((rect, criterion), stored) in rects.iter().zip(criteria.iter()).zip(&mut stored) {
                let (row, column) = (
                    rect.row_start + relative_row as u32,
                    rect.col_start + relative_col as u32,
                );
                let value = engine.read_stored_reference_cell(
                    context,
                    (rect.sheet, row, column),
                    stored.at(row, column),
                )?;
                if !matcher.matches(criterion, &value)? {
                    matched = false;
                    break;
                }
            }
            if matched {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn kernel_countifs(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    single_pair: bool,
) -> Value {
    let pairs = match split_pairs(engine, context, args, single_pair) {
        Ok(pairs) => pairs,
        Err(kind) => return Value::Error(kind),
    };
    let mut runtime = CriteriaRuntime::new(engine, context);
    let mut criteria = Vec::with_capacity(pairs.criteria_exprs.len());
    for expr in &pairs.criteria_exprs {
        let value = engine.eval_scalar(context, expr);
        match runtime.compile_criteria(&value) {
            Ok(criterion) => criteria.push(criterion),
            Err(kind) => return Value::Error(kind),
        }
    }
    match count_matches(engine, context, &pairs.rects, &criteria, &mut runtime) {
        Ok(count) => Value::Number(count),
        Err(kind) => Value::Error(kind),
    }
}

fn kernel_sumproduct(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let mut arrays = Vec::with_capacity(args.len());
    for arg in args {
        match engine.eval_array_evaluation(context, arg) {
            Ok(evaluated) => arrays.push(evaluated),
            Err(kind) => return Value::Error(kind),
        }
    }
    let rows = arrays[0].array.rows;
    let cols = arrays[0].array.cols;
    for evaluated in &arrays[1..] {
        if evaluated.array.rows != rows || evaluated.array.cols != cols {
            return Value::Error(ErrorKind::Value);
        }
    }
    let Some(cells) = u64::from(rows).checked_mul(u64::from(cols)) else {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    };
    if let Err(kind) = engine.ensure_array_cells(cells) {
        return Value::Error(kind);
    }
    // Excel does not correct a near-zero total of products: `SUMPRODUCT({0.1,0.2,-0.3})` keeps its
    // residue where `SUM` returns zero.
    let mut total = ExcelSum::plain();
    for row in 0..rows {
        for col in 0..cols {
            let mut product = 1.0_f64;
            for evaluated in &arrays {
                product *= match element_at(&evaluated.array, row, col) {
                    Value::Error(kind) => return Value::Error(*kind),
                    Value::Number(number) => *number,
                    // Excel treats a non-numeric cell as zero rather than as an error.
                    Value::Logical(_) | Value::Text(_) | Value::Blank => 0.0,
                };
            }
            total.add(product);
        }
    }
    Value::Number(total.total())
}

fn kernel_index(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match index_source(engine, context, args) {
        // A selection of several values takes its first, as a formula that is not an array
        // formula takes an array's.
        Ok(Some(values)) => select_from_values(engine, context, &values, args)
            .map_or_else(Value::Error, |selected| selected.data[0].clone()),
        Ok(None) => engine
            .resolve_index_rect(context, args)
            .and_then(|rect| engine.implicit_intersection_rect(context, rect))
            .and_then(|rect| {
                engine.read_reference_cell(context, (rect.sheet, rect.row_start, rect.col_start))
            })
            .unwrap_or_else(Value::Error),
        Err(kind) => Value::Error(kind),
    }
}

/// The values `INDEX` selects from when its first argument is not a reference, such as an array
/// constant or a computed array; `None` for a reference.
fn index_source(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Option<Array>, ErrorKind> {
    let Some(source) = args.first() else {
        return Err(ErrorKind::Value);
    };
    match engine.resolve_reference_value_expr(context, source) {
        Ok(_) => Ok(None),
        Err(kind) if kind.is_engine_issue() => Err(kind),
        Err(_) => engine.eval_array(context, source).map(Some),
    }
}

/// Selects from an array of values as `INDEX` does (Mac Excel 16.114): one index picks from a row
/// or a column (`INDEX({10,20,30},2)` is 20), and a two-dimensional array without a column picks a
/// whole row (`INDEX({10,20;30,40},2)` is `{30,40}`), where a reference of several rows and columns
/// needs its column. A zero index selects every row or column, and one past the array is `#REF!`.
fn select_from_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    values: &Array,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let index = |position: usize| -> Result<Option<f64>, ErrorKind> {
        match args.get(position) {
            None | Some(Expr::Missing) => Ok(None),
            Some(expr) => to_number(&engine.eval_scalar(context, expr), engine.numeric_text())
                .map(|number| Some(number.trunc())),
        }
    };
    let first = index(1)?.unwrap_or(0.0);
    let second = index(2)?;
    if index(3)?.is_some_and(|area| area != 1.0) {
        return Err(ErrorKind::Ref);
    }
    let (row, column) = match second {
        None if values.rows == 1 => (0.0, first),
        None if values.cols == 1 => (first, 0.0),
        None => (first, 0.0),
        Some(second) => (first, second),
    };
    if row < 0.0 || column < 0.0 {
        return Err(ErrorKind::Value);
    }
    if row > f64::from(values.rows) || column > f64::from(values.cols) {
        return Err(ErrorKind::Ref);
    }
    let rows = if row == 0.0 {
        0..values.rows
    } else {
        row as u32 - 1..row as u32
    };
    let columns = if column == 0.0 {
        0..values.cols
    } else {
        column as u32 - 1..column as u32
    };
    let data = rows
        .clone()
        .flat_map(|row| {
            columns
                .clone()
                .map(move |column| values.at(row, column).clone())
        })
        .collect();
    Ok(Array {
        rows: rows.len() as u32,
        cols: columns.len() as u32,
        data,
    })
}

/// Whether `expr` is an `INDEX` call whose row, column, or area index holds several values, which
/// selects an array of values rather than one reference.
pub(super) fn is_index_over_arrays(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> bool {
    match expr {
        Expr::Paren(inner) => is_index_over_arrays(engine, context, inner),
        Expr::Call { name, args }
            if context.binding(name).is_none()
                && engine
                    .resolve_name_expr_with_id_in_context(context, name)
                    .is_none()
                && super::function_evaluator(name)
                    == Some(super::Evaluator::Legacy(LegacyFunction::Index)) =>
        {
            index_has_arrays(engine, context, args)
        }
        _ => false,
    }
}

/// Whether a row, column, or area index of `INDEX` evaluates to more than one value.
pub(super) fn index_has_arrays(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> bool {
    super::elementwise::lifted_positions(engine, context, args, 1..args.len())
        .into_iter()
        .any(|index| {
            engine
                .eval_array(context, &args[index])
                .is_ok_and(|array| !array.is_scalar())
        })
}

/// `INDEX` in an array context. Row, column, or area indexes that hold arrays select one value per
/// element, as Mac Excel 16.114 returns `{11,12;31,32}` for `INDEX(Y1:Z3,{1;3},{1,2})`; an element
/// that selects a whole row or column takes its first cell (`INDEX(Y1:Z3,{1;3},0)` is `{11;31}`).
fn index_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let values = index_source(engine, context, args)?;
    let lifted = super::elementwise::lifted_positions(engine, context, args, 1..args.len());
    let lifting = super::elementwise::Lifting {
        lifted: &lifted,
        once: &[],
        only_several: true,
    };
    let selected =
        super::elementwise::lift(engine, context, args, args, &lifting, |context, args, _| {
            let selected = match &values {
                Some(values) => select_from_values(engine, context, values, args)
                    .map(|selected| selected.data[0].clone()),
                None => engine.resolve_index_rect(context, args).and_then(|rect| {
                    engine
                        .read_reference_cell(context, (rect.sheet, rect.row_start, rect.col_start))
                }),
            };
            selected.unwrap_or_else(Value::Error)
        })?;
    if let Some(selected) = selected {
        return Ok(selected);
    }
    match values {
        Some(values) => select_from_values(engine, context, &values, args),
        None => {
            let rect = engine.resolve_index_rect(context, args)?;
            engine.array_from_rect(context, &args[0], rect)
        }
    }
}

fn kernel_match(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    // A reference keeps its lookup index; an array constant or a computed array is searched as it
    // is, as `MATCH(1,(A1:A3="b")*(B1:B3=2),0)` searches in Excel.
    let source = match reference_rect(engine, context, &args[1]) {
        Ok(rect) => MatchSource::Rect(rect),
        Err(kind) if kind.is_engine_issue() => return Value::Error(kind),
        Err(_) => match engine.eval_array(context, &args[1]) {
            Ok(array) => MatchSource::Array(array),
            Err(kind) => return Value::Error(kind),
        },
    };
    // A lookup array of several rows and columns is `#N/A` (Mac Excel 16.114).
    let (vertical, length) = match &source {
        MatchSource::Rect(rect) if rect.height() > 1 && rect.width() > 1 => {
            return Value::Error(ErrorKind::NA);
        }
        MatchSource::Array(array) if array.rows > 1 && array.cols > 1 => {
            return Value::Error(ErrorKind::NA);
        }
        MatchSource::Rect(rect) => {
            let vertical = rect.width() == 1;
            let length = if vertical {
                engine.operation_row_count([rect])
            } else {
                rect.width()
            };
            (vertical, length)
        }
        MatchSource::Array(array) => (array.cols == 1, array.data.len() as u64),
    };
    // Excel selects the mode by sign, so any positive type behaves as 1 and any negative as -1.
    let match_type = match args.get(2) {
        Some(expr) => match to_number(&engine.eval_scalar(context, expr), engine.numeric_text()) {
            Ok(number) if number > 0.0 => 1.0,
            Ok(number) if number < 0.0 => -1.0,
            Ok(_) => 0.0,
            Err(kind) => return Value::Error(kind),
        },
        None => 1.0,
    };
    if lookup.is_blank_like() {
        return Value::Error(ErrorKind::NA);
    }
    let mut matcher = CriteriaRuntime::new(engine, context);
    let criterion = if match_type == 0.0 {
        match matcher.compile_exact_equality(&lookup) {
            Ok(Some(criterion)) => Some(criterion),
            Ok(None) => return Value::Error(ErrorKind::NA),
            Err(kind) => return Value::Error(kind),
        }
    } else {
        None
    };
    if let Err(kind) = engine.ensure_array_cells(length) {
        return Value::Error(kind);
    }
    let found = match source {
        MatchSource::Array(array) => {
            scan_match(
                &mut matcher,
                criterion.as_ref(),
                &lookup,
                match_type,
                length,
                |offset| Ok(array.data[offset as usize].clone()),
            )
            .found
        }
        MatchSource::Rect(rect) => {
            let search = MatchSearch {
                criterion: criterion.as_ref(),
                lookup: &lookup,
                match_type,
                length,
            };
            match_in_rect(engine, context, rect, vertical, &search, &mut matcher)
        }
    };
    match found {
        Ok(offset) => Value::Number(f64::from(offset + 1)),
        Err(kind) => Value::Error(kind),
    }
}

/// Where `MATCH` looks: a reference, or an array of values.
enum MatchSource {
    Rect(Rect),
    Array(Array),
}

/// What `MATCH` looks for, and how far.
struct MatchSearch<'search> {
    /// The exact equality of match type 0; `None` for the approximate types.
    criterion: Option<&'search CompiledCriteria>,
    lookup: &'search Value,
    match_type: f64,
    length: u64,
}

/// Searches a one-row or one-column reference, through its lookup index when it has one.
fn match_in_rect(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    rect: Rect,
    vertical: bool,
    search: &MatchSearch<'_>,
    matcher: &mut CriteriaRuntime<'_, '_, '_>,
) -> Result<u32, ErrorKind> {
    let area = LookupArea::new(rect, search.length, !vertical);
    let indexed = area.and_then(|area| match search.criterion {
        Some(_) => indexed_match_offset(engine, context, area, search.lookup),
        None => indexed_approximate_match_offset(
            engine,
            context,
            area,
            search.lookup,
            search.match_type == -1.0,
        ),
    });
    if let Some(found) = indexed {
        return found;
    }
    let mut stored = engine.rect_cells(&rect, rect.row_end);
    let scan = scan_match(
        matcher,
        search.criterion,
        search.lookup,
        search.match_type,
        search.length,
        |offset| {
            let cell = if vertical {
                (rect.sheet, rect.row_start + offset as u32, rect.col_start)
            } else {
                (rect.sheet, rect.row_start, rect.col_start + offset as u32)
            };
            engine.read_stored_reference_cell(context, cell, stored.at(cell.1, cell.2))
        },
    );
    if let Some(area) = area {
        record_linear_search(engine, area, SearchDirection::Forward, &scan.reads);
    }
    scan.found
}

/// The offset a linear `MATCH` finds, and how far it read, which the lookup statistics record.
struct MatchScan {
    found: Result<u32, ErrorKind>,
    reads: Result<u32, ErrorKind>,
}

/// Reads `length` values through `value_at` as `MATCH` does: the first equal value for match type
/// 0, otherwise the last value on the matching side before the order turns, passing over errors.
fn scan_match(
    matcher: &mut CriteriaRuntime<'_, '_, '_>,
    criterion: Option<&CompiledCriteria>,
    lookup: &Value,
    match_type: f64,
    length: u64,
    mut value_at: impl FnMut(u64) -> Result<Value, ErrorKind>,
) -> MatchScan {
    if let Some(criterion) = criterion {
        let found = (|| {
            for offset in 0..length {
                if matcher.matches(criterion, &value_at(offset)?)? {
                    return Ok(offset as u32);
                }
            }
            Err(ErrorKind::NA)
        })();
        return MatchScan {
            reads: found,
            found,
        };
    }
    let mut approximate = None;
    let mut equal = None;
    let mut stop = None;
    for offset in 0..length {
        let value = match value_at(offset) {
            Ok(value) => value,
            Err(kind) => {
                return MatchScan {
                    found: Err(kind),
                    reads: Err(kind),
                };
            }
        };
        // Approximate matching passes over error cells, as Excel does.
        if matches!(value, Value::Error(_)) {
            continue;
        }
        let ordering = match matcher.compare(&value, lookup) {
            Ok(ordering) => ordering,
            Err(kind) => {
                return MatchScan {
                    found: Err(kind),
                    reads: Err(kind),
                };
            }
        };
        if ordering == std::cmp::Ordering::Equal {
            stop = Some(Ok(offset as u32));
            equal = Some(offset);
            break;
        }
        if (match_type == 1.0 && ordering == std::cmp::Ordering::Less)
            || (match_type == -1.0 && ordering == std::cmp::Ordering::Greater)
        {
            approximate = Some(offset);
        } else if (match_type == 1.0 && ordering == std::cmp::Ordering::Greater)
            || (match_type == -1.0 && ordering == std::cmp::Ordering::Less)
        {
            stop = Some(Ok(offset as u32));
            break;
        }
    }
    MatchScan {
        found: equal
            .or(approximate)
            .map(|offset| offset as u32)
            .ok_or(ErrorKind::NA),
        reads: stop.unwrap_or(Err(ErrorKind::NA)),
    }
}
