use std::collections::{BTreeMap, BTreeSet};

use super::{Engine, EvalContext};
use crate::CellContent;
use crate::calculation::ast::Expr;
use crate::calculation::functions::normalize_name;
use crate::calculation::graph::DependencyGraph;
use crate::calculation::lambda::{is_lambda_local, walk_lambda_scope};
use crate::calculation::runtime::Rect;

impl Engine<'_> {
    pub(super) fn dependencies_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(DependencyGraph, bool), ()> {
        self.collect_dependencies(true, cancelled)
    }

    pub(super) fn exceeds_dependency_limit(&self) -> bool {
        self.collect_dependencies(false, &|| false)
            .expect("non-cancellable dependency collection cannot be cancelled")
            .1
    }

    pub(super) fn has_unresolved_dynamic_dependencies(&self) -> bool {
        self.workbook.sheets().iter().any(|sheet| {
            sheet.cells().any(|cell| {
                matches!(
                    cell.content(),
                    CellContent::Formula(formula)
                        if matches!(
                            formula.metadata(),
                            crate::FormulaMetadata::DynamicArray { range: None, .. }
                        )
                )
            })
        }) || self.asts.iter().any(|(cell, expr)| {
            self.expr_has_unresolved_dynamic_dependency(
                EvalContext::for_cell(*cell),
                expr,
                &mut BTreeSet::new(),
            )
        })
    }

    pub(super) fn has_unstable_incremental_dependencies(&self) -> bool {
        self.workbook.sheets().iter().any(|sheet| {
            sheet.cells().any(|cell| {
                matches!(
                    cell.content(),
                    CellContent::Formula(formula)
                        if matches!(
                            formula.metadata(),
                            crate::FormulaMetadata::DynamicArray { range: None, .. }
                        )
                )
            })
        }) || self.asts.iter().any(|(cell, expr)| {
            self.expr_contains_dynamic_reference_function(
                EvalContext::for_cell(*cell),
                expr,
                &mut BTreeSet::new(),
                &mut Vec::new(),
            )
        })
    }

    pub(super) fn dependency_rectangles(&self) -> BTreeMap<super::CellId, Vec<Rect>> {
        let mut result = BTreeMap::new();
        for (cell, expr) in &self.asts {
            let mut rects = Vec::new();
            self.collect_dependency_rects(
                EvalContext::for_cell(*cell),
                expr,
                &mut BTreeSet::new(),
                &mut Vec::new(),
                &mut rects,
            );
            rects.sort_by_key(|rect| {
                (
                    rect.sheet,
                    rect.row_start,
                    rect.col_start,
                    rect.row_end,
                    rect.col_end,
                )
            });
            rects.dedup();
            result.insert(*cell, rects);
        }
        result
    }

    fn expr_has_unresolved_dynamic_dependency(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        names: &mut BTreeSet<String>,
    ) -> bool {
        match expr {
            Expr::Call { name, args } => {
                let normalized = normalize_name(name);
                let dynamic = matches!(normalized.as_str(), "OFFSET" | "INDIRECT");
                (dynamic && self.resolve_dynamic_rect(context, name, args).is_err())
                    || args
                        .iter()
                        .any(|arg| self.expr_has_unresolved_dynamic_dependency(context, arg, names))
            }
            Expr::Name(name) => {
                let key = name.to_ascii_lowercase();
                names.insert(key)
                    && self
                        .resolve_name_expr(context.sheet(), name)
                        .is_some_and(|named| {
                            self.expr_has_unresolved_dynamic_dependency(context, named, names)
                        })
            }
            Expr::ImplicitIntersection(inner)
            | Expr::Paren(inner)
            | Expr::Unary { operand: inner, .. } => {
                self.expr_has_unresolved_dynamic_dependency(context, inner, names)
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                self.expr_has_unresolved_dynamic_dependency(context, left, names)
                    || self.expr_has_unresolved_dynamic_dependency(context, right, names)
            }
            Expr::Array(rows) => rows.iter().flatten().any(|element| {
                self.expr_has_unresolved_dynamic_dependency(context, element, names)
            }),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::Missing => false,
        }
    }

    fn collect_dependencies(
        &self,
        retain_graph: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(DependencyGraph, bool), ()> {
        let mut dependencies = BTreeMap::new();
        let formula_cells: Vec<BTreeSet<(u32, u32)>> = self
            .workbook
            .sheets()
            .iter()
            .map(|sheet| {
                sheet
                    .cells()
                    .filter(|cell| matches!(cell.content(), CellContent::Formula(_)))
                    .map(|cell| (cell.address().row().get(), cell.address().column().get()))
                    .collect()
            })
            .collect();
        let mut edge_count = 0_u64;
        for (cell, expr) in &self.asts {
            if cancelled() {
                return Err(());
            }
            if self.name_cycle_cells.contains(cell) || self.name_limit_cells.contains(cell) {
                if retain_graph {
                    dependencies.insert(*cell, Vec::new());
                }
                continue;
            }
            let mut rects = Vec::new();
            self.collect_dependency_rects(
                EvalContext::for_cell(*cell),
                expr,
                &mut BTreeSet::new(),
                &mut Vec::new(),
                &mut rects,
            );
            let mut cell_dependencies = Vec::new();
            for rect in rects {
                if cancelled() {
                    return Err(());
                }
                if rect.is_single_cell() {
                    if formula_cells[rect.sheet].contains(&(rect.row_start, rect.col_start)) {
                        cell_dependencies.push((rect.sheet, rect.row_start, rect.col_start));
                    }
                    if let Some(owner) =
                        self.array_owner((rect.sheet, rect.row_start, rect.col_start))
                    {
                        cell_dependencies.push(owner);
                    }
                    continue;
                }
                for (row, column) in
                    formula_cells[rect.sheet].range((rect.row_start, 0)..=(rect.row_end, u32::MAX))
                {
                    if *column >= rect.col_start && *column <= rect.col_end {
                        cell_dependencies.push((rect.sheet, *row, *column));
                    }
                }
                for region in &self.array_regions {
                    if rects_intersect(&rect, &region.rect) {
                        cell_dependencies.push(region.anchor);
                    }
                }
            }
            cell_dependencies.sort_unstable();
            cell_dependencies.dedup();
            edge_count = match edge_count.checked_add(cell_dependencies.len() as u64) {
                Some(total) => total,
                None => {
                    if retain_graph {
                        dependencies.insert(*cell, cell_dependencies);
                    }
                    return Ok((dependencies, true));
                }
            };
            if edge_count > self.options.limits().max_dependency_edges() {
                if retain_graph {
                    dependencies.insert(*cell, cell_dependencies);
                }
                return Ok((dependencies, true));
            }
            if retain_graph {
                dependencies.insert(*cell, cell_dependencies);
            }
        }
        Ok((dependencies, false))
    }

    fn collect_dependency_rects(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        names: &mut BTreeSet<String>,
        local_names: &mut Vec<String>,
        output: &mut Vec<Rect>,
    ) {
        match expr {
            Expr::Ref(reference) => {
                if let Ok(rect) = self.resolve_reference(context.sheet(), reference) {
                    output.push(rect);
                }
            }
            Expr::Range { start, end } => {
                if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                    output.push(rect);
                }
                self.collect_dependency_rects(context, start, names, local_names, output);
                self.collect_dependency_rects(context, end, names, local_names, output);
            }
            Expr::Name(name) => {
                if is_lambda_local(name, local_names) {
                    return;
                }
                let key = name.to_ascii_lowercase();
                if names.insert(key)
                    && let Some(named) = self.resolve_name_expr(context.sheet(), name)
                {
                    self.collect_dependency_rects(context, named, names, local_names, output);
                }
            }
            Expr::ImplicitIntersection(inner) => {
                if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                    output.push(rect);
                    let mut selection_names = BTreeSet::new();
                    self.collect_reference_selection_inputs(
                        context,
                        inner,
                        &mut selection_names,
                        local_names,
                        output,
                    );
                } else {
                    self.collect_dependency_rects(context, inner, names, local_names, output);
                }
            }
            Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
                self.collect_dependency_rects(context, inner, names, local_names, output);
            }
            Expr::Binary { left, right, .. } => {
                self.collect_dependency_rects(context, left, names, local_names, output);
                self.collect_dependency_rects(context, right, names, local_names, output);
            }
            Expr::Call { name, args } => {
                let normalized = normalize_name(name);
                if matches!(normalized.as_str(), "SUMIF" | "AVERAGEIF")
                    && args.len() == 3
                    && let (Ok(criteria_range), Ok(value_anchor)) = (
                        self.resolve_rect_expr(context, &args[0]),
                        self.resolve_rect_expr(context, &args[2]),
                    )
                    && let Some(value_range) = value_anchor
                        .resized_from_anchor(criteria_range.height(), criteria_range.width())
                {
                    output.push(value_range);
                }
                if matches!(normalized.as_str(), "OFFSET" | "INDIRECT")
                    && let Ok(rect) = self.resolve_dynamic_rect(context, name, args)
                {
                    output.push(rect);
                }
                if walk_lambda_scope(name, args, local_names, |arg, scope| {
                    self.collect_dependency_rects(context, arg, names, scope, output);
                }) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_rects(context, arg, names, local_names, output);
                }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for element in row {
                        self.collect_dependency_rects(context, element, names, local_names, output);
                    }
                }
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Missing => {}
        }
    }

    fn collect_reference_selection_inputs(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        names: &mut BTreeSet<String>,
        local_names: &mut Vec<String>,
        output: &mut Vec<Rect>,
    ) {
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.collect_reference_selection_inputs(context, inner, names, local_names, output);
            }
            Expr::Range { start, end } => {
                self.collect_reference_selection_inputs(context, start, names, local_names, output);
                self.collect_reference_selection_inputs(context, end, names, local_names, output);
            }
            Expr::Name(name) => {
                if is_lambda_local(name, local_names) {
                    return;
                }
                let key = name.to_ascii_lowercase();
                if names.insert(key)
                    && let Some(named) = self.resolve_name_expr(context.sheet(), name)
                {
                    self.collect_reference_selection_inputs(
                        context,
                        named,
                        names,
                        local_names,
                        output,
                    );
                }
            }
            Expr::Call { name, args } => {
                if walk_lambda_scope(name, args, local_names, |arg, scope| {
                    self.collect_dependency_rects(context, arg, names, scope, output);
                }) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_rects(context, arg, names, local_names, output);
                }
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Array(_)
            | Expr::Missing => {}
        }
    }

    fn expr_contains_dynamic_reference_function(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        names: &mut BTreeSet<String>,
        local_names: &mut Vec<String>,
    ) -> bool {
        match expr {
            Expr::Call { name, args } => {
                if matches!(normalize_name(name).as_str(), "OFFSET" | "INDIRECT") {
                    return true;
                }
                let mut found = false;
                if walk_lambda_scope(name, args, local_names, |arg, scope| {
                    found |=
                        self.expr_contains_dynamic_reference_function(context, arg, names, scope);
                }) {
                    return found;
                }
                args.iter().any(|arg| {
                    self.expr_contains_dynamic_reference_function(context, arg, names, local_names)
                })
            }
            Expr::Name(name) => {
                if is_lambda_local(name, local_names) {
                    return false;
                }
                let key = name.to_ascii_lowercase();
                names.insert(key)
                    && self
                        .resolve_name_expr(context.sheet(), name)
                        .is_some_and(|named| {
                            self.expr_contains_dynamic_reference_function(
                                context,
                                named,
                                names,
                                local_names,
                            )
                        })
            }
            Expr::ImplicitIntersection(inner)
            | Expr::Paren(inner)
            | Expr::Unary { operand: inner, .. } => {
                self.expr_contains_dynamic_reference_function(context, inner, names, local_names)
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                self.expr_contains_dynamic_reference_function(context, left, names, local_names)
                    || self.expr_contains_dynamic_reference_function(
                        context,
                        right,
                        names,
                        local_names,
                    )
            }
            Expr::Array(rows) => rows.iter().flatten().any(|element| {
                self.expr_contains_dynamic_reference_function(context, element, names, local_names)
            }),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::Missing => false,
        }
    }
}

fn rects_intersect(left: &Rect, right: &Rect) -> bool {
    left.sheet == right.sheet
        && left.row_start <= right.row_end
        && right.row_start <= left.row_end
        && left.col_start <= right.col_end
        && right.col_start <= left.col_end
}

#[cfg(test)]
#[path = "dependency_tests.rs"]
mod tests;
