use std::collections::{BTreeMap, BTreeSet};

use super::{Engine, EvalContext, EvaluationBudget};
use crate::CellContent;
use crate::calculation::ast::Expr;
use crate::calculation::functions::{normalize_name, with_let_scope};
use crate::calculation::graph::DependencyGraph;
use crate::calculation::lambda::{is_local_name, walk_local_scope};
use crate::calculation::runtime::{Rect, RectSpan};
use crate::calculation::scope::{DefinedLambdaId, ScopeValue};

#[derive(Default)]
struct VisitedDefinitions {
    values: BTreeSet<DefinedLambdaId>,
    lambdas: BTreeSet<DefinedLambdaId>,
}

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

    pub(super) fn has_unresolved_dynamic_dependencies(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        for sheet in self.workbook.sheets() {
            for cell in sheet.cells() {
                if cancelled() {
                    return Err(());
                }
                if matches!(
                    cell.content(),
                    CellContent::Formula(formula)
                        if matches!(
                            formula.metadata(),
                            crate::FormulaMetadata::DynamicArray { range: None, .. }
                        )
                ) {
                    return Ok(true);
                }
            }
        }
        for (cell, parsed) in &self.asts {
            if cancelled() {
                return Err(());
            }
            let budget = EvaluationBudget::default();
            let found = self.expr_has_unresolved_dynamic_dependency(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
            );
            if cancelled() {
                return Err(());
            }
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn has_unstable_incremental_dependencies(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        for sheet in self.workbook.sheets() {
            for cell in sheet.cells() {
                if cancelled() {
                    return Err(());
                }
                if matches!(
                    cell.content(),
                    CellContent::Formula(formula)
                        if matches!(
                            formula.metadata(),
                            crate::FormulaMetadata::DynamicArray { range: None, .. }
                        )
                ) {
                    return Ok(true);
                }
            }
        }
        for (cell, parsed) in &self.asts {
            if cancelled() {
                return Err(());
            }
            let budget = EvaluationBudget::default();
            let found = self.expr_contains_dynamic_reference_function(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
            );
            if cancelled() {
                return Err(());
            }
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    pub(super) fn dependency_rectangles(&self) -> BTreeMap<super::CellId, Vec<RectSpan>> {
        self.dependency_rectangles_cancellable(&|| false)
            .expect("non-cancellable dependency collection cannot be cancelled")
    }

    pub(super) fn dependency_rectangles_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BTreeMap<super::CellId, Vec<RectSpan>>, ()> {
        let mut result = BTreeMap::new();
        for (cell, parsed) in &self.asts {
            if cancelled() {
                return Err(());
            }
            let mut rects = Vec::new();
            let budget = EvaluationBudget::default();
            self.collect_dependency_rects(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut rects,
            );
            if cancelled() {
                return Err(());
            }
            rects.sort_by_key(RectSpan::sort_key);
            rects.dedup();
            result.insert(*cell, rects);
        }
        Ok(result)
    }

    fn expr_has_unresolved_dynamic_dependency(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
    ) -> bool {
        if context.is_cancelled() {
            return true;
        }
        match expr {
            Expr::Call { name, args } => {
                if let Some(binding) = context.binding(name) {
                    if !matches!(binding, ScopeValue::Callable(_)) {
                        return false;
                    }
                    return args.iter().any(|arg| {
                        self.expr_has_unresolved_dynamic_dependency(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    });
                }
                if is_local_name(name, local_names) {
                    return args.iter().any(|arg| {
                        self.expr_has_unresolved_dynamic_dependency(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    });
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    let Some(lambda) = crate::calculation::lambda::definition(named) else {
                        return false;
                    };
                    let mut found = false;
                    if visited.lambdas.insert(id.clone()) {
                        let mut lambda_names = lambda.parameters().to_vec();
                        found |= self.expr_has_unresolved_dynamic_dependency(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            lambda.body(),
                            visited,
                            &mut lambda_names,
                        );
                    }
                    return found
                        || args.iter().any(|arg| {
                            self.expr_has_unresolved_dynamic_dependency(
                                context,
                                arg,
                                visited,
                                local_names,
                            )
                        });
                }
                let normalized = normalize_name(name);
                if normalized == "LET" {
                    let mut found = false;
                    let result =
                        with_let_scope(self, context, args, |engine, scoped, arg, final_arg| {
                            found |= engine.expr_has_unresolved_dynamic_dependency(
                                scoped,
                                arg,
                                visited,
                                local_names,
                            );
                            final_arg.then_some(())
                        });
                    return found || result.is_err();
                }
                let dynamic = matches!(normalized.as_str(), "OFFSET" | "INDIRECT");
                if dynamic && self.resolve_dynamic_rect(context, name, args).is_err() {
                    return true;
                }
                let mut found = false;
                if walk_local_scope(name, args, local_names, |arg, scope| {
                    found |=
                        self.expr_has_unresolved_dynamic_dependency(context, arg, visited, scope);
                }) {
                    return found;
                }
                args.iter().any(|arg| {
                    self.expr_has_unresolved_dynamic_dependency(context, arg, visited, local_names)
                })
            }
            Expr::Invoke { callee, args } => {
                self.expr_has_unresolved_dynamic_dependency(context, callee, visited, local_names)
                    || args.iter().any(|arg| {
                        self.expr_has_unresolved_dynamic_dependency(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    })
            }
            Expr::Name(name) => {
                if context.binding(name).is_some() || is_local_name(name, local_names) {
                    return false;
                }
                self.resolve_name_expr_with_id_in_context(context, name)
                    .is_some_and(|(id, named)| {
                        if !visited.values.insert(id.clone()) {
                            return false;
                        }
                        let mut defined_locals = Vec::new();
                        self.expr_has_unresolved_dynamic_dependency(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            named,
                            visited,
                            &mut defined_locals,
                        )
                    })
            }
            Expr::ImplicitIntersection(inner)
            | Expr::SpillRef(inner)
            | Expr::Paren(inner)
            | Expr::Unary { operand: inner, .. } => {
                self.expr_has_unresolved_dynamic_dependency(context, inner, visited, local_names)
            }
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                self.expr_has_unresolved_dynamic_dependency(context, left, visited, local_names)
                    || self.expr_has_unresolved_dynamic_dependency(
                        context,
                        right,
                        visited,
                        local_names,
                    )
            }
            Expr::Array(rows) => rows.iter().flatten().any(|element| {
                self.expr_has_unresolved_dynamic_dependency(context, element, visited, local_names)
            }),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Missing => false,
        }
    }

    fn collect_dependencies(
        &self,
        retain_graph: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(DependencyGraph, bool), ()> {
        let mut dependencies = BTreeMap::new();
        let mut formula_cells = Vec::with_capacity(self.workbook.sheets().len());
        for sheet in self.workbook.sheets() {
            if cancelled() {
                return Err(());
            }
            let mut cells = BTreeSet::new();
            for cell in sheet.cells() {
                if cancelled() {
                    return Err(());
                }
                if matches!(cell.content(), CellContent::Formula(_)) {
                    cells.insert((cell.address().row().get(), cell.address().column().get()));
                }
            }
            formula_cells.push(cells);
        }
        let mut edge_count = 0_u64;
        for (cell, parsed) in &self.asts {
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
            let budget = EvaluationBudget::default();
            self.collect_dependency_rects(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut rects,
            );
            if cancelled() {
                return Err(());
            }
            let mut cell_dependencies = Vec::new();
            for span in rects {
                for rect in span.rects() {
                    if cancelled() {
                        return Err(());
                    }
                    if rect.is_single_cell() {
                        if formula_cells[rect.sheet].contains(&(rect.row_start, rect.col_start)) {
                            cell_dependencies.push((rect.sheet, rect.row_start, rect.col_start));
                        }
                        if let Some(owner) = self.cancellable_array_owner(
                            (rect.sheet, rect.row_start, rect.col_start),
                            cancelled,
                        )? {
                            cell_dependencies.push(owner);
                        }
                        continue;
                    }
                    for (row, column) in formula_cells[rect.sheet]
                        .range((rect.row_start, 0)..=(rect.row_end, u32::MAX))
                    {
                        if cancelled() {
                            return Err(());
                        }
                        if *column >= rect.col_start && *column <= rect.col_end {
                            cell_dependencies.push((rect.sheet, *row, *column));
                        }
                    }
                    for region in &self.array_regions {
                        if cancelled() {
                            return Err(());
                        }
                        if rects_intersect(&rect, &region.rect) {
                            cell_dependencies.push(region.anchor);
                        }
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

    fn cancellable_array_owner(
        &self,
        cell: super::CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<super::CellId>, ()> {
        for region in &self.array_regions {
            if cancelled() {
                return Err(());
            }
            if region.rect.sheet == cell.0
                && (region.rect.row_start..=region.rect.row_end).contains(&cell.1)
                && (region.rect.col_start..=region.rect.col_end).contains(&cell.2)
            {
                return Ok(Some(region.anchor));
            }
        }
        Ok(None)
    }

    fn collect_dependency_rects(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
        output: &mut Vec<RectSpan>,
    ) {
        if context.is_cancelled() {
            return;
        }
        match expr {
            Expr::Ref(reference) => {
                if let Ok(span) = self.resolve_reference_span(context.sheet(), reference) {
                    output.push(span);
                }
            }
            Expr::Range { start, end } => {
                if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                    output.push(RectSpan::single(rect));
                }
                self.collect_dependency_rects(context, start, visited, local_names, output);
                self.collect_dependency_rects(context, end, visited, local_names, output);
            }
            Expr::Name(name) => {
                if context.binding(name).is_some() || is_local_name(name, local_names) {
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                    && visited.values.insert(id.clone())
                {
                    self.collect_dependency_rects(
                        context
                            .without_bindings()
                            .with_defined_name_scope(Some(id.scope())),
                        named,
                        visited,
                        &mut Vec::new(),
                        output,
                    );
                }
            }
            Expr::ImplicitIntersection(inner) => {
                if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                    output.push(RectSpan::single(rect));
                    let mut selection_names = VisitedDefinitions::default();
                    self.collect_reference_selection_inputs(
                        context,
                        inner,
                        &mut selection_names,
                        local_names,
                        output,
                    );
                } else {
                    self.collect_dependency_rects(context, inner, visited, local_names, output);
                }
            }
            Expr::Paren(inner) | Expr::SpillRef(inner) | Expr::Unary { operand: inner, .. } => {
                self.collect_dependency_rects(context, inner, visited, local_names, output);
            }
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right } => {
                self.collect_dependency_rects(context, left, visited, local_names, output);
                self.collect_dependency_rects(context, right, visited, local_names, output);
            }
            Expr::Call { name, args } => {
                if let Some(binding) = context.binding(name) {
                    if !matches!(binding, ScopeValue::Callable(_)) {
                        return;
                    }
                    for arg in args {
                        self.collect_dependency_rects(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if is_local_name(name, local_names) {
                    for arg in args {
                        self.collect_dependency_rects(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    if let Some(lambda) = crate::calculation::lambda::definition(named) {
                        if visited.lambdas.insert(id.clone()) {
                            let mut lambda_names = lambda.parameters().to_vec();
                            self.collect_dependency_rects(
                                context
                                    .without_bindings()
                                    .with_defined_name_scope(Some(id.scope())),
                                lambda.body(),
                                visited,
                                &mut lambda_names,
                                output,
                            );
                        }
                        for arg in args {
                            self.collect_dependency_rects(
                                context,
                                arg,
                                visited,
                                local_names,
                                output,
                            );
                        }
                    }
                    return;
                }
                let normalized = normalize_name(name);
                if normalized == "LET" {
                    let _ =
                        with_let_scope(self, context, args, |engine, scoped, arg, final_arg| {
                            engine.collect_dependency_rects(
                                scoped,
                                arg,
                                visited,
                                local_names,
                                output,
                            );
                            final_arg.then_some(())
                        });
                    return;
                }
                if matches!(normalized.as_str(), "SUMIF" | "AVERAGEIF")
                    && args.len() == 3
                    && let (Ok(criteria_range), Ok(value_anchor)) = (
                        self.resolve_rect_expr(context, &args[0]),
                        self.resolve_rect_expr(context, &args[2]),
                    )
                    && let Some(value_range) = value_anchor
                        .resized_from_anchor(criteria_range.height(), criteria_range.width())
                {
                    output.push(RectSpan::single(value_range));
                }
                if matches!(normalized.as_str(), "OFFSET" | "INDIRECT")
                    && let Ok(rect) = self.resolve_dynamic_rect(context, name, args)
                {
                    output.push(RectSpan::single(rect));
                }
                if walk_local_scope(name, args, local_names, |arg, scope| {
                    self.collect_dependency_rects(context, arg, visited, scope, output);
                }) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_rects(context, arg, visited, local_names, output);
                }
            }
            Expr::Invoke { callee, args } => {
                self.collect_dependency_rects(context, callee, visited, local_names, output);
                for arg in args {
                    self.collect_dependency_rects(context, arg, visited, local_names, output);
                }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for element in row {
                        self.collect_dependency_rects(
                            context,
                            element,
                            visited,
                            local_names,
                            output,
                        );
                    }
                }
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Missing => {}
        }
    }

    fn collect_reference_selection_inputs(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
        output: &mut Vec<RectSpan>,
    ) {
        if context.is_cancelled() {
            return;
        }
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) | Expr::SpillRef(inner) => {
                self.collect_reference_selection_inputs(
                    context,
                    inner,
                    visited,
                    local_names,
                    output,
                );
            }
            Expr::Range { start, end }
            | Expr::ReferenceUnion {
                left: start,
                right: end,
            }
            | Expr::ReferenceIntersection {
                left: start,
                right: end,
            } => {
                self.collect_reference_selection_inputs(
                    context,
                    start,
                    visited,
                    local_names,
                    output,
                );
                self.collect_reference_selection_inputs(context, end, visited, local_names, output);
            }
            Expr::Name(name) => {
                if context.binding(name).is_some() || is_local_name(name, local_names) {
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                    && visited.values.insert(id.clone())
                {
                    self.collect_reference_selection_inputs(
                        context
                            .without_bindings()
                            .with_defined_name_scope(Some(id.scope())),
                        named,
                        visited,
                        &mut Vec::new(),
                        output,
                    );
                }
            }
            Expr::Call { name, args } => {
                if let Some(binding) = context.binding(name) {
                    if !matches!(binding, ScopeValue::Callable(_)) {
                        return;
                    }
                    for arg in args {
                        self.collect_dependency_rects(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if is_local_name(name, local_names) {
                    for arg in args {
                        self.collect_dependency_rects(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    if let Some(lambda) = crate::calculation::lambda::definition(named) {
                        if visited.lambdas.insert(id.clone()) {
                            let mut lambda_names = lambda.parameters().to_vec();
                            self.collect_dependency_rects(
                                context
                                    .without_bindings()
                                    .with_defined_name_scope(Some(id.scope())),
                                lambda.body(),
                                visited,
                                &mut lambda_names,
                                output,
                            );
                        }
                        for arg in args {
                            self.collect_dependency_rects(
                                context,
                                arg,
                                visited,
                                local_names,
                                output,
                            );
                        }
                    }
                    return;
                }
                if normalize_name(name) == "LET" {
                    let _ =
                        with_let_scope(self, context, args, |engine, scoped, arg, final_arg| {
                            if final_arg {
                                engine.collect_reference_selection_inputs(
                                    scoped,
                                    arg,
                                    visited,
                                    local_names,
                                    output,
                                );
                            } else {
                                engine.collect_dependency_rects(
                                    scoped,
                                    arg,
                                    visited,
                                    local_names,
                                    output,
                                );
                            }
                            final_arg.then_some(())
                        });
                    return;
                }
                if walk_local_scope(name, args, local_names, |arg, scope| {
                    self.collect_dependency_rects(context, arg, visited, scope, output);
                }) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_rects(context, arg, visited, local_names, output);
                }
            }
            Expr::Invoke { callee, args } => {
                self.collect_reference_selection_inputs(
                    context,
                    callee,
                    visited,
                    local_names,
                    output,
                );
                for arg in args {
                    self.collect_reference_selection_inputs(
                        context,
                        arg,
                        visited,
                        local_names,
                        output,
                    );
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
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Missing => {}
        }
    }

    fn expr_contains_dynamic_reference_function(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
    ) -> bool {
        if context.is_cancelled() {
            return true;
        }
        match expr {
            Expr::Call { name, args } => {
                if let Some(binding) = context.binding(name) {
                    if !matches!(binding, ScopeValue::Callable(_)) {
                        return false;
                    }
                    return args.iter().any(|arg| {
                        self.expr_contains_dynamic_reference_function(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    });
                }
                if is_local_name(name, local_names) {
                    return args.iter().any(|arg| {
                        self.expr_contains_dynamic_reference_function(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    });
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    let Some(lambda) = crate::calculation::lambda::definition(named) else {
                        return false;
                    };
                    let mut found = false;
                    if visited.lambdas.insert(id.clone()) {
                        let mut lambda_names = lambda.parameters().to_vec();
                        found |= self.expr_contains_dynamic_reference_function(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            lambda.body(),
                            visited,
                            &mut lambda_names,
                        );
                    }
                    return found
                        || args.iter().any(|arg| {
                            self.expr_contains_dynamic_reference_function(
                                context,
                                arg,
                                visited,
                                local_names,
                            )
                        });
                }
                if matches!(normalize_name(name).as_str(), "OFFSET" | "INDIRECT") {
                    return true;
                }
                let mut found = false;
                if walk_local_scope(name, args, local_names, |arg, scope| {
                    found |=
                        self.expr_contains_dynamic_reference_function(context, arg, visited, scope);
                }) {
                    return found;
                }
                args.iter().any(|arg| {
                    self.expr_contains_dynamic_reference_function(
                        context,
                        arg,
                        visited,
                        local_names,
                    )
                })
            }
            Expr::Invoke { callee, args } => {
                self.expr_contains_dynamic_reference_function(context, callee, visited, local_names)
                    || args.iter().any(|arg| {
                        self.expr_contains_dynamic_reference_function(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    })
            }
            Expr::Name(name) => {
                if context.binding(name).is_some() || is_local_name(name, local_names) {
                    return false;
                }
                self.resolve_name_expr_with_id_in_context(context, name)
                    .is_some_and(|(id, named)| {
                        if !visited.values.insert(id.clone()) {
                            return false;
                        }
                        let mut defined_locals = Vec::new();
                        self.expr_contains_dynamic_reference_function(
                            context
                                .without_bindings()
                                .with_defined_name_scope(Some(id.scope())),
                            named,
                            visited,
                            &mut defined_locals,
                        )
                    })
            }
            Expr::ImplicitIntersection(inner)
            | Expr::SpillRef(inner)
            | Expr::Paren(inner)
            | Expr::Unary { operand: inner, .. } => {
                self.expr_contains_dynamic_reference_function(context, inner, visited, local_names)
            }
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                self.expr_contains_dynamic_reference_function(context, left, visited, local_names)
                    || self.expr_contains_dynamic_reference_function(
                        context,
                        right,
                        visited,
                        local_names,
                    )
            }
            Expr::Array(rows) => rows.iter().flatten().any(|element| {
                self.expr_contains_dynamic_reference_function(
                    context,
                    element,
                    visited,
                    local_names,
                )
            }),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
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
