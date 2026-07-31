use std::collections::BTreeMap;

use super::{Engine, EvalContext};
use crate::Sheet;
use crate::calculation::ast::{Expr, RefBody, Reference};
use crate::calculation::coerce::{to_logical, to_number, to_text};
use crate::calculation::functions::{callable_call_scope, let_reference, normalize_name};
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::runtime::{Rect, RectSpan, SheetSpan};
use crate::calculation::scope::ScopeValue;
use crate::calculation::value::ErrorKind;
use crate::calculation::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

pub(super) fn cell_at(sheet: &Sheet, row: u32, column: u32) -> Option<&crate::Cell> {
    let address = crate::CellAddress::from_indices(row, column).ok()?;
    sheet.cell(address)
}

fn used_rows(sheet: &Sheet) -> u32 {
    sheet
        .used_range()
        .map_or(0, |range| range.end().row().get())
}

/// The greatest populated row of each populated column on one sheet.
///
/// A whole-column *array* operand clamps to the columns it actually names rather than to the
/// sheet-wide used range. The sheet-wide range would make the materialized height depend on cells
/// that no dependency rectangle covers, so an edit in an unreferenced column would change the
/// correct answer without dirtying the formula and full and incremental recalculation would
/// disagree. Clamping per column keeps the value a function of the recorded dependencies alone.
#[derive(Debug, Clone, Default)]
pub(in crate::calculation) struct ColumnExtents {
    rows_by_column: BTreeMap<u32, u32>,
}

impl ColumnExtents {
    pub(super) fn record(&mut self, column: u32, row: u32) {
        self.rows_by_column
            .entry(column)
            .and_modify(|current| *current = (*current).max(row))
            .or_insert(row);
    }

    fn row_end_within(&self, col_start: u32, col_end: u32) -> u32 {
        self.rows_by_column
            .range(col_start..=col_end)
            .map(|(_, row)| *row)
            .max()
            .unwrap_or(0)
    }
}

pub(super) fn is_reference_returning_function(name: &str) -> bool {
    matches!(
        normalize_name(name).as_str(),
        "INDEX" | "INDIRECT" | "OFFSET"
    )
}

