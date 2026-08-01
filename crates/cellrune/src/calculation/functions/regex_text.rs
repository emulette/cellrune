use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::modern_text::push_bounded;
use super::regex_common::{CompiledRegex, RegexCaptureSet};
use super::util::required_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractMode {
    First,
    All,
    Captures,
}

#[derive(Debug, Clone, Copy)]
struct NamedCaptureLookup<'a> {
    subject: &'a str,
    captures: &'a RegexCaptureSet,
    name_indexes: &'a std::collections::BTreeMap<String, Vec<usize>>,
}

pub(super) fn extract_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    match extract_array(engine, context, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Calc)),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn extract_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || args.len() > 4 {
        return Err(ErrorKind::Value);
    }
    let subject = required_text(engine, context, &args[0])?;
    let pattern = required_text(engine, context, &args[1])?;
    let mode = match integer_option(engine, context, args.get(2), 0)? {
        0 => ExtractMode::First,
        1 => ExtractMode::All,
        2 => ExtractMode::Captures,
        _ => return Err(ErrorKind::Value),
    };
    let case_insensitive = case_option(engine, context, args.get(3))?;
    let mut regex = CompiledRegex::compile(engine, context, &pattern, case_insensitive)?;
    let maximum = match mode {
        ExtractMode::All => usize::try_from(engine.max_array_cells())
            .ok()
            .and_then(|maximum| maximum.checked_add(1)),
        ExtractMode::First | ExtractMode::Captures => Some(1),
    };
    let matches = regex.captures(engine, context, &subject, maximum)?;
    let Some(first) = matches.first() else {
        return Err(ErrorKind::NA);
    };

    match mode {
        ExtractMode::First => array_from_spans(engine, context, &subject, 1, 1, [first.span(0)]),
        ExtractMode::All => {
            let rows = u32::try_from(matches.len()).map_err(|_| ErrorKind::Num)?;
            array_from_spans(
                engine,
                context,
                &subject,
                rows,
                1,
                matches.iter().map(|captures| captures.span(0)),
            )
        }
        ExtractMode::Captures => {
            let groups = first.len().checked_sub(1).ok_or(ErrorKind::Calc)?;
            if groups == 0 {
                return Err(ErrorKind::Calc);
            }
            let columns = u32::try_from(groups).map_err(|_| ErrorKind::Num)?;
            array_from_spans(
                engine,
                context,
                &subject,
                1,
                columns,
                (1..first.len()).map(|index| first.span(index)),
            )
        }
    }
}

fn array_from_spans(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    subject: &str,
    rows: u32,
    cols: u32,
    spans: impl IntoIterator<Item = Option<(usize, usize)>>,
) -> Result<Array, ErrorKind> {
    let cells = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    let spans: Vec<Option<(usize, usize)>> = spans.into_iter().collect();
    if u64::try_from(spans.len()).map_err(|_| ErrorKind::Num)? != cells {
        return Err(ErrorKind::Calc);
    }
    let copied_bytes = spans.iter().try_fold(0_u64, |bytes, span| {
        let length = match span {
            Some((start, end)) => subject.get(*start..*end).ok_or(ErrorKind::Value)?.len(),
            None => 0,
        };
        bytes
            .checked_add(u64::try_from(length).map_err(|_| ErrorKind::Num)?)
            .ok_or(ErrorKind::Num)
    })?;
    engine.charge_function_iterations(context, copied_bytes)?;
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for span in spans {
        let text = match span {
            Some((start, end)) => subject.get(start..end).ok_or(ErrorKind::Value)?.to_owned(),
            None => String::new(),
        };
        engine.ensure_text_bytes(text.len())?;
        data.push(Value::Text(text));
    }
    Ok(Array { rows, cols, data })
}

