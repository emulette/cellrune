use std::collections::BTreeMap;

use super::{Engine, EvalContext};
use crate::Sheet;
use crate::calculation::ast::{Expr, Reference, StructuredReference};
use crate::calculation::coerce::{to_logical, to_number, to_text};
use crate::calculation::functions::descriptor::{DependencyKind, DynamicReferenceKind};
use crate::calculation::functions::kernel::{LegacyFunction, LookupFunction};
use crate::calculation::functions::{
    DynamicFunction, Evaluator, callable_call_scope, function_call_shape_is_valid,
    function_dependency_kind, function_evaluator, let_reference, prepare_evaluator_arguments,
};
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::reference_resolution::{
    intersect_reference_values, intersection_reference_work, range_reference_rect,
    resolve_reference_span, resolve_structured_reference, structured_table_coordinates,
    union_reference_values,
};
use crate::calculation::runtime::{Rect, RectSpan, ReferenceValue};
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

impl Engine<'_> {
    pub(in crate::calculation) fn resolve_reference_span(
        &self,
        current_sheet: usize,
        reference: &Reference,
    ) -> Result<RectSpan, ErrorKind> {
        resolve_reference_span(self.workbook, current_sheet, reference)
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

    fn validate_reference_value(
        &self,
        reference: ReferenceValue,
    ) -> Result<ReferenceValue, ErrorKind> {
        let area_count = u64::try_from(reference.area_count())
            .map_err(|_| ErrorKind::ResourceLimit(CalculationLimitKind::ReferenceAreas))?;
        if area_count > self.options.limits().max_reference_areas() {
            return Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::ReferenceAreas,
            ));
        }
        let mut sheet = None;
        for area in reference.areas() {
            match area {
                crate::calculation::runtime::ReferenceArea::Rect(rect) => match sheet {
                    Some(current) if current != rect.sheet => return Err(ErrorKind::Value),
                    Some(_) => {}
                    None => sheet = Some(rect.sheet),
                },
                crate::calculation::runtime::ReferenceArea::SheetSpan(_)
                    if reference.area_count() > 1 =>
                {
                    return Err(ErrorKind::Value);
                }
                crate::calculation::runtime::ReferenceArea::SheetSpan(_) => {}
            }
        }
        Ok(reference)
    }

    pub(super) fn structured_table_coordinates(
        &self,
        context: EvalContext<'_>,
        reference: &StructuredReference,
    ) -> Result<(usize, usize), ErrorKind> {
        structured_table_coordinates(
            self.workbook,
            (context.sheet(), context.row(), context.column()),
            reference,
        )
    }

    fn resolve_structured_reference(
        &self,
        context: EvalContext<'_>,
        reference: &StructuredReference,
    ) -> Result<ReferenceValue, ErrorKind> {
        resolve_structured_reference(
            self.workbook,
            (context.sheet(), context.row(), context.column()),
            reference,
        )
    }

    pub(in crate::calculation) fn resolve_reference_value_expr(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<ReferenceValue, ErrorKind> {
        let reference = match expr {
            Expr::Paren(inner) => return self.resolve_reference_value_expr(context, inner),
            Expr::Ref(reference) => {
                ReferenceValue::from_span(self.resolve_reference_span(context.sheet(), reference)?)
            }
            Expr::StructuredRef(reference) => {
                self.resolve_structured_reference(context, reference)?
            }
            Expr::SpillRef(anchor) => {
                ReferenceValue::from_rect(self.resolve_spill_reference(context, anchor)?)
            }
            Expr::ReferenceUnion { left, right } => {
                let left = self.resolve_reference_value_expr(context, left)?;
                let right = self.resolve_reference_value_expr(context, right)?;
                union_reference_values(&left, &right)?
            }
            Expr::ReferenceIntersection { left, right } => {
                let left = self.resolve_reference_value_expr(context, left)?;
                let right = self.resolve_reference_value_expr(context, right)?;
                let comparisons = intersection_reference_work(&left, &right)?;
                self.ensure_function_iterations(comparisons)?;
                if context.charges_reference_work() {
                    self.charge_function_iterations(context, comparisons)?;
                }
                let max_areas = self.options.limits().max_reference_areas();
                intersect_reference_values(&left, &right, max_areas, || {
                    if context.is_cancelled() {
                        Err(ErrorKind::ResourceLimit(
                            CalculationLimitKind::FunctionIterations,
                        ))
                    } else {
                        Ok(())
                    }
                })?
            }
            Expr::Range { .. } => ReferenceValue::from_rect(self.resolve_rect_expr(context, expr)?),
            Expr::Name(name) => match context.binding(name) {
                Some(ScopeValue::Reference(reference)) => reference.clone(),
                Some(_) => return Err(ErrorKind::Value),
                None => self
                    .resolve_name_expr_with_id_in_context(context, name)
                    .ok_or(ErrorKind::Name)
                    .and_then(|(id, named)| {
                        self.resolve_reference_value_expr(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            named,
                        )
                    })?,
            },
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    match scoped {
                        ScopeValue::Reference(reference) => reference,
                        _ => return Err(ErrorKind::Value),
                    }
                } else if function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Let))
                {
                    let_reference(self, context, args)?
                } else {
                    ReferenceValue::from_rect(self.resolve_rect_expr(context, expr)?)
                }
            }
            Expr::ErrorLit(kind) => return Err(*kind),
            Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::ImplicitIntersection(_)
            | Expr::Invoke { .. }
            | Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Array(_)
            | Expr::Missing => return Err(ErrorKind::Value),
        };
        self.validate_reference_value(reference)
    }

    pub(super) fn resolve_spill_anchor_expr(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<super::CellId, ErrorKind> {
        let rect = self
            .resolve_reference_value_expr(context, expr)?
            .single_rect()?;
        if !rect.is_single_cell() {
            return Err(ErrorKind::Ref);
        }
        Ok((rect.sheet, rect.row_start, rect.col_start))
    }

    fn resolve_spill_reference(
        &self,
        context: EvalContext<'_>,
        anchor: &Expr,
    ) -> Result<Rect, ErrorKind> {
        let anchor = self.resolve_spill_anchor_expr(context, anchor)?;
        if let Some(range) = self.dynamic_spill(anchor) {
            return Ok(range);
        }
        if self.dynamic_array_range(anchor).is_none() {
            return Err(ErrorKind::Ref);
        }
        match self.cell_value(anchor) {
            crate::calculation::value::Value::Error(kind) => Err(kind),
            _ => Err(ErrorKind::Ref),
        }
    }

    pub(in crate::calculation) fn resolve_rect_span_expr(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<RectSpan, ErrorKind> {
        self.resolve_reference_value_expr(context, expr)?
            .single_area_span()
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
            Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. } => self
                .resolve_reference_value_expr(context, expr)?
                .into_single_rect(),
            Expr::Range { start, end } => {
                // A sheet span is not a rectangle the range operator can join. Excel reports the
                // same `#VALUE!` it gives a range whose endpoints sit on different sheets, and the
                // capability scanner classifies this position with `ARRAY_EXPRESSION_POLICY`, so
                // answering with the engine-capability `Unsupported` here would make the scanner
                // and the evaluator disagree.
                let start = self.resolve_reference_value_expr(context, start)?;
                let end = self.resolve_reference_value_expr(context, end)?;
                range_reference_rect(&start, &end)
            }
            Expr::Name(name) => match context.binding(name) {
                Some(ScopeValue::Reference(reference)) => reference.clone().into_single_rect(),
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
                        ScopeValue::Reference(reference) => reference.into_single_rect(),
                        _ => Err(ErrorKind::Value),
                    };
                }
                if function_evaluator(name).is_some() && !function_call_shape_is_valid(name, args) {
                    return Err(ErrorKind::Value);
                }
                match function_evaluator(name) {
                    Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
                        let_reference(self, context, args)?.into_single_rect()
                    }
                    Some(Evaluator::Legacy(LegacyFunction::Index)) => {
                        self.resolve_index_rect(context, args)
                    }
                    _ => match function_dependency_kind(name) {
                        Some(DependencyKind::DynamicReference(kind)) => {
                            self.resolve_dynamic_rect(context, kind, args)
                        }
                        _ => Err(ErrorKind::Value),
                    },
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
        let prepared = prepare_evaluator_arguments(Evaluator::Legacy(LegacyFunction::Index), args)
            .ok_or(ErrorKind::Value)?;
        let args = prepared.as_ref();
        let reference = self.resolve_reference_value_expr(context, &args[0])?;
        let area_index = match args.get(3) {
            Some(Expr::Missing) | None => 1.0,
            Some(expr) => to_number(&self.eval_scalar(context, expr))?.trunc(),
        };
        if !area_index.is_finite() || area_index < 1.0 {
            return Err(ErrorKind::Value);
        }
        let span = reference.area_span(area_index as usize)?;
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

    pub(in crate::calculation) fn resolve_dynamic_rect(
        &self,
        context: EvalContext<'_>,
        dynamic_kind: DynamicReferenceKind,
        args: &[Expr],
    ) -> Result<Rect, ErrorKind> {
        let evaluator = match dynamic_kind {
            DynamicReferenceKind::Indirect => Evaluator::Lookup(LookupFunction::Indirect),
            DynamicReferenceKind::Offset => Evaluator::Lookup(LookupFunction::Offset),
        };
        let prepared = prepare_evaluator_arguments(evaluator, args).ok_or(ErrorKind::Value)?;
        let args = prepared.as_ref();
        if dynamic_kind == DynamicReferenceKind::Indirect {
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
        if dynamic_kind != DynamicReferenceKind::Offset {
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