impl Engine<'_> {
    pub(in crate::calculation) fn resolve_reference_span(
        &self,
        current_sheet: usize,
        reference: &Reference,
    ) -> Result<RectSpan, ErrorKind> {
        let (start_sheet, end_sheet) = match &reference.sheet {
            // An external workbook is an unsupported capability, not a missing sheet. Returning
            // `Ref` here would let `IFERROR` hide it and would disagree with the capability scan.
            Some(prefix) if prefix.external_workbook_detail().is_some() => {
                return Err(ErrorKind::Unsupported);
            }
            Some(prefix) => {
                let start = self
                    .workbook
                    .sheet_index_by_name(&prefix.name)
                    .ok_or(ErrorKind::Ref)?;
                let end = match &prefix.end_name {
                    Some(name) => self
                        .workbook
                        .sheet_index_by_name(name)
                        .ok_or(ErrorKind::Ref)?,
                    None => start,
                };
                (start, end)
            }
            None => (current_sheet, current_sheet),
        };
        let (row_start, col_start, row_end, col_end, whole_rows) =
            reference_bounds(&reference.body);
        let sheets = reference.sheet.as_ref().map_or_else(
            || SheetSpan::single(start_sheet),
            |prefix| {
                if prefix.end_name.is_some() {
                    SheetSpan::new(start_sheet, end_sheet)
                } else {
                    SheetSpan::single(start_sheet)
                }
            },
        );
        Ok(RectSpan::new(
            sheets,
            Rect {
                sheet: start_sheet,
                row_start,
                col_start,
                row_end,
                col_end,
                whole_rows,
            },
        ))
    }

    pub fn resolve_reference(
        &self,
        current_sheet: usize,
        reference: &Reference,
    ) -> Result<Rect, ErrorKind> {
        let span = self.resolve_reference_span(current_sheet, reference)?;
        if span.is_sheet_range() {
            return Err(ErrorKind::Unsupported);
        }
        span.into_rect().map_err(|_| ErrorKind::Unsupported)
    }

    pub(in crate::calculation) fn resolve_rect_span_expr(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<RectSpan, ErrorKind> {
        match expr {
            Expr::Paren(inner) => self.resolve_rect_span_expr(context, inner),
            Expr::Ref(reference) => self.resolve_reference_span(context.sheet(), reference),
            Expr::Range { .. } => {
                let rect = self.resolve_rect_expr(context, expr)?;
                Ok(RectSpan::new(SheetSpan::single(rect.sheet), rect))
            }
            Expr::Name(name) => match context.binding(name) {
                Some(ScopeValue::Reference(span)) => Ok(span.clone()),
                Some(_) => Err(ErrorKind::Value),
                None => self
                    .resolve_name_expr_with_id_in_context(context, name)
                    .ok_or(ErrorKind::Name)
                    .and_then(|(id, named)| {
                        self.resolve_rect_span_expr(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            named,
                        )
                    }),
            },
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return match scoped {
                        ScopeValue::Reference(span) => Ok(span),
                        _ => Err(ErrorKind::Value),
                    };
                }
                if normalize_name(name) == "LET" {
                    let_reference(self, context, args)
                } else {
                    self.resolve_rect_expr(context, expr).map(RectSpan::single)
                }
            }
            _ => Err(ErrorKind::Value),
        }
    }

    pub fn resolve_rect_expr(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<Rect, ErrorKind> {
        match expr {
            Expr::Paren(inner) => self.resolve_rect_expr(context, inner),
            Expr::ImplicitIntersection(inner) => self
                .resolve_rect_expr(context, inner)
                .and_then(|rect| self.implicit_intersection_rect(context, rect)),
            Expr::Ref(reference) => self.resolve_reference(context.sheet(), reference),
            Expr::Range { start, end } => {
                // A sheet span is not a rectangle the range operator can join. Excel reports the
                // same `#VALUE!` it gives a range whose endpoints sit on different sheets, and the
                // capability scanner classifies this position with `ARRAY_EXPRESSION_POLICY`, so
                // answering with the engine-capability `Unsupported` here would make the scanner
                // and the evaluator disagree.
                let start_span = self.resolve_rect_span_expr(context, start)?;
                let end_span = self.resolve_rect_span_expr(context, end)?;
                if start_span.is_sheet_range() || end_span.is_sheet_range() {
                    return Err(ErrorKind::Value);
                }
                let start = start_span.into_rect().map_err(|_| ErrorKind::Value)?;
                let end = end_span.into_rect().map_err(|_| ErrorKind::Value)?;
                if start.sheet != end.sheet {
                    // Excel yields #VALUE! for a range operator whose endpoints
                    // sit on different sheets.
                    return Err(ErrorKind::Value);
                }
                let row_start = start.row_start.min(end.row_start);
                let row_end = start.row_end.max(end.row_end);
                Ok(Rect {
                    sheet: start.sheet,
                    row_start,
                    col_start: start.col_start.min(end.col_start),
                    row_end,
                    col_end: start.col_end.max(end.col_end),
                    whole_rows: row_start == 1
                        && row_end == EXCEL_MAX_ROWS
                        && (start.whole_rows || end.whole_rows),
                })
            }
            Expr::Name(name) => match context.binding(name) {
                Some(ScopeValue::Reference(span)) => {
                    span.clone().into_rect().map_err(|_| ErrorKind::Value)
                }
                Some(_) => Err(ErrorKind::Value),
                None => self
                    .resolve_name_expr_with_id_in_context(context, name)
                    .ok_or(ErrorKind::Name)
                    .and_then(|(id, named)| {
                        self.resolve_rect_expr(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            named,
                        )
                    }),
            },
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return match scoped {
                        ScopeValue::Reference(span) => {
                            span.into_rect().map_err(|_| ErrorKind::Value)
                        }
                        _ => Err(ErrorKind::Value),
                    };
                }
                match normalize_name(name).as_str() {
                    "LET" => let_reference(self, context, args)?
                        .into_rect()
                        .map_err(|_| ErrorKind::Value),
                    "INDEX" => self.resolve_index_rect(context, args),
                    _ => self.resolve_dynamic_rect(context, name, args),
                }
            }
            _ => Err(ErrorKind::Value),
        }
    }

    pub(in crate::calculation) fn resolve_index_rect(
        &self,
        context: EvalContext<'_>,
        args: &[Expr],
    ) -> Result<Rect, ErrorKind> {
        if args.len() < 2 || args.len() > 3 {
            return Err(ErrorKind::Value);
        }
        let span = self.resolve_rect_span_expr(context, &args[0])?;
        if span.is_sheet_range() {
            return Err(ErrorKind::Value);
        }
        let rect = span.into_rect().map_err(|_| ErrorKind::Value)?;
        let first_index = to_number(&self.eval_scalar(context, &args[1]))?.trunc();
        let second_index = match args.get(2) {
            Some(Expr::Missing) | None => None,
            Some(expr) => Some(to_number(&self.eval_scalar(context, expr))?.trunc()),
        };
        let single_row = rect.height() == 1 && rect.width() > 1;
        let (row_index, col_index) = if single_row && second_index.is_none() {
            (1.0, first_index)
        } else {
            (first_index, second_index.unwrap_or(1.0))
        };
        if row_index < 0.0 || col_index < 0.0 {
            return Err(ErrorKind::Value);
        }
        if row_index as u64 > rect.height() || col_index as u64 > rect.width() {
            return Err(ErrorKind::Ref);
        }

        let (row_start, row_end) = if row_index == 0.0 {
            (rect.row_start, rect.row_end)
        } else {
            let row = rect.row_start + row_index as u32 - 1;
            (row, row)
        };
        let (col_start, col_end) = if col_index == 0.0 {
            (rect.col_start, rect.col_end)
        } else {
            let column = rect.col_start + col_index as u32 - 1;
            (column, column)
        };
        Ok(Rect {
            sheet: rect.sheet,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows: rect.whole_rows && row_start == 1 && row_end == EXCEL_MAX_ROWS,
        })
    }

    pub(super) fn resolve_dynamic_rect(
        &self,
        context: EvalContext<'_>,
        name: &str,
        args: &[Expr],
    ) -> Result<Rect, ErrorKind> {
        let normalized = normalize_name(name);
        if normalized.eq_ignore_ascii_case("INDIRECT") {
            if args.is_empty() || args.len() > 2 {
                return Err(ErrorKind::Value);
            }
            if let Some(style) = args.get(1)
                && !to_logical(&self.eval_scalar(context, style))?
            {
                return Err(ErrorKind::Unsupported);
            }
            let formula = to_text(&self.eval_scalar(context, &args[0]))?;
            let parsed =
                parse_formula_with_limits(&formula, self.options.limits()).map_err(|error| {
                    match error.limit {
                        Some(limit) => ErrorKind::ResourceLimit(limit),
                        None => ErrorKind::Ref,
                    }
                })?;
            // The capability scan cannot look inside this text, so an unsupported reference form
            // built here is one it reported as supported. Surfacing it as an engine issue would
            // make the workbook yield unavailable cells that the scan promised were calculable,
            // and no `IFERROR` could recover. Excel answers `#REF!` for text it cannot resolve to
            // a reference, so degrade to that; a resource limit is a genuine engine outcome and
            // still propagates.
            return self
                .resolve_rect_expr(context, parsed.root())
                .map_err(|error| match error {
                    resource @ ErrorKind::ResourceLimit(_) => resource,
                    _ => ErrorKind::Ref,
                });
        }
        if !normalized.eq_ignore_ascii_case("OFFSET") || args.len() < 3 || args.len() > 5 {
            return Err(ErrorKind::Value);
        }
        let span = self.resolve_rect_span_expr(context, &args[0])?;
        if span.is_sheet_range() {
            return Err(ErrorKind::Ref);
        }
        let base = span.into_rect().map_err(|_| ErrorKind::Ref)?;
        let rows = to_number(&self.eval_scalar(context, &args[1]))?.trunc() as i64;
        let columns = to_number(&self.eval_scalar(context, &args[2]))?.trunc() as i64;
        let height = match args.get(3) {
            Some(Expr::Missing) | None => base.height() as i64,
            Some(expr) => to_number(&self.eval_scalar(context, expr))?.trunc() as i64,
        };
        let width = match args.get(4) {
            Some(Expr::Missing) | None => base.width() as i64,
            Some(expr) => to_number(&self.eval_scalar(context, expr))?.trunc() as i64,
        };
        let Some(row_start) = i64::from(base.row_start).checked_add(rows) else {
            return Err(ErrorKind::Ref);
        };
        let Some(col_start) = i64::from(base.col_start).checked_add(columns) else {
            return Err(ErrorKind::Ref);
        };
        let Some(row_end) = height
            .checked_sub(1)
            .and_then(|offset| row_start.checked_add(offset))
        else {
            return Err(ErrorKind::Ref);
        };
        let Some(col_end) = width
            .checked_sub(1)
            .and_then(|offset| col_start.checked_add(offset))
        else {
            return Err(ErrorKind::Ref);
        };
        if height <= 0
            || width <= 0
            || row_start < 1
            || col_start < 1
            || row_end > i64::from(EXCEL_MAX_ROWS)
            || col_end > i64::from(EXCEL_MAX_COLUMNS)
        {
            return Err(ErrorKind::Ref);
        }
        Ok(Rect {
            sheet: base.sheet,
            row_start: row_start as u32,
            col_start: col_start as u32,
            row_end: row_end as u32,
            col_end: col_end as u32,
            whole_rows: false,
        })
    }

    pub fn clamped_row_end(&self, rect: &Rect) -> u32 {
        if rect.whole_rows {
            rect.row_end
                .min(used_rows(&self.workbook.sheets()[rect.sheet]))
        } else {
            rect.row_end
        }
    }

    /// Clamps a whole-column rect to the greatest populated row inside its own columns.
    ///
    /// Whole-column array materialization uses this instead of [`Self::clamped_row_end`] so that
    /// the resulting height, and therefore the value, depends only on the columns the expression
    /// references. See [`ColumnExtents`].
    pub(in crate::calculation) fn whole_column_row_end(&self, rect: &Rect) -> u32 {
        if !rect.whole_rows {
            return rect.row_end;
        }
        let used = self.column_extents.get(rect.sheet).map_or(0, |extents| {
            extents.row_end_within(rect.col_start, rect.col_end)
        });
        rect.row_end.min(used)
    }

    pub(in crate::calculation) fn implicit_intersection_rect(
        &self,
        context: EvalContext<'_>,
        rect: Rect,
    ) -> Result<Rect, ErrorKind> {
        if rect.is_single_cell() {
            return Ok(rect);
        }
        if rect.width() == 1 && (rect.row_start..=rect.row_end).contains(&context.row()) {
            return Ok(Rect {
                sheet: rect.sheet,
                row_start: context.row(),
                col_start: rect.col_start,
                row_end: context.row(),
                col_end: rect.col_start,
                whole_rows: false,
            });
        }
        if rect.height() == 1 && (rect.col_start..=rect.col_end).contains(&context.column()) {
            return Ok(Rect {
                sheet: rect.sheet,
                row_start: rect.row_start,
                col_start: context.column(),
                row_end: rect.row_start,
                col_end: context.column(),
                whole_rows: false,
            });
        }
        Err(ErrorKind::Value)
    }
}

fn reference_bounds(body: &RefBody) -> (u32, u32, u32, u32, bool) {
    match body {
        RefBody::Cell(cell) => (cell.row, cell.column, cell.row, cell.column, false),
        RefBody::Area(start, end) => (
            start.row.min(end.row),
            start.column.min(end.column),
            start.row.max(end.row),
            start.column.max(end.column),
            false,
        ),
        RefBody::Columns(start, end) => (
            1,
            start.column.min(end.column),
            EXCEL_MAX_ROWS,
            start.column.max(end.column),
            true,
        ),
        RefBody::Rows(start, end) => (
            start.row.min(end.row),
            1,
            start.row.max(end.row),
            EXCEL_MAX_COLUMNS,
            false,
        ),
    }
}
