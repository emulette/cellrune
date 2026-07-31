use std::collections::BTreeMap;

use super::{Engine, EvalContext};
use crate::Sheet;
use crate::calculation::ast::{
    Expr, RefBody, Reference, StructuredColumns, StructuredItem, StructuredReference,
};
use crate::calculation::coerce::{to_logical, to_number, to_text};
use crate::calculation::functions::{callable_call_scope, let_reference, normalize_name};
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::runtime::{Rect, RectSpan, ReferenceValue, SheetSpan};
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

    fn resolve_structured_reference(
        &self,
        context: EvalContext<'_>,
        reference: &StructuredReference,
    ) -> Result<ReferenceValue, ErrorKind> {
        let address = crate::CellAddress::from_indices(context.row(), context.column())
            .map_err(|_| ErrorKind::Ref)?;
        let location = match &reference.table {
            Some(name) => self.workbook.table_location(name).ok_or(ErrorKind::Name)?,
            None => {
                let sheet = self
                    .workbook
                    .sheets()
                    .get(context.sheet())
                    .ok_or(ErrorKind::Ref)?;
                self.workbook
                    .containing_table_location(sheet.id(), address)
                    .ok_or(ErrorKind::Value)?
            }
        };
        let table = &self.workbook.sheets()[location.sheet_index].tables()[location.table_index];
        let range = table.range();
        let table_row_start = range.start().row().get();
        let table_row_end = range.end().row().get();
        let table_col_start = range.start().column().get();
        let table_col_end = range.end().column().get();
        let header_end = table_row_start
            .checked_add(table.header_row_count())
            .and_then(|row| row.checked_sub(1))
            .ok_or(ErrorKind::Ref)?;
        let data_row_start = table_row_start
            .checked_add(table.header_row_count())
            .ok_or(ErrorKind::Ref)?;
        let data_row_end = table_row_end
            .checked_sub(table.totals_row_count())
            .ok_or(ErrorKind::Ref)?;
        let has_headers = table.header_row_count() > 0;
        let has_totals = table.totals_row_count() > 0 && table.totals_row_shown();
        let column_index = |name: &str| {
            self.workbook
                .table_column_location(table.id(), name)
                .map(|column| column.column_index)
                .ok_or(ErrorKind::Ref)
        };
        let (col_start, col_end) = match &reference.columns {
            None => (table_col_start, table_col_end),
            Some(StructuredColumns::Single(name)) => {
                let index = column_index(name)?;
                let column = table_col_start
                    .checked_add(u32::try_from(index).map_err(|_| ErrorKind::Ref)?)
                    .ok_or(ErrorKind::Ref)?;
                (column, column)
            }
            Some(StructuredColumns::Range { start, end }) => {
                let start = column_index(start)?;
                let end = column_index(end)?;
                let start = table_col_start
                    .checked_add(u32::try_from(start).map_err(|_| ErrorKind::Ref)?)
                    .ok_or(ErrorKind::Ref)?;
                let end = table_col_start
                    .checked_add(u32::try_from(end).map_err(|_| ErrorKind::Ref)?)
                    .ok_or(ErrorKind::Ref)?;
                (start.min(end), start.max(end))
            }
        };

        let default_item = if reference.table.is_some() {
            StructuredItem::Data
        } else {
            StructuredItem::ThisRow
        };
        let mut row_start = None::<u32>;
        let mut row_end = None::<u32>;
        let items = if reference.items.is_empty() {
            std::slice::from_ref(&default_item)
        } else {
            reference.items.as_slice()
        };
        for item in items {
            let band = match item {
                StructuredItem::All => Some((table_row_start, table_row_end)),
                StructuredItem::Headers if !has_headers => return Err(ErrorKind::Ref),
                StructuredItem::Headers => Some((table_row_start, header_end)),
                StructuredItem::Data if data_row_start > data_row_end => None,
                StructuredItem::Data => Some((data_row_start, data_row_end)),
                StructuredItem::Totals if !has_totals => None,
                StructuredItem::Totals => {
                    Some((table_row_end - table.totals_row_count() + 1, table_row_end))
                }
                StructuredItem::ThisRow
                    if context.sheet() != location.sheet_index
                        || context.row() < data_row_start
                        || context.row() > data_row_end =>
                {
                    return Err(ErrorKind::Value);
                }
                StructuredItem::ThisRow => Some((context.row(), context.row())),
            };
            if let Some((start, end)) = band {
                row_start = Some(row_start.map_or(start, |current| current.min(start)));
                row_end = Some(row_end.map_or(end, |current| current.max(end)));
            }
        }
        let (Some(row_start), Some(row_end)) = (row_start, row_end) else {
            return Ok(ReferenceValue::Empty);
        };
        Ok(ReferenceValue::from_rect(Rect {
            sheet: location.sheet_index,
            row_start,
            col_start,
            row_end,
            col_end,
            whole_rows: false,
        }))
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
            Expr::ReferenceUnion { left, right } => {
                let left = self.resolve_reference_value_expr(context, left)?;
                let right = self.resolve_reference_value_expr(context, right)?;
                if matches!(&left, ReferenceValue::Empty) || matches!(&right, ReferenceValue::Empty)
                {
                    return Err(ErrorKind::Ref);
                }
                let mut areas =
                    Vec::with_capacity(left.area_count().saturating_add(right.area_count()));
                areas.extend_from_slice(left.areas());
                areas.extend_from_slice(right.areas());
                ReferenceValue::from_areas(areas)
            }
            Expr::ReferenceIntersection { left, right } => {
                let left = self.resolve_reference_value_expr(context, left)?;
                let right = self.resolve_reference_value_expr(context, right)?;
                if matches!(&left, ReferenceValue::Empty) || matches!(&right, ReferenceValue::Empty)
                {
                    return Err(ErrorKind::Ref);
                }
                if left.has_sheet_span() || right.has_sheet_span() {
                    return Err(ErrorKind::Value);
                }
                let left_sheet = left
                    .areas()
                    .first()
                    .and_then(|area| area.rects().next())
                    .map(|rect| rect.sheet);
                let right_sheet = right
                    .areas()
                    .first()
                    .and_then(|area| area.rects().next())
                    .map(|rect| rect.sheet);
                if left_sheet != right_sheet {
                    return Err(ErrorKind::Value);
                }
                let comparisons = u64::try_from(left.area_count())
                    .ok()
                    .and_then(|left| {
                        u64::try_from(right.area_count())
                            .ok()
                            .and_then(|right| left.checked_mul(right))
                    })
                    .ok_or(ErrorKind::ResourceLimit(
                        CalculationLimitKind::FunctionIterations,
                    ))?;
                self.ensure_function_iterations(comparisons)?;
                if context.charges_reference_work() {
                    self.charge_function_iterations(context, comparisons)?;
                }
                let max_areas = self.options.limits().max_reference_areas();
                let mut areas = Vec::new();
                for left in left.areas() {
                    for right in right.areas() {
                        if context.is_cancelled() {
                            return Err(ErrorKind::ResourceLimit(
                                CalculationLimitKind::FunctionIterations,
                            ));
                        }
                        if let Some(area) = left.intersection(right) {
                            areas.push(area);
                            if u64::try_from(areas.len()).map_or(true, |count| count > max_areas) {
                                return Err(ErrorKind::ResourceLimit(
                                    CalculationLimitKind::ReferenceAreas,
                                ));
                            }
                        }
                    }
                }
                if areas.is_empty() {
                    return Err(ErrorKind::Null);
                }
                ReferenceValue::from_areas(areas)
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
                } else if normalize_name(name) == "LET" {
                    let_reference(self, context, args)?
                } else {
                    ReferenceValue::from_rect(self.resolve_rect_expr(context, expr)?)
                }
            }
            Expr::ErrorLit(kind) => return Err(*kind),
            Expr::SpillRef(_)
            | Expr::ExternalReference(_)
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
                let start = self
                    .resolve_reference_value_expr(context, start)?
                    .bounding_rect()?;
                let end = self
                    .resolve_reference_value_expr(context, end)?
                    .bounding_rect()?;
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
                match normalize_name(name).as_str() {
                    "LET" => let_reference(self, context, args)?.into_single_rect(),
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
        if args.len() < 2 || args.len() > 4 {
            return Err(ErrorKind::Value);
        }
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