pub(super) fn test(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let subject = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let pattern = match required_text(engine, context, &args[1]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let case_insensitive = match case_option(engine, context, args.get(2)) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let mut regex = match CompiledRegex::compile(engine, context, &pattern, case_insensitive) {
        Ok(regex) => regex,
        Err(kind) => return Value::Error(kind),
    };
    match regex.captures(engine, context, &subject, Some(1)) {
        Ok(matches) => Value::Logical(!matches.is_empty()),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn replace(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match replace_result(engine, context, args) {
        Ok(output) => Value::Text(output),
        Err(kind) => Value::Error(kind),
    }
}

fn replace_result(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<String, ErrorKind> {
    if args.len() < 3 || args.len() > 5 {
        return Err(ErrorKind::Value);
    }
    let subject = required_text(engine, context, &args[0])?;
    let pattern = required_text(engine, context, &args[1])?;
    let replacement = required_text(engine, context, &args[2])?;
    engine.ensure_text_bytes(replacement.len())?;
    let occurrence = integer_option(engine, context, args.get(3), 0)?;
    let case_insensitive = case_option(engine, context, args.get(4))?;
    let mut regex = CompiledRegex::compile(engine, context, &pattern, case_insensitive)?;
    let maximum_matches = (occurrence > 0)
        .then(|| usize::try_from(occurrence).map_err(|_| ErrorKind::Value))
        .transpose()?;
    let matches = regex.captures(engine, context, &subject, maximum_matches)?;
    let selected = selected_match(occurrence, matches.len());
    let mut output = String::new();
    let mut cursor = 0;
    for (index, captures) in matches.iter().enumerate() {
        if selected.is_some_and(|selected| selected != index) {
            continue;
        }
        let (start, end) = captures.span(0).ok_or(ErrorKind::Value)?;
        push_bounded(
            engine,
            &mut output,
            subject.get(cursor..start).ok_or(ErrorKind::Value)?,
        )?;
        expand_replacement(
            engine,
            context,
            &mut output,
            &replacement,
            &subject,
            captures,
            regex.capture_name_indexes(),
        )?;
        cursor = end;
    }
    push_bounded(
        engine,
        &mut output,
        subject.get(cursor..).ok_or(ErrorKind::Value)?,
    )?;
    Ok(output)
}

fn selected_match(occurrence: i64, count: usize) -> Option<usize> {
    if occurrence == 0 {
        return None;
    }
    if occurrence > 0 {
        usize::try_from(occurrence - 1)
            .ok()
            .filter(|index| *index < count)
            .or(Some(count))
    } else {
        usize::try_from(occurrence.unsigned_abs())
            .ok()
            .and_then(|offset| count.checked_sub(offset))
            .or(Some(count))
    }
}

fn expand_replacement(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    output: &mut String,
    replacement: &str,
    subject: &str,
    captures: &RegexCaptureSet,
    name_indexes: &std::collections::BTreeMap<String, Vec<usize>>,
) -> Result<(), ErrorKind> {
    engine.charge_function_iterations(context, replacement.len() as u64)?;
    let named_lookup = NamedCaptureLookup {
        subject,
        captures,
        name_indexes,
    };
    let mut cursor = 0;
    while let Some(relative) = replacement[cursor..].find('$') {
        let dollar = cursor + relative;
        push_bounded(engine, output, &replacement[cursor..dollar])?;
        let tail = replacement.get(dollar + 1..).ok_or(ErrorKind::Value)?;
        let Some(next) = tail.chars().next() else {
            return Err(ErrorKind::Value);
        };
        cursor = dollar + 1 + next.len_utf8();
        match next {
            '$' => push_bounded(engine, output, "$"),
            '&' => append_capture(engine, output, subject, captures, 0),
            '`' => {
                let (start, _) = captures.span(0).ok_or(ErrorKind::Value)?;
                push_bounded(
                    engine,
                    output,
                    subject.get(..start).ok_or(ErrorKind::Value)?,
                )
            }
            '\'' => {
                let (_, end) = captures.span(0).ok_or(ErrorKind::Value)?;
                push_bounded(engine, output, subject.get(end..).ok_or(ErrorKind::Value)?)
            }
            '_' => push_bounded(engine, output, subject),
            '0'..='9' => {
                let end = replacement[cursor..]
                    .find(|character: char| !character.is_ascii_digit())
                    .map_or(replacement.len(), |relative| cursor + relative);
                let start = dollar + 1;
                let index = replacement[start..end]
                    .parse::<usize>()
                    .map_err(|_| ErrorKind::Value)?;
                cursor = end;
                append_capture(engine, output, subject, captures, index)
            }
            '{' => {
                let relative_end = replacement[cursor..].find('}').ok_or(ErrorKind::Value)?;
                let end = cursor + relative_end;
                let name = &replacement[cursor..end];
                cursor = end + 1;
                append_named_capture(engine, context, output, named_lookup, name, true)
            }
            '<' => {
                let relative_end = replacement[cursor..].find('>').ok_or(ErrorKind::Value)?;
                let end = cursor + relative_end;
                let name = &replacement[cursor..end];
                cursor = end + 1;
                append_named_capture(engine, context, output, named_lookup, name, false)
            }
            _ => Err(ErrorKind::Value),
        }?;
    }
    push_bounded(engine, output, &replacement[cursor..])
}

fn append_named_capture(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    output: &mut String,
    lookup: NamedCaptureLookup<'_>,
    name: &str,
    allow_number: bool,
) -> Result<(), ErrorKind> {
    if allow_number && let Ok(index) = name.parse::<usize>() {
        return append_capture(engine, output, lookup.subject, lookup.captures, index);
    }
    if name == "*MARK" {
        return Err(ErrorKind::Value);
    }
    let indexes = lookup.name_indexes.get(name).ok_or(ErrorKind::Value)?;
    engine.charge_function_iterations(
        context,
        u64::try_from(indexes.len()).map_err(|_| ErrorKind::Num)?,
    )?;
    let first = *indexes.first().ok_or(ErrorKind::Value)?;
    let index = indexes
        .iter()
        .copied()
        .find(|index| lookup.captures.span(*index).is_some())
        .unwrap_or(first);
    append_capture(engine, output, lookup.subject, lookup.captures, index)
}

fn append_capture(
    engine: &Engine<'_>,
    output: &mut String,
    subject: &str,
    captures: &RegexCaptureSet,
    index: usize,
) -> Result<(), ErrorKind> {
    let (start, end) = captures.span(index).ok_or(ErrorKind::Value)?;
    push_bounded(
        engine,
        output,
        subject.get(start..end).ok_or(ErrorKind::Value)?,
    )
}

fn case_option(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expression: Option<&Expr>,
) -> Result<bool, ErrorKind> {
    match integer_option(engine, context, expression, 0)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ErrorKind::Value),
    }
}

fn integer_option(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expression: Option<&Expr>,
    default: i64,
) -> Result<i64, ErrorKind> {
    let Some(expression) = expression else {
        return Ok(default);
    };
    if matches!(expression, Expr::Missing) {
        return Ok(default);
    }
    let number = to_number(&engine.eval_scalar(context, expression))?.trunc();
    if number < i64::MIN as f64 || number >= i64::MAX as f64 {
        return Err(ErrorKind::Value);
    }
    Ok(number as i64)
}
