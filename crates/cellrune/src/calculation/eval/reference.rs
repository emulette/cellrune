use std::collections::BTreeMap;
use std::iter::Peekable;
use std::ops::RangeInclusive;

use super::{Engine, EvalContext};
use crate::Sheet;
use crate::calculation::ast::{Expr, Reference, StructuredReference};
use crate::calculation::coerce::{to_logical, to_number, to_text};
use crate::calculation::functions::descriptor::{DependencyKind, DynamicReferenceKind};
use crate::calculation::functions::kernel::{LegacyFunction, LookupFunction};
use crate::calculation::functions::{
    DynamicFunction, Evaluator, callable_call_scope, evaluator_value_argument_holds_array,
    function_call_shape_is_valid, function_dependency_kind, function_evaluator,
    index_arguments_hold_arrays, let_reference, may_hold_array, prepare_evaluator_arguments,
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
use crate::workbook::CellsIn;

pub(super) fn cell_at(sheet: &Sheet, row: u32, column: u32) -> Option<&crate::Cell> {
    let address = crate::CellAddress::from_indices(row, column).ok()?;
    sheet.cell(address)
}

/// The cell a sheet stores at a position: `None` when no sheet has the position's sheet index,
/// and `Some(None)` when the sheet stores no cell there.
pub(in crate::calculation) type StoredCell<'workbook> = Option<Option<&'workbook crate::Cell>>;

/// The stored cells of a rectangle, found in one row-major walk for positions read in that
/// order instead of one lookup each.
pub(in crate::calculation) struct RectCells<'workbook> {
    sheet: Option<&'workbook Sheet>,
    rows: RangeInclusive<u32>,
    columns: RangeInclusive<u32>,
    cells: Option<Peekable<CellsIn<'workbook>>>,
    /// The last position read from the walk and its cell.
    last: Option<((u32, u32), Option<&'workbook crate::Cell>)>,
}

impl<'workbook> RectCells<'workbook> {
    /// The stored cell at `row` and `column`. Positions in the rectangle come from the walk and
    /// must not come before a position read before; others are looked up.
    pub(in crate::calculation) fn at(&mut self, row: u32, column: u32) -> StoredCell<'workbook> {
        let sheet = self.sheet?;
        if !self.rows.contains(&row) || !self.columns.contains(&column) {
            return Some(cell_at(sheet, row, column));
        }
        if let Some((position, cell)) = self.last {
            if position == (row, column) {
                return Some(cell);
            }
            debug_assert!(
                position < (row, column),
                "rectangle cells read out of order"
            );
        }
        let position =
            |cell: &&crate::Cell| (cell.address().row().get(), cell.address().column().get());
        let cells = self.cells.get_or_insert_with(|| {
            sheet
                .cells_in(self.rows.clone(), self.columns.clone())
                .peekable()
        });
        while cells
            .next_if(|cell| position(cell) < (row, column))
            .is_some()
        {}
        let cell = cells.next_if(|cell| position(cell) == (row, column));
        self.last = Some(((row, column), cell));
        Some(cell)
    }
}

impl<'workbook> Engine<'workbook> {
    /// The stored cells of `rect` through `row_end`.
    pub(in crate::calculation) fn rect_cells(
        &self,
        rect: &Rect,
        row_end: u32,
    ) -> RectCells<'workbook> {
        RectCells {
            sheet: self.workbook.sheets().get(rect.sheet),
            rows: rect.row_start..=row_end.min(rect.row_end),
            columns: rect.col_start..=rect.col_end,
            cells: None,
            last: None,
        }
    }
}

/// The greatest populated row of each populated column on one sheet.
///
/// A whole-column operand clamps to the columns it actually names rather than to the sheet-wide
/// used range. The sheet-wide range would make the traversal height depend on cells
/// that no dependency rectangle covers, so an edit in an unreferenced column would change the
/// correct answer without dirtying the formula and full and incremental recalculation would
/// disagree. Clamping per column keeps the value a function of the recorded dependencies alone.
#[derive(Debug, Clone, Copy)]
pub(in crate::calculation) struct ColumnExtents<'workbook> {
    rows_by_column: &'workbook BTreeMap<u32, u32>,
}

