use super::reference::is_reference_returning_function;
use super::{Engine, EvalContext};
use crate::calculation::ast::Expr;
use crate::calculation::functions::{call_function, call_function_array};
use crate::calculation::operators::{apply_binary, apply_unary, lift_binary};
use crate::calculation::runtime::{Array, Rect};
use crate::calculation::value::{ErrorKind, Value};

impl Engine<'_> {
    fn eval_implicit_intersection(&self, context: EvalContext<'_>, expr: &Expr) -> Value {
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.eval_implicit_intersection(context, inner)
            }
            Expr::Ref(reference) => self
                .resolve_reference(context.sheet(), reference)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            Expr::Range { .. } => self
                .resolve_rect_expr(context, expr)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            Expr::Name(name) => match self.resolve_name_expr(context.sheet(), name) {
                Some(named) => self.eval_implicit_intersection(context, named),
                None => Value::Error(ErrorKind::Name),
            },
            Expr::Call { name, .. } if is_reference_returning_function(name) => self
                .resolve_rect_expr(context, expr)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            _ => self
                .eval_array(context, expr)
                .map_or_else(Value::Error, |array| {
                    array
                        .data
                        .into_iter()
                        .next()
                        .unwrap_or(Value::Error(ErrorKind::Value))
                }),
        }
    }

    pub fn eval_scalar(&self, context: EvalContext<'_>, expr: &Expr) -> Value {
        match expr {
            Expr::Number(number) => Value::Number(*number),
            Expr::Text(text) => Value::Text(text.clone()),
            Expr::Logical(logical) => Value::Logical(*logical),
            Expr::ErrorLit(kind) => Value::Error(*kind),
            Expr::Missing => Value::Blank,
            Expr::Paren(inner) => self.eval_scalar(context, inner),
            Expr::ImplicitIntersection(inner) => self.eval_implicit_intersection(context, inner),
            Expr::Array(_) => Value::Error(ErrorKind::Unsupported),
            Expr::Name(name) => match context.binding(name) {
                Some(value) => value.clone(),
                None => match self.resolve_name_expr(context.sheet(), name) {
                    Some(named) => self.eval_scalar(context, named),
                    None => Value::Error(ErrorKind::Name),
                },
            },
            Expr::Ref(_) | Expr::Range { .. } => self.eval_implicit_intersection(context, expr),
            Expr::Call { name, .. } if is_reference_returning_function(name) => {
                self.eval_implicit_intersection(context, expr)
            }
            Expr::Call { name, args } => call_function(self, context, name, args),
            Expr::Unary { op, operand } => apply_unary(*op, &self.eval_scalar(context, operand)),
            Expr::Binary { op, left, right } => apply_binary(
                *op,
                &self.eval_scalar(context, left),
                &self.eval_scalar(context, right),
                self.options.limits().max_text_bytes(),
            ),
        }
    }

    pub fn eval_array(&self, context: EvalContext<'_>, expr: &Expr) -> Result<Array, ErrorKind> {
        match expr {
            Expr::Paren(inner) => self.eval_array(context, inner),
            Expr::ImplicitIntersection(inner) => Ok(Array::scalar(
                self.eval_implicit_intersection(context, inner),
            )),
            Expr::Name(name) if context.binding(name).is_some() => Ok(Array::scalar(
                context
                    .binding(name)
                    .cloned()
                    .expect("binding presence checked"),
            )),
            Expr::Ref(_) | Expr::Range { .. } | Expr::Name(_) => {
                let rect = self.resolve_rect_expr(context, expr)?;
                self.array_from_rect(rect)
            }
            Expr::Array(rows) => {
                let cols = rows.first().map_or(0, Vec::len);
                if rows.is_empty() || cols == 0 || rows.iter().any(|row| row.len() != cols) {
                    return Err(ErrorKind::Value);
                }
                let cell_count = (rows.len() as u64) * (cols as u64);
                self.ensure_array_cells(cell_count)?;
                let data = rows
                    .iter()
                    .flat_map(|row| row.iter().map(|value| self.eval_scalar(context, value)))
                    .collect();
                Ok(Array {
                    rows: rows.len() as u32,
                    cols: cols as u32,
                    data,
                })
            }
            Expr::Binary { op, left, right } => lift_binary(
                *op,
                &self.eval_array(context, left)?,
                &self.eval_array(context, right)?,
                self.options.limits().max_text_bytes(),
                self.options.limits().max_array_cells(),
            ),
            Expr::Unary { op, operand } => {
                let array = self.eval_array(context, operand)?;
                let data = array
                    .data
                    .iter()
                    .map(|value| apply_unary(*op, value))
                    .collect();
                Ok(Array {
                    rows: array.rows,
                    cols: array.cols,
                    data,
                })
            }
            Expr::Call { name, args } => {
                if let Some(result) = call_function_array(self, context, name, args) {
                    result
                } else {
                    Ok(Array::scalar(call_function(self, context, name, args)))
                }
            }
            _ => Ok(Array::scalar(self.eval_scalar(context, expr))),
        }
    }

    pub(in crate::calculation) fn array_from_rect(&self, rect: Rect) -> Result<Array, ErrorKind> {
        if rect.is_single_cell() {
            return Ok(Array::scalar(self.cell_value((
                rect.sheet,
                rect.row_start,
                rect.col_start,
            ))));
        }
        if rect.whole_rows {
            return Err(ErrorKind::Unsupported);
        }
        let cells = rect.height() * rect.width();
        self.ensure_array_cells(cells)?;
        let mut data = Vec::with_capacity(cells as usize);
        for row in rect.row_start..=rect.row_end {
            for column in rect.col_start..=rect.col_end {
                data.push(self.cell_value((rect.sheet, row, column)));
            }
        }
        Ok(Array {
            rows: rect.height() as u32,
            cols: rect.width() as u32,
            data,
        })
    }
}
