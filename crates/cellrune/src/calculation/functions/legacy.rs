use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number, to_text};
use super::super::criteria::{
    Criteria, CriteriaOp, CriteriaRhs, WildcardStepBudget, parse_criteria,
};
use super::super::decimal::DecimalTrace;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::operators::{broadcast_shape, element_at};
use super::super::runtime::{Array, Rect};
use super::super::textfmt::format_number;
use super::super::value::{ErrorKind, Value};
use super::util::ExcelSum;

pub(super) fn call_legacy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name.to_ascii_uppercase().as_str() {
        "IF" => kernel_if(engine, context, args),
        "AND" => kernel_and(engine, context, args),
        "IFERROR" => kernel_iferror(engine, context, args),
        "LOWER" => kernel_lower(engine, context, args),
        "TEXT" => kernel_text(engine, context, args),
        "COUNTIF" => kernel_countifs(engine, context, args, true),
        "COUNTIFS" => kernel_countifs(engine, context, args, false),
        "SUMPRODUCT" => kernel_sumproduct(engine, context, args),
        "INDEX" => kernel_index(engine, context, args),
        "MATCH" => kernel_match(engine, context, args),
        "__XLUDF.DUMMYFUNCTION" if args.len() == 1 => Value::Error(ErrorKind::Name),
        "__XLUDF.DUMMYFUNCTION" => Value::Error(ErrorKind::Value),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

pub(super) fn call_legacy_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<Array, ErrorKind>> {
    match name.to_ascii_uppercase().as_str() {
        "IF" => Some(if_array(engine, context, args)),
        "COUNTIF" => countifs_array(engine, context, args, true),
        "COUNTIFS" => countifs_array(engine, context, args, false),
        "INDEX" => Some(index_array(engine, context, args)),
        _ => None,
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
    let when_true = match args.get(1) {
        Some(Expr::Missing) => Array::scalar(Value::Number(0.0)),
        Some(expr) => engine.eval_array(context, expr)?,
        None => return Err(ErrorKind::Value),
    };
    let when_false = match args.get(2) {
        Some(Expr::Missing) => Array::scalar(Value::Number(0.0)),
        Some(expr) => engine.eval_array(context, expr)?,
        None => Array::scalar(Value::Logical(false)),
    };
    let condition_and_true = shape_array(&condition, &when_true)?;
    let (rows, cols) = broadcast_shape(&condition_and_true, &when_false)?;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for column in 0..cols {
            let selected = match to_logical(element_at(&condition, row, column)) {
                Ok(true) => element_at(&when_true, row, column).clone(),
                Ok(false) => element_at(&when_false, row, column).clone(),
                Err(kind) => Value::Error(kind),
            };
            data.push(selected);
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
            let row_end = engine.clamped_row_end(&rect);
            if row_end >= rect.row_start {
                let cells = (u64::from(row_end - rect.row_start) + 1).checked_mul(rect.width());
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
            for row in rect.row_start..=row_end {
                for column in rect.col_start..=rect.col_end {
                    match engine.cell_value((rect.sheet, row, column)) {
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
            }
            continue;
        }
        let value = engine.eval_scalar(context, arg);
        if reference_rect(engine, context, arg).is_ok() {
            if matches!(&value, Value::Blank)
                || matches!(&value, Value::Text(text) if text.is_empty())
            {
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
        match to_logical_aggregate_argument(&value, is_text_literal(arg)) {
            Ok(logical) => {
                participants += 1;
                all_true &= logical;
            }
            Err(kind) => return Value::Error(kind),
        }
    }
    if participants == 0 {
        return Value::Error(ErrorKind::Value);
    }
    Value::Logical(all_true)
}

fn to_logical_aggregate_argument(
    value: &Value,
    coerce_numeric_text: bool,
) -> Result<bool, ErrorKind> {
    match value {
        Value::Text(text) if coerce_numeric_text => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(|number| number != 0.0)
            .or_else(|| {
                if text.eq_ignore_ascii_case("TRUE") {
                    Some(true)
                } else if text.eq_ignore_ascii_case("FALSE") {
                    Some(false)
                } else {
                    None
                }
            })
            .ok_or(ErrorKind::Value),
        _ => to_logical(value),
    }
}

fn is_text_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Text(_) => true,
        Expr::Paren(inner) => is_text_literal(inner),
        _ => false,
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

fn kernel_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match to_number(&engine.eval_scalar(context, &args[0])) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let format = match to_text(&engine.eval_scalar(context, &args[1])) {
        Ok(format) => format,
        Err(kind) => return Value::Error(kind),
    };
    match format_number(number, &format) {
        Ok(text) => engine.bounded_text(text),
        Err(kind) => Value::Error(kind),
    }
}

struct CriteriaPairs {
    rects: Vec<Rect>,
    criteria_exprs: Vec<Expr>,
}

fn split_pairs(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    single_pair: bool,
) -> Result<CriteriaPairs, ErrorKind> {
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
        criteria_exprs.push(pair[1].clone());
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
    rects: &[Rect],
    criteria: &[Criteria],
    wildcard_budget: &mut WildcardStepBudget,
) -> Result<f64, ErrorKind> {
    let height = rects[0].height();
    let width = rects[0].width();
    let mut iter_rows = 0_u64;
    for rect in rects {
        let clamped = engine.clamped_row_end(rect);
        let available = if clamped >= rect.row_start {
            u64::from(clamped - rect.row_start) + 1
        } else {
            0
        };
        iter_rows = iter_rows.max(available.min(height));
    }
    let visits = iter_rows
        .checked_mul(width)
        .and_then(|cells| cells.checked_mul(rects.len() as u64))
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    engine.ensure_array_cells(visits)?;
    let mut count = 0_u64;
    for relative_row in 0..iter_rows {
        for relative_col in 0..width {
            let mut matched = true;
            for (rect, criterion) in rects.iter().zip(criteria.iter()) {
                let value = engine.cell_value((
                    rect.sheet,
                    rect.row_start + relative_row as u32,
                    rect.col_start + relative_col as u32,
                ));
                if !criterion.matches(&value, wildcard_budget)? {
                    matched = false;
                    break;
                }
            }
            if matched {
                count += 1;
            }
        }
    }
    let virtual_cells = (height - iter_rows) * width;
    if virtual_cells > 0 && criteria.iter().all(Criteria::matches_blank) {
        count += virtual_cells;
    }
    Ok(count as f64)
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
    let mut criteria = Vec::with_capacity(pairs.criteria_exprs.len());
    for expr in &pairs.criteria_exprs {
        let value = engine.eval_scalar(context, expr);
        match parse_criteria(&value) {
            Ok(criterion) => criteria.push(criterion),
            Err(kind) => return Value::Error(kind),
        }
    }
    let mut wildcard_budget = WildcardStepBudget::new(engine.max_function_iterations());
    match count_matches(engine, &pairs.rects, &criteria, &mut wildcard_budget) {
        Ok(count) => Value::Number(count),
        Err(kind) => Value::Error(kind),
    }
}

fn countifs_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    single_pair: bool,
) -> Option<Result<Array, ErrorKind>> {
    let pairs = match split_pairs(engine, context, args, single_pair) {
        Ok(pairs) => pairs,
        Err(_) => return None,
    };
    let has_broadcast = pairs
        .criteria_exprs
        .iter()
        .any(|expr| multi_cell_rect(engine, context, expr).is_some());
    if !has_broadcast {
        return None;
    }
    Some(countifs_broadcast(engine, context, &pairs))
}

fn countifs_broadcast(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    pairs: &CriteriaPairs,
) -> Result<Array, ErrorKind> {
    let mut criteria_arrays = Vec::with_capacity(pairs.criteria_exprs.len());
    for expr in &pairs.criteria_exprs {
        if multi_cell_rect(engine, context, expr).is_some() {
            criteria_arrays.push(engine.eval_array(context, expr)?);
        } else {
            criteria_arrays.push(Array::scalar(engine.eval_scalar(context, expr)));
        }
    }
    let mut rows = 1_u32;
    let mut cols = 1_u32;
    for array in &criteria_arrays {
        let scalar_shape = Array {
            rows,
            cols,
            data: Vec::new(),
        };
        let (next_rows, next_cols) = broadcast_shape(&scalar_shape, array)?;
        rows = next_rows;
        cols = next_cols;
    }
    engine.ensure_array_cells(u64::from(rows) * u64::from(cols))?;
    let mut data = Vec::with_capacity((rows * cols) as usize);
    let mut wildcard_budget = WildcardStepBudget::new(engine.max_function_iterations());
    for row in 0..rows {
        for col in 0..cols {
            let mut criteria = Vec::with_capacity(criteria_arrays.len());
            let mut element_error = None;
            for array in &criteria_arrays {
                match parse_criteria(element_at(array, row, col)) {
                    Ok(criterion) => criteria.push(criterion),
                    Err(kind) => {
                        element_error = Some(kind);
                        break;
                    }
                }
            }
            if let Some(kind) = element_error {
                data.push(Value::Error(kind));
                continue;
            }
            match count_matches(engine, &pairs.rects, &criteria, &mut wildcard_budget) {
                Ok(count) => data.push(Value::Number(count)),
                Err(kind) => data.push(Value::Error(kind)),
            }
        }
    }
    Ok(Array { rows, cols, data })
}

fn kernel_sumproduct(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    // Read with traces: the terms this sums are products of exact decimals, so they can cancel the
    // same way a chain of `+` and `-` does. Discarding the traces here would leave `SUMPRODUCT`
    // outside the arithmetic policy and disagreeing with `SUM` on identical data.
    let mut arrays = Vec::with_capacity(args.len());
    for arg in args {
        match engine.eval_array_with_trace(context, arg) {
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
    let mut total = ExcelSum::new(engine);
    for row in 0..rows {
        for col in 0..cols {
            let mut product = 1.0_f64;
            let mut product_trace = Some(DecimalTrace::ONE);
            for evaluated in &arrays {
                let (number, trace) = match element_at(&evaluated.array, row, col) {
                    Value::Error(kind) => return Value::Error(*kind),
                    Value::Number(number) => (*number, evaluated.decimal_at(row, col)),
                    // Excel treats a non-numeric cell as zero rather than as an error, and zero is
                    // exactly representable, so the term stays traceable.
                    Value::Logical(_) | Value::Text(_) | Value::Blank => {
                        (0.0, Some(DecimalTrace::ZERO))
                    }
                };
                product *= number;
                product_trace = product_trace.and_then(|running| running.multiply(trace?));
            }
            total.add_with_trace(product, product_trace);
        }
    }
    Value::Number(total.total())
}

fn kernel_index(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    engine
        .resolve_index_rect(context, args)
        .and_then(|rect| engine.implicit_intersection_rect(context, rect))
        .map_or_else(Value::Error, |rect| {
            engine.cell_value((rect.sheet, rect.row_start, rect.col_start))
        })
}

fn index_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let rect = engine.resolve_index_rect(context, args)?;
    engine.array_from_rect(rect)
}

fn kernel_match(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let rect = match reference_rect(engine, context, &args[1]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    if rect.height() > 1 && rect.width() > 1 {
        return Value::Error(ErrorKind::Unsupported);
    }
    let match_type = match args.get(2) {
        Some(expr) => match to_number(&engine.eval_scalar(context, expr)) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
        None => 1.0,
    };
    if !matches!(match_type, -1.0 | 0.0 | 1.0) {
        return Value::Error(ErrorKind::NA);
    }
    if lookup.is_blank_like() {
        return Value::Error(ErrorKind::NA);
    }
    let criterion = match &lookup {
        Value::Number(number) => Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Number(*number),
        },
        Value::Logical(logical) => Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Logical(*logical),
        },
        Value::Text(text) => Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Text(text.clone()),
        },
        _ => return Value::Error(ErrorKind::NA),
    };
    let vertical = rect.width() == 1;
    let clamped_row_end = engine.clamped_row_end(&rect);
    let length = if vertical {
        if clamped_row_end >= rect.row_start {
            u64::from(clamped_row_end - rect.row_start) + 1
        } else {
            0
        }
    } else {
        rect.width()
    };
    if let Err(kind) = engine.ensure_array_cells(length) {
        return Value::Error(kind);
    }
    let mut approximate = None;
    let mut wildcard_budget = WildcardStepBudget::new(engine.max_function_iterations());
    for offset in 0..length {
        let (row, column) = if vertical {
            (rect.row_start + offset as u32, rect.col_start)
        } else {
            (rect.row_start, rect.col_start + offset as u32)
        };
        let value = engine.cell_value((rect.sheet, row, column));
        match criterion.matches(&value, &mut wildcard_budget) {
            Ok(true) => return Value::Number((offset + 1) as f64),
            Ok(false) => {}
            Err(kind) => return Value::Error(kind),
        }
        if match_type != 0.0 {
            let ordering = match super::super::coerce::compare(&value, &lookup) {
                Ok(ordering) => ordering,
                Err(kind) => return Value::Error(kind),
            };
            if (match_type == 1.0 && ordering == std::cmp::Ordering::Less)
                || (match_type == -1.0 && ordering == std::cmp::Ordering::Greater)
            {
                approximate = Some(offset);
            } else if (match_type == 1.0 && ordering == std::cmp::Ordering::Greater)
                || (match_type == -1.0 && ordering == std::cmp::Ordering::Less)
            {
                break;
            }
        }
    }
    approximate.map_or(Value::Error(ErrorKind::NA), |offset| {
        Value::Number((offset + 1) as f64)
    })
}