impl<'workbook> ColumnExtents<'workbook> {
    pub(super) fn from_sheet(sheet: &'workbook Sheet) -> Self {
        Self {
            rows_by_column: sheet.column_max_rows(),
        }
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
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
            let reference = match expr {
                Expr::Paren(inner) => return self.resolve_reference_value_expr(context, inner),
                Expr::Ref(reference) => ReferenceValue::from_span(
                    self.resolve_reference_span(context.sheet(), reference)?,
                ),
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
                Expr::Range { .. } => {
                    ReferenceValue::from_rect(self.resolve_rect_expr(context, expr)?)
                }
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
                Expr::BuiltinCallable(callable) => {
                    let name = callable.canonical_name();
                    match context.binding(name) {
                        Some(ScopeValue::Reference(reference)) => reference.clone(),
                        Some(_) => return Err(ErrorKind::Value),
                        None => self
                            .resolve_name_expr_with_id_in_context(context, name)
                            .ok_or(ErrorKind::Value)
                            .and_then(|(id, named)| {
                                self.resolve_reference_value_expr(
                                    context
                                        .without_bindings()
                                        .with_defined_name_scope(Some(id.scope())),
                                    named,
                                )
                            })?,
                    }
                }
                Expr::Call { name, args } => {
                    if let Some(scoped) = callable_call_scope(self, context, name, args) {
                        match scoped {
                            ScopeValue::Reference(reference) => reference,
                            _ => return Err(ErrorKind::Value),
                        }
                    } else if function_evaluator(name)
                        == Some(Evaluator::Dynamic(DynamicFunction::Let))
                    {
                        let_reference(self, context, args)?
                    } else if let Some(branch) = self.selected_branch(context, name, args) {
                        self.resolve_reference_value_expr(context, branch?)?
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
        })
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
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
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
                    if function_evaluator(name).is_some()
                        && !function_call_shape_is_valid(name, args)
                    {
                        return Err(ErrorKind::Value);
                    }
                    if let Some(branch) = self.selected_branch(context, name, args) {
                        return self.resolve_rect_expr(context, branch?);
                    }
                    match function_evaluator(name) {
                        Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
                            let_reference(self, context, args)?.into_single_rect()
                        }
                        // Indexes holding arrays select a reference per element, which one rect
                        // cannot hold.
                        Some(Evaluator::Legacy(LegacyFunction::Index))
                            if index_arguments_hold_arrays(context, args) =>
                        {
                            Err(ErrorKind::Unsupported)
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
        })
    }

