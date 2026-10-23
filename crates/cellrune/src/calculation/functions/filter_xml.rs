//! FILTERXML(xml, xpath): the nodes an XPath 1.0 expression selects from an XML document.
//!
//! Microsoft documents `#VALUE!` for text that is not valid XML or that uses a namespace prefix
//! it does not declare, and its example fills one column with the selected nodes. FILTERXML is
//! Windows only, so the rest rests on XML 1.0 with namespaces and XPath 1.0: each selected node
//! gives its string value, in document order. Excel does not document whether a value that reads
//! as a number becomes one, what an empty selection or a value that is not a node set returns, how
//! a DTD or white-space-only text is handled, or how an expression that is not XPath fails, so
//! those are unsupported.

mod document;
mod evaluate;
#[cfg(test)]
mod tests;
mod xpath;

use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::util::required_text;
use document::{Document, DocumentError};
use evaluate::{EvalError, Evaluator, XValue};
use xpath::ParseError;

pub(super) fn call_scalar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match call_array(engine, context, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Unsupported)),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let [xml, path] = args else {
        return Err(ErrorKind::Value);
    };
    let xml = required_text(engine, context, xml)?;
    let path = required_text(engine, context, path)?;
    engine.charge_function_iterations(context, (xml.len() + path.len()) as u64)?;
    let document = match Document::parse(&xml) {
        Ok(document) => document,
        Err(DocumentError::Invalid) => return Err(ErrorKind::Value),
        Err(DocumentError::Unsupported) => return Err(ErrorKind::Unsupported),
    };
    let expr = xpath::parse(&path).map_err(|error| match error {
        ParseError::Syntax | ParseError::Unsupported => ErrorKind::Unsupported,
    })?;
    let mut budget_error = None;
    let mut evaluator = Evaluator::new(&document, |units| {
        let charged = poll_cancellation(context)
            .and_then(|()| engine.charge_function_iterations(context, units));
        charged.map_err(|kind| {
            budget_error = Some(kind);
            EvalError::Budget
        })
    });
    let result = evaluator.evaluate(&expr).and_then(|value| match value {
        XValue::Nodes(nodes) if !nodes.is_empty() => {
            evaluator.observe(&nodes)?;
            nodes
                .iter()
                .map(|node| evaluator.string_value(*node))
                .collect::<Result<Vec<_>, _>>()
        }
        _ => Err(EvalError::Unsupported),
    });
    let values = match result {
        Ok(values) => values,
        Err(EvalError::Budget) => {
            return Err(budget_error.unwrap_or(ErrorKind::Unsupported));
        }
        Err(EvalError::Unsupported) => return Err(ErrorKind::Unsupported),
    };
    let mut data = Vec::with_capacity(values.len());
    for value in values {
        // Whether Excel turns a value that reads as a number, or an empty one, into a number is
        // not documented.
        if value.is_empty() || to_number(&Value::Text(value.clone())).is_ok() {
            return Err(ErrorKind::Unsupported);
        }
        engine.ensure_text_bytes(value.len())?;
        data.push(Value::Text(value));
    }
    Ok(Array {
        rows: u32::try_from(data.len()).map_err(|_| ErrorKind::Unsupported)?,
        cols: 1,
        data,
    })
}
