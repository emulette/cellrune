use std::collections::HashSet;

use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::validate_array_input;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldHeaders {
    Automatic,
    None,
    ExistingHidden,
    Generated,
    ExistingShown,
}

impl FieldHeaders {
    pub(super) const fn consumes_input_header(self) -> Option<bool> {
        match self {
            Self::Automatic => None,
            Self::None | Self::Generated => Some(false),
            Self::ExistingHidden | Self::ExistingShown => Some(true),
        }
    }

    pub(super) const fn explicitly_shows_output(self) -> bool {
        matches!(self, Self::Generated | Self::ExistingShown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TotalPlacement {
    End,
    Start,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TotalDepth {
    pub(super) levels: u32,
    pub(super) placement: TotalPlacement,
    pub(super) automatic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SortCriterion {
    pub(super) index: usize,
    pub(super) descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldRelationship {
    Hierarchy,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelativeSet {
    Column,
    Row,
    Grand,
    ParentColumn,
    ParentRow,
}

pub(super) fn parse_field_headers(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<FieldHeaders, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(FieldHeaders::Automatic);
    };
    match integer(engine, context, expr)? {
        0 => Ok(FieldHeaders::None),
        1 => Ok(FieldHeaders::ExistingHidden),
        2 => Ok(FieldHeaders::Generated),
        3 => Ok(FieldHeaders::ExistingShown),
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn resolve_input_header(headers: FieldHeaders, values: &Array) -> bool {
    headers.consumes_input_header().unwrap_or_else(|| {
        values.rows >= 2
            && matches!(values.at(0, 0), Value::Text(_))
            && matches!(values.at(1, 0), Value::Number(_))
    })
}

pub(super) const fn output_headers_are_shown(headers: FieldHeaders) -> bool {
    headers.explicitly_shows_output()
}

pub(super) fn parse_total_depth(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    field_levels: u32,
) -> Result<TotalDepth, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(TotalDepth {
            levels: field_levels,
            placement: TotalPlacement::End,
            automatic: true,
        });
    };
    let value = integer(engine, context, expr)?;
    let magnitude = value.unsigned_abs();
    if magnitude > u64::from(field_levels) {
        return Err(ErrorKind::Value);
    }
    Ok(TotalDepth {
        levels: magnitude as u32,
        placement: if value < 0 {
            TotalPlacement::Start
        } else {
            TotalPlacement::End
        },
        automatic: false,
    })
}

pub(super) fn parse_sort_order(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    field_count: usize,
    value_count: usize,
) -> Result<Vec<SortCriterion>, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(Vec::new());
    };
    let values = engine.eval_array(context, expr)?;
    validate_array_input(engine, context, &values)?;
    if values.rows > 1 && values.cols > 1 {
        return Err(ErrorKind::Value);
    }
    let vector = values.data.len() > 1;
    let maximum = if vector {
        field_count
    } else {
        field_count.checked_add(value_count).ok_or(ErrorKind::Num)?
    };
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(values.data.len());
    engine.charge_function_iterations(
        context,
        u64::try_from(values.data.len()).map_err(|_| ErrorKind::Num)?,
    )?;
    for value in &values.data {
        super::array_common::poll_cancellation(context)?;
        let signed = integer_value(value)?;
        let magnitude = signed.unsigned_abs();
        if magnitude == 0 || magnitude > maximum as u64 {
            return Err(ErrorKind::Value);
        }
        let index = magnitude as usize - 1;
        if !seen.insert(index) {
            return Err(ErrorKind::Value);
        }
        result.push(SortCriterion {
            index,
            descending: signed < 0,
        });
    }
    Ok(result)
}

pub(super) fn parse_filter(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    expected_rows: u32,
) -> Result<Option<Vec<bool>>, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(None);
    };
    let filter = engine.eval_array(context, expr)?;
    validate_array_input(engine, context, &filter)?;
    if filter.cols != 1 || filter.rows != expected_rows {
        return Err(ErrorKind::Value);
    }
    engine.charge_function_iterations(context, u64::from(filter.rows))?;
    let mut result = Vec::with_capacity(filter.data.len());
    for value in &filter.data {
        super::array_common::poll_cancellation(context)?;
        result.push(to_logical(value)?);
    }
    Ok(Some(result))
}

pub(super) fn parse_field_relationship(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<FieldRelationship, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(FieldRelationship::Hierarchy);
    };
    match integer(engine, context, expr)? {
        0 => Ok(FieldRelationship::Hierarchy),
        1 => Ok(FieldRelationship::Table),
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn validate_relationship_total_depth(
    relationship: FieldRelationship,
    total_depth: TotalDepth,
) -> Result<(), ErrorKind> {
    if relationship == FieldRelationship::Table && !total_depth.automatic && total_depth.levels > 1
    {
        return Err(ErrorKind::Value);
    }
    Ok(())
}

pub(super) fn parse_relative_to(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<RelativeSet, ErrorKind> {
    let Some(expr) = present(expr) else {
        return Ok(RelativeSet::Column);
    };
    match integer(engine, context, expr)? {
        0 => Ok(RelativeSet::Column),
        1 => Ok(RelativeSet::Row),
        2 => Ok(RelativeSet::Grand),
        3 => Ok(RelativeSet::ParentColumn),
        4 => Ok(RelativeSet::ParentRow),
        _ => Err(ErrorKind::Value),
    }
}

fn present(expr: Option<&Expr>) -> Option<&Expr> {
    expr.filter(|expr| !matches!(expr, Expr::Missing))
}

fn integer(engine: &Engine<'_>, context: EvalContext<'_>, expr: &Expr) -> Result<i64, ErrorKind> {
    integer_value(&engine.eval_scalar(context, expr))
}

fn integer_value(value: &Value) -> Result<i64, ErrorKind> {
    let number = to_number(value)?;
    if !number.is_finite() || number < i64::MIN as f64 || number > i64::MAX as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(number.trunc() as i64)
}