    /// The argument `IF` or `CHOOSE` returns, which is a reference when it is written as one:
    /// Excel takes `IF(TRUE,A1)` and `CHOOSE(1,A1)` as references (`ISREF(IF(TRUE,A1))` is `TRUE`,
    /// and `SUM(IF(TRUE,A1))` passes over text in `A1`, Mac Excel 16.114). `None` for other calls.
    fn selected_branch<'args>(
        &self,
        context: EvalContext<'_>,
        name: &str,
        args: &'args [Expr],
    ) -> Option<Result<&'args Expr, ErrorKind>> {
        let evaluator = function_evaluator(name)?;
        if !matches!(
            evaluator,
            Evaluator::Legacy(LegacyFunction::If) | Evaluator::Lookup(LookupFunction::Choose)
        ) {
            return None;
        }
        let Some(selector) = args.first() else {
            return Some(Err(ErrorKind::Value));
        };
        // A condition or index that holds several values selects element by element, an array of
        // values rather than one reference. A name or range of one cell selects one.
        let selector = if may_hold_array(self, context, selector) {
            match self.eval_array(context, selector) {
                Ok(array) if array.is_scalar() => array
                    .data
                    .into_iter()
                    .next()
                    .expect("a scalar array holds one value"),
                Ok(_) => return Some(Err(ErrorKind::Value)),
                Err(kind) => return Some(Err(kind)),
            }
        } else {
            self.eval_scalar(context, selector)
        };
        let selected = match evaluator {
            Evaluator::Legacy(LegacyFunction::If) => match to_logical(&selector) {
                Ok(true) => args.get(1),
                Ok(false) => args.get(2),
                Err(kind) => return Some(Err(kind)),
            },
            Evaluator::Lookup(LookupFunction::Choose) => {
                match to_number(&selector, self.numeric_text()) {
                    Ok(index) if index >= 1.0 => args.get(index.trunc() as usize),
                    Ok(_) => None,
                    Err(kind) => return Some(Err(kind)),
                }
            }
            _ => return None,
        };
        Some(match selected {
            Some(Expr::Missing) | None => Err(ErrorKind::Value),
            Some(branch) => Ok(branch),
        })
    }

    pub(in crate::calculation) fn resolve_index_rect(
        &self,
        context: EvalContext<'_>,
        args: &[Expr],
    ) -> Result<Rect, ErrorKind> {
        let prepared = prepare_evaluator_arguments(Evaluator::Legacy(LegacyFunction::Index), args)
            .ok_or(ErrorKind::Value)?;
        let args = prepared.as_ref();
        let context = context.with_arguments(args);
        let reference = self.resolve_reference_value_expr(context, &args[0])?;
        let area_index = match args.get(3) {
            Some(Expr::Missing) | None => 1.0,
            Some(expr) => to_number(&self.eval_scalar(context, expr), self.numeric_text())?.trunc(),
        };
        if !area_index.is_finite() || area_index < 1.0 {
            return Err(ErrorKind::Value);
        }
        let span = reference.area_span(area_index as usize)?;
        if span.is_sheet_range() {
            return Err(ErrorKind::Value);
        }
        let rect = span.into_rect().map_err(|_| ErrorKind::Value)?;
        let first_index =
            to_number(&self.eval_scalar(context, &args[1]), self.numeric_text())?.trunc();
        let second_index = match args.get(2) {
            Some(Expr::Missing) | None => None,
            Some(expr) => {
                Some(to_number(&self.eval_scalar(context, expr), self.numeric_text())?.trunc())
            }
        };
        let single_row = rect.height() == 1 && rect.width() > 1;
        // A reference of several rows and columns needs its column: `INDEX(A1:B3,2)` is `#REF!`
        // (Mac Excel 16.114).
        if second_index.is_none() && rect.height() > 1 && rect.width() > 1 {
            return Err(ErrorKind::Ref);
        }
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
        let context = context.with_arguments(args);
        // A reference array (`OFFSET(A1,{0;1},0)`) is not modeled; reading the first element
        // would answer for one reference.
        if evaluator_value_argument_holds_array(context, evaluator, args) {
            return Err(ErrorKind::Unsupported);
        }
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
        let rows =
            to_number(&self.eval_scalar(context, &args[1]), self.numeric_text())?.trunc() as i64;
        let columns =
            to_number(&self.eval_scalar(context, &args[2]), self.numeric_text())?.trunc() as i64;
        let height = match args.get(3) {
            Some(Expr::Missing) | None => base.height() as i64,
            Some(expr) => {
                to_number(&self.eval_scalar(context, expr), self.numeric_text())?.trunc() as i64
            }
        };
        let width = match args.get(4) {
            Some(Expr::Missing) | None => base.width() as i64,
            Some(expr) => {
                to_number(&self.eval_scalar(context, expr), self.numeric_text())?.trunc() as i64
            }
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

    /// Clamps a whole-column rect to the greatest populated row inside its own columns.
    ///
    /// Whole-column operations use this so that the result depends only on columns covered by the
    /// operation's dependency rectangles. See [`ColumnExtents`]. Array result areas populate their
    /// columns as stored cells do; dependency collection already links a whole-column operand to
    /// every array anchor whose area intersects it.
    pub(in crate::calculation) fn whole_column_row_end(&self, rect: &Rect) -> u32 {
        if !rect.whole_rows {
            return rect.row_end;
        }
        self.populated_row_end(rect)
    }

    /// Returns the last row of `rect` that holds a stored cell or an array result in its columns.
    ///
    /// Every cell below it reads as blank: no formula, array result, or result kept from a
    /// previous calculation lies there.
    pub(in crate::calculation) fn populated_row_end(&self, rect: &Rect) -> u32 {
        let stored = self.column_extents.get(rect.sheet).map_or(0, |extents| {
            extents.row_end_within(rect.col_start, rect.col_end)
        });
        let arrays = self
            .array_regions
            .iter()
            .map(|region| region.rect)
            .filter(|area| {
                area.sheet == rect.sheet
                    && area.col_start <= rect.col_end
                    && rect.col_start <= area.col_end
            })
            .map(|area| area.row_end)
            .max()
            .unwrap_or(0);
        rect.row_end.min(stored.max(arrays))
    }

    /// Returns the shared vertical traversal extent for one operation's participating ranges.
    ///
    /// Explicit ranges contribute their declared height. Whole-column ranges contribute only the
    /// populated prefix of their own columns. Taking the maximum preserves aligned virtual blanks
    /// in shorter participating columns without making the operation depend on unrelated columns.
    pub(in crate::calculation) fn operation_row_count<'rect>(
        &self,
        rects: impl IntoIterator<Item = &'rect Rect>,
    ) -> u64 {
        rects
            .into_iter()
            .map(|rect| {
                let row_end = self.whole_column_row_end(rect);
                if row_end < rect.row_start {
                    0
                } else {
                    u64::from(row_end - rect.row_start) + 1
                }
            })
            .max()
            .unwrap_or(0)
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
