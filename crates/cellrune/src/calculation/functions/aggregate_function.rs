//! `AGGREGATE(function_num, options, ref1, …)` and `AGGREGATE(function_num, options, array, k)`.
//!
//! Microsoft documents the nineteen functions and the eight options: 0 to 3 skip cells holding
//! `SUBTOTAL` or `AGGREGATE`, odd options skip hidden rows, and 2, 3, 6, and 7 skip error values.
//! Functions 1 to 13 take references, and 14 to 19 take one array and `k`; hidden rows and nested
//! totals are skipped only inside references, never inside a calculated array. Excel 16.114 draws
//! no line between rows a filter hides and rows hidden by hand: the even options keep both and the
//! odd options skip both (excel-measure RESULTS.md, section C), unlike `SUBTOTAL` 1 to 11.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::aggregate::{contains_subtotal_call, subtotal_operation};
use super::statistical::{RankInterval, median_of, mode_of, order_statistic_of, percentile_of};
use super::util::{collect_argument_values_including, excel_numbers, required_number};

pub(super) fn aggregate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match evaluate(engine, context, args) {
        Ok(value) | Err(value) => value,
    }
}

fn evaluate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Value, Value> {
    if args.len() < 3 {
        return Err(Value::Error(ErrorKind::Value));
    }
    let function = required_number(engine, context, &args[0])
        .map_err(Value::Error)?
        .trunc();
    let options = match &args[1] {
        Expr::Missing => 0.0,
        expr => required_number(engine, context, expr)
            .map_err(Value::Error)?
            .trunc(),
    };
    if !(1.0..=19.0).contains(&function) || !(0.0..=7.0).contains(&options) {
        return Err(Value::Error(ErrorKind::Value));
    }
    let (function, options) = (function as u8, options as u8);
    let skip_hidden_rows = options % 2 == 1;
    let skip_errors = matches!(options, 2 | 3 | 6 | 7);
    let skip_nested_totals = options <= 3;
    // Functions 14 to 19 read one array and its `k`.
    let (sources, k) = if function <= 13 {
        (&args[2..], None)
    } else if args.len() == 4 {
        (&args[2..3], Some(&args[3]))
    } else {
        return Err(Value::Error(ErrorKind::Value));
    };

    let include = |cell| {
        let nested_total =
            skip_nested_totals && engine.parsed_expr(cell).is_some_and(contains_subtotal_call);
        let hidden_row = skip_hidden_rows && engine.is_row_hidden(cell);
        !nested_total && !hidden_row
    };
    let mut values = Vec::new();
    for source in sources {
        // The reference form takes references only: a constant array is `#VALUE!`.
        if function <= 13 {
            engine
                .resolve_reference_value_expr(context, source)
                .map_err(Value::Error)?;
        }
        values.extend(
            collect_argument_values_including(
                engine,
                context,
                std::slice::from_ref(source),
                SheetSpanPolicy::ReturnExcelError(ErrorKind::Value),
                &include,
            )
            .map_err(Value::Error)?,
        );
    }
    if skip_errors {
        values.retain(|item| !matches!(item.value, Value::Error(kind) if !kind.is_engine_issue()));
    }
    Ok(match function {
        1..=11 => subtotal_operation(engine, context, function, values),
        12 => excel_numbers(values, engine.numeric_text()).map_or_else(Value::Error, median_of),
        13 => mode_of(values),
        _ => {
            let numbers = excel_numbers(values, engine.numeric_text()).map_err(Value::Error)?;
            let k = required_number(engine, context, k.expect("functions 14 to 19 read k"))
                .map_err(Value::Error)?;
            match function {
                14 => order_statistic_of(numbers, k, false),
                15 => order_statistic_of(numbers, k, true),
                16 => percentile_of(numbers, k, false, RankInterval::Inclusive),
                17 => percentile_of(numbers, k, true, RankInterval::Inclusive),
                18 => percentile_of(numbers, k, false, RankInterval::Exclusive),
                _ => percentile_of(numbers, k, true, RankInterval::Exclusive),
            }
        }
    })
}
