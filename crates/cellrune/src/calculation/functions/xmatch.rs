use std::borrow::Cow;
use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::validate_array_input;
use super::criteria_runtime::CriteriaRuntime;
use super::lookup_common::VectorView;
use super::util::required_number;

pub(super) fn xmatch(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if !(2..=4).contains(&args.len()) {
        return Value::Error(ErrorKind::Value);
    }
    let lookup = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let array = match engine.eval_array(context, &args[1]) {
        Ok(array) => array,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = validate_array_input(engine, context, &array) {
        return Value::Error(kind);
    }
    let Some(values) = VectorView::new(&array) else {
        return Value::Error(ErrorKind::Value);
    };
    let match_mode = match parse_match_mode(engine, context, args.get(2)) {
        Ok(mode) => mode,
        Err(kind) => return Value::Error(kind),
    };
    let search_mode = match parse_search_mode(engine, context, args.get(3)) {
        Ok(mode) => mode,
        Err(kind) => return Value::Error(kind),
    };
    let result = find_match(
        engine,
        context,
        &lookup,
        values.len(),
        match_mode,
        search_mode,
        |offset| Ok(Cow::Borrowed(values.at(offset))),
    );
    match result {
        Ok(offset) => Value::Number(f64::from(offset + 1)),
        Err(kind) => Value::Error(kind),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatchMode {
    Exact,
    NextSmaller,
    NextLarger,
    Wildcard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchMode {
    Forward,
    Reverse,
    BinaryAscending,
    BinaryDescending,
}

pub(super) fn parse_match_mode(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<MatchMode, ErrorKind> {
    let mode = match expr {
        Some(expr) if !matches!(expr, Expr::Missing) => {
            required_number(engine, context, expr)?.trunc()
        }
        _ => 0.0,
    };
    match mode {
        0.0 => Ok(MatchMode::Exact),
        -1.0 => Ok(MatchMode::NextSmaller),
        1.0 => Ok(MatchMode::NextLarger),
        2.0 => Ok(MatchMode::Wildcard),
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn parse_search_mode(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<SearchMode, ErrorKind> {
    let mode = match expr {
        Some(expr) if !matches!(expr, Expr::Missing) => {
            required_number(engine, context, expr)?.trunc()
        }
        _ => 1.0,
    };
    match mode {
        1.0 => Ok(SearchMode::Forward),
        -1.0 => Ok(SearchMode::Reverse),
        2.0 => Ok(SearchMode::BinaryAscending),
        -2.0 => Ok(SearchMode::BinaryDescending),
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn find_match<'value, F>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lookup: &Value,
    length: u32,
    match_mode: MatchMode,
    search_mode: SearchMode,
    values: F,
) -> Result<u32, ErrorKind>
where
    F: FnMut(u32) -> Result<Cow<'value, Value>, ErrorKind>,
{
    match search_mode {
        SearchMode::Forward | SearchMode::Reverse => linear_match(
            engine,
            context,
            lookup,
            length,
            match_mode,
            search_mode,
            values,
        ),
        SearchMode::BinaryAscending | SearchMode::BinaryDescending => binary_match(
            engine,
            context,
            lookup,
            length,
            match_mode,
            search_mode,
            values,
        ),
    }
}

fn linear_match<'value, F>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lookup: &Value,
    length: u32,
    match_mode: MatchMode,
    search_mode: SearchMode,
    mut value_at: F,
) -> Result<u32, ErrorKind>
where
    F: FnMut(u32) -> Result<Cow<'value, Value>, ErrorKind>,
{
    let mut criteria_runtime = CriteriaRuntime::new(engine, context);
    let wildcard_pattern = if match_mode == MatchMode::Wildcard {
        let Value::Text(pattern) = lookup else {
            return Err(ErrorKind::NA);
        };
        Some(criteria_runtime.compile_wildcard(pattern)?)
    } else {
        None
    };
    let mut best: Option<(u32, Cow<'value, Value>)> = None;
    for step in 0..length {
        criteria_runtime.charge_work(1)?;
        let offset = match search_mode {
            SearchMode::Forward => step,
            SearchMode::Reverse => length - step - 1,
            SearchMode::BinaryAscending | SearchMode::BinaryDescending => {
                unreachable!("binary modes use binary_match")
            }
        };
        let candidate = value_at(offset)?;
        if let Some(pattern) = &wildcard_pattern {
            match candidate.as_ref() {
                Value::Text(text) => {
                    if criteria_runtime.wildcard_matches(pattern, text)? {
                        return Ok(offset);
                    }
                }
                Value::Error(kind) => return Err(*kind),
                Value::Blank | Value::Number(_) | Value::Logical(_) => {}
            }
            continue;
        }
        let ordering = criteria_runtime.compare(candidate.as_ref(), lookup)?;
        if ordering == Ordering::Equal {
            return Ok(offset);
        }
        if is_better_approximate(
            &mut criteria_runtime,
            candidate.as_ref(),
            best.as_ref().map(|(_, value)| value.as_ref()),
            ordering,
            match_mode,
        )? {
            best = Some((offset, candidate));
        }
    }
    best.map(|(offset, _)| offset).ok_or(ErrorKind::NA)
}

fn is_better_approximate(
    criteria_runtime: &mut CriteriaRuntime<'_, '_, '_>,
    candidate: &Value,
    best: Option<&Value>,
    lookup_ordering: Ordering,
    mode: MatchMode,
) -> Result<bool, ErrorKind> {
    let qualifies = matches!(
        (mode, lookup_ordering),
        (MatchMode::NextSmaller, Ordering::Less) | (MatchMode::NextLarger, Ordering::Greater)
    );
    if !qualifies {
        return Ok(false);
    }
    let Some(best) = best else {
        return Ok(true);
    };
    criteria_runtime.charge_work(1)?;
    let ordering = criteria_runtime.compare(candidate, best)?;
    Ok(match mode {
        MatchMode::NextSmaller => ordering == Ordering::Greater,
        MatchMode::NextLarger => ordering == Ordering::Less,
        MatchMode::Exact | MatchMode::Wildcard => false,
    })
}

fn binary_match<'value, F>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lookup: &Value,
    length: u32,
    match_mode: MatchMode,
    search_mode: SearchMode,
    mut value_at: F,
) -> Result<u32, ErrorKind>
where
    F: FnMut(u32) -> Result<Cow<'value, Value>, ErrorKind>,
{
    if match_mode == MatchMode::Wildcard {
        return Err(ErrorKind::Value);
    }
    let ascending = match search_mode {
        SearchMode::BinaryAscending => true,
        SearchMode::BinaryDescending => false,
        SearchMode::Forward | SearchMode::Reverse => {
            unreachable!("linear modes use linear_match")
        }
    };
    let mut low = 0_u32;
    let mut high = length;
    let mut criteria_runtime = CriteriaRuntime::new(engine, context);
    while low < high {
        criteria_runtime.charge_work(1)?;
        let middle = low + (high - low) / 2;
        let candidate = value_at(middle)?;
        let ordering = criteria_runtime.compare(candidate.as_ref(), lookup)?;
        if ordering == Ordering::Equal {
            return Ok(middle);
        }
        let search_ordering = if ascending {
            ordering
        } else {
            ordering.reverse()
        };
        if search_ordering == Ordering::Less {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let candidate = match (ascending, match_mode) {
        (_, MatchMode::Exact) => None,
        (true, MatchMode::NextSmaller) | (false, MatchMode::NextLarger) => low.checked_sub(1),
        (true, MatchMode::NextLarger) | (false, MatchMode::NextSmaller) => {
            (low < length).then_some(low)
        }
        (_, MatchMode::Wildcard) => unreachable!("wildcard returned before binary search"),
    };
    candidate.ok_or(ErrorKind::NA)
}
