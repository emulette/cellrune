use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{Engine, EvalContext, EvaluationBudget};
use crate::CellContent;
use crate::calculation::ast::{Expr, StructuredReference};
use crate::calculation::functions::descriptor::DependencyKind;
use crate::calculation::functions::{
    DynamicFunction, Evaluator, function_arguments_are_reachable, function_dependency_kind,
    function_evaluator, normalize_name, with_let_scope,
};
use crate::calculation::graph::DependencyGraph;
use crate::calculation::lambda::{is_local_name, walk_local_scope};
use crate::calculation::runtime::{Rect, RectSpan};
use crate::calculation::scope::{DefinedLambdaId, ScopeValue};
use crate::{SheetId, Table, TableId, WorkbookSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TableTopologyRevision([u8; 32]);

fn is_let_function(name: &str) -> bool {
    function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Let))
}

impl TableTopologyRevision {
    fn from_table(
        sheet_index: usize,
        sheet_id: SheetId,
        table: &Table,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let range = table.range();
        let mut digest = Sha256::new();
        digest.update(b"cellrune.table-topology.v1");
        digest.update((sheet_index as u64).to_le_bytes());
        digest.update(sheet_id.get().to_le_bytes());
        digest.update(table.id().get().to_le_bytes());
        digest.update(range.start().row().get().to_le_bytes());
        digest.update(range.start().column().get().to_le_bytes());
        digest.update(range.end().row().get().to_le_bytes());
        digest.update(range.end().column().get().to_le_bytes());
        digest.update(table.header_row_count().to_le_bytes());
        digest.update(table.totals_row_count().to_le_bytes());
        digest.update([u8::from(table.totals_row_shown())]);
        update_topology_text(&mut digest, table.name().as_str());
        update_topology_text(&mut digest, table.display_name().as_str());
        digest.update((table.columns().len() as u64).to_le_bytes());
        for column in table.columns() {
            if cancelled() {
                return Err(());
            }
            digest.update(column.column_id().get().to_le_bytes());
            update_topology_text(&mut digest, column.name());
        }
        Ok(Self(digest.finalize().into()))
    }
}

fn update_topology_text(digest: &mut Sha256, text: &str) {
    digest.update((text.len() as u64).to_le_bytes());
    digest.update(text.as_bytes());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) struct TableDependency {
    table_id: TableId,
    topology: TableTopologyRevision,
}

impl TableDependency {
    pub(super) const fn table_id(self) -> TableId {
        self.table_id
    }

    pub(super) const fn topology(self) -> TableTopologyRevision {
        self.topology
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::calculation) enum DependencyTarget {
    Cell(super::CellId),
    Area(RectSpan),
    TableIdentity(TableDependency),
    SpillAnchor(super::CellId),
}

impl DependencyTarget {
    fn from_span(span: RectSpan) -> Self {
        if let Ok(rect) = span.clone().into_rect()
            && rect.is_single_cell()
        {
            return Self::Cell((rect.sheet, rect.row_start, rect.col_start));
        }
        Self::Area(span)
    }

    #[cfg(test)]
    pub(super) fn span(&self) -> Option<RectSpan> {
        match self {
            Self::Cell((sheet, row, column)) | Self::SpillAnchor((sheet, row, column)) => {
                Some(RectSpan::single(Rect {
                    sheet: *sheet,
                    row_start: *row,
                    col_start: *column,
                    row_end: *row,
                    col_end: *column,
                    whole_rows: false,
                }))
            }
            Self::Area(span) => Some(span.clone()),
            Self::TableIdentity(_) => None,
        }
    }
}

fn compare_targets(left: &DependencyTarget, right: &DependencyTarget) -> Ordering {
    let rank = |target: &DependencyTarget| match target {
        DependencyTarget::Cell(_) => 0_u8,
        DependencyTarget::Area(_) => 1,
        DependencyTarget::TableIdentity(_) => 2,
        DependencyTarget::SpillAnchor(_) => 3,
    };
    rank(left)
        .cmp(&rank(right))
        .then_with(|| match (left, right) {
            (DependencyTarget::Cell(left), DependencyTarget::Cell(right))
            | (DependencyTarget::SpillAnchor(left), DependencyTarget::SpillAnchor(right)) => {
                left.cmp(right)
            }
            (DependencyTarget::Area(left), DependencyTarget::Area(right)) => {
                left.sort_key().cmp(&right.sort_key())
            }
            (DependencyTarget::TableIdentity(left), DependencyTarget::TableIdentity(right)) => {
                (left.table_id, left.topology.0).cmp(&(right.table_id, right.topology.0))
            }
            _ => Ordering::Equal,
        })
}

#[cfg(test)]
pub(super) fn table_dependency_by_id(
    workbook: &WorkbookSnapshot,
    table_id: TableId,
) -> Option<TableDependency> {
    table_dependency_by_id_cancellable(workbook, table_id, &|| false)
        .expect("non-cancellable table topology hashing cannot be cancelled")
}

pub(super) fn table_dependency_by_id_cancellable(
    workbook: &WorkbookSnapshot,
    table_id: TableId,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<TableDependency>, ()> {
    if cancelled() {
        return Err(());
    }
    let Some(location) = workbook.table_location_by_id(table_id) else {
        return Ok(None);
    };
    let Some(sheet) = workbook.sheets().get(location.sheet_index) else {
        return Ok(None);
    };
    let Some(table) = sheet.tables().get(location.table_index) else {
        return Ok(None);
    };
    Ok(Some(TableDependency {
        table_id,
        topology: TableTopologyRevision::from_table(
            location.sheet_index,
            sheet.id(),
            table,
            cancelled,
        )?,
    }))
}

pub(super) fn workbook_table_topologies(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<TableId, TableTopologyRevision>, ()> {
    let mut topologies = BTreeMap::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        if cancelled() {
            return Err(());
        }
        for table in sheet.tables() {
            if cancelled() {
                return Err(());
            }
            topologies.insert(
                table.id(),
                TableTopologyRevision::from_table(sheet_index, sheet.id(), table, cancelled)?,
            );
        }
    }
    Ok(topologies)
}

pub(super) fn table_topologies(
    targets: &BTreeMap<super::CellId, Vec<DependencyTarget>>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<TableId, TableTopologyRevision>, ()> {
    let mut topologies = BTreeMap::new();
    for target in targets.values().flatten() {
        if cancelled() {
            return Err(());
        }
        if let DependencyTarget::TableIdentity(table) = target {
            topologies.insert(table.table_id(), table.topology());
        }
    }
    Ok(topologies)
}

#[derive(Default)]
struct VisitedDefinitions {
    values: BTreeSet<DefinedLambdaId>,
    lambdas: BTreeSet<DefinedLambdaId>,
}

impl Engine<'_> {
    fn builtin_arguments_are_reachable(&self, name: &str, args: &[Expr]) -> bool {
        function_evaluator(name).is_none()
            || function_arguments_are_reachable(
                name,
                args,
                self.calculation_limits().max_let_bindings(),
            )
    }

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
        self.dependency_targets_cancellable(&|| false)
            .expect("non-cancellable dependency collection cannot be cancelled")
            .into_iter()
            .map(|(cell, targets)| {
                (
                    cell,
                    targets.iter().filter_map(DependencyTarget::span).collect(),
                )
            })
            .collect()
    }

    pub(super) fn dependency_targets_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BTreeMap<super::CellId, Vec<DependencyTarget>>, ()> {
        let mut result = BTreeMap::new();
        for (cell, parsed) in &self.asts {
            if cancelled() {
                return Err(());
            }
            let mut targets = Vec::new();
            let budget = EvaluationBudget::default();
            self.collect_dependency_targets(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut targets,
            );
            if cancelled() {
                return Err(());
            }
            targets.sort_by(compare_targets);
            targets.dedup();
            result.insert(*cell, targets);
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
                if !self.builtin_arguments_are_reachable(name, args) {
                    return false;
                }
                let normalized = normalize_name(name);
                if is_let_function(name) {
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
                if let Some(DependencyKind::DynamicReference(kind)) =
                    function_dependency_kind(&normalized)
                    && self.resolve_dynamic_rect(context, kind, args).is_err()
                {
                    return true;
                }
                let mut found = false;
                if walk_local_scope(
                    name,
                    args,
                    local_names,
                    self.calculation_limits().max_let_bindings(),
                    |arg, scope| {
                        found |= self
                            .expr_has_unresolved_dynamic_dependency(context, arg, visited, scope);
                    },
                ) {
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
            let mut targets = Vec::new();
            let budget = EvaluationBudget::default();
            self.collect_dependency_targets(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut targets,
            );
            if cancelled() {
                return Err(());
            }
            let mut cell_dependencies = Vec::new();
            for target in targets {
                match target {
                    DependencyTarget::Cell(cell) | DependencyTarget::SpillAnchor(cell) => {
                        if formula_cells[cell.0].contains(&(cell.1, cell.2)) {
                            cell_dependencies.push(cell);
                        }
                        if let Some(owner) = self.cancellable_array_owner(cell, cancelled)? {
                            cell_dependencies.push(owner);
                        }
                    }
                    DependencyTarget::TableIdentity(_) => {}
                    DependencyTarget::Area(span) => {
                        for rect in span.rects() {
                            if cancelled() {
                                return Err(());
                            }
                            if rect.is_single_cell() {
                                if formula_cells[rect.sheet]
                                    .contains(&(rect.row_start, rect.col_start))
                                {
                                    cell_dependencies.push((
                                        rect.sheet,
                                        rect.row_start,
                                        rect.col_start,
                                    ));
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

    fn structured_table_dependency(
        &self,
        context: EvalContext<'_>,
        reference: &StructuredReference,
    ) -> Option<TableDependency> {
        let (sheet_index, table_index) =
            self.structured_table_coordinates(context, reference).ok()?;
        let table = self
            .workbook
            .sheets()
            .get(sheet_index)?
            .tables()
            .get(table_index)?;
        Some(TableDependency {
            table_id: table.id(),
            topology: *self.table_topologies.get(&table.id())?,
        })
    }

    fn collect_dependency_targets(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
        output: &mut Vec<DependencyTarget>,
    ) {
        if context.is_cancelled() {
            return;
        }
        match expr {
            Expr::Ref(reference) => {
                if let Ok(span) = self.resolve_reference_span(context.sheet(), reference) {
                    output.push(DependencyTarget::from_span(span));
                }
            }
            Expr::StructuredRef(reference) => {
                if let Some(table) = self.structured_table_dependency(context, reference) {
                    output.push(DependencyTarget::TableIdentity(table));
                }
                if let Ok(reference) = self.resolve_reference_value_expr(context, expr) {
                    output.extend(
                        reference
                            .areas()
                            .iter()
                            .map(|area| DependencyTarget::from_span(area.as_span())),
                    );
                }
            }
            Expr::SpillRef(anchor) => {
                if let Ok(anchor_cell) = self.resolve_spill_anchor_expr(context, anchor) {
                    output.push(DependencyTarget::SpillAnchor(anchor_cell));
                }
                let mut selection_names = VisitedDefinitions::default();
                self.collect_reference_selection_inputs(
                    context,
                    anchor,
                    &mut selection_names,
                    local_names,
                    output,
                );
            }
            Expr::ReferenceUnion { .. } | Expr::ReferenceIntersection { .. } => {
                if let Ok(reference) = self.resolve_reference_value_expr(context, expr) {
                    output.extend(
                        reference
                            .areas()
                            .iter()
                            .map(|area| DependencyTarget::from_span(area.as_span())),
                    );
                }
                let mut selection_names = VisitedDefinitions::default();
                self.collect_reference_selection_inputs(
                    context,
                    expr,
                    &mut selection_names,
                    local_names,
                    output,
                );
            }
            Expr::Range { start, end } => {
                if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                    output.push(DependencyTarget::from_span(RectSpan::single(rect)));
                }
                self.collect_dependency_targets(context, start, visited, local_names, output);
                self.collect_dependency_targets(context, end, visited, local_names, output);
            }
            Expr::Name(name) => {
                if context.binding(name).is_some() || is_local_name(name, local_names) {
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                    && visited.values.insert(id.clone())
                {
                    self.collect_dependency_targets(
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
                    output.push(DependencyTarget::from_span(RectSpan::single(rect)));
                    let mut selection_names = VisitedDefinitions::default();
                    self.collect_reference_selection_inputs(
                        context,
                        inner,
                        &mut selection_names,
                        local_names,
                        output,
                    );
                } else {
                    self.collect_dependency_targets(context, inner, visited, local_names, output);
                }
            }
            Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
                self.collect_dependency_targets(context, inner, visited, local_names, output);
            }
            Expr::Binary { left, right, .. } => {
                self.collect_dependency_targets(context, left, visited, local_names, output);
                self.collect_dependency_targets(context, right, visited, local_names, output);
            }
            Expr::Call { name, args } => {
                if let Some(binding) = context.binding(name) {
                    if !matches!(binding, ScopeValue::Callable(_)) {
                        return;
                    }
                    for arg in args {
                        self.collect_dependency_targets(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if is_local_name(name, local_names) {
                    for arg in args {
                        self.collect_dependency_targets(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    if let Some(lambda) = crate::calculation::lambda::definition(named) {
                        if visited.lambdas.insert(id.clone()) {
                            let mut lambda_names = lambda.parameters().to_vec();
                            self.collect_dependency_targets(
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
                            self.collect_dependency_targets(
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
                if !self.builtin_arguments_are_reachable(name, args) {
                    return;
                }
                let normalized = normalize_name(name);
                if crate::calculation::functions::uses_reference_metadata_only(&normalized) {
                    let mut selection_names = VisitedDefinitions::default();
                    for arg in args {
                        self.collect_reference_selection_inputs(
                            context,
                            arg,
                            &mut selection_names,
                            local_names,
                            output,
                        );
                    }
                    return;
                }
                if is_let_function(name) {
                    let _ =
                        with_let_scope(self, context, args, |engine, scoped, arg, final_arg| {
                            engine.collect_dependency_targets(
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
                if matches!(
                    function_dependency_kind(&normalized),
                    Some(DependencyKind::ResizedCriteriaValueRange)
                ) && args.len() == 3
                    && let (Ok(criteria_range), Ok(value_anchor)) = (
                        self.resolve_rect_expr(context, &args[0]),
                        self.resolve_rect_expr(context, &args[2]),
                    )
                    && let Some(value_range) = value_anchor
                        .resized_from_anchor(criteria_range.height(), criteria_range.width())
                {
                    output.push(DependencyTarget::from_span(RectSpan::single(value_range)));
                }
                if let Some(DependencyKind::DynamicReference(kind)) =
                    function_dependency_kind(&normalized)
                    && let Ok(rect) = self.resolve_dynamic_rect(context, kind, args)
                {
                    output.push(DependencyTarget::from_span(RectSpan::single(rect)));
                }
                if walk_local_scope(
                    name,
                    args,
                    local_names,
                    self.calculation_limits().max_let_bindings(),
                    |arg, scope| {
                        self.collect_dependency_targets(context, arg, visited, scope, output);
                    },
                ) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_targets(context, arg, visited, local_names, output);
                }
            }
            Expr::Invoke { callee, args } => {
                self.collect_dependency_targets(context, callee, visited, local_names, output);
                for arg in args {
                    self.collect_dependency_targets(context, arg, visited, local_names, output);
                }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for element in row {
                        self.collect_dependency_targets(
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
        output: &mut Vec<DependencyTarget>,
    ) {
        if context.is_cancelled() {
            return;
        }
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.collect_reference_selection_inputs(
                    context,
                    inner,
                    visited,
                    local_names,
                    output,
                );
            }
            Expr::SpillRef(anchor) => {
                if let Ok(anchor_cell) = self.resolve_spill_anchor_expr(context, anchor) {
                    output.push(DependencyTarget::SpillAnchor(anchor_cell));
                }
                self.collect_reference_selection_inputs(
                    context,
                    anchor,
                    visited,
                    local_names,
                    output,
                );
            }
            Expr::StructuredRef(reference) => {
                if let Some(table) = self.structured_table_dependency(context, reference) {
                    output.push(DependencyTarget::TableIdentity(table));
                }
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
                        self.collect_dependency_targets(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if is_local_name(name, local_names) {
                    for arg in args {
                        self.collect_dependency_targets(context, arg, visited, local_names, output);
                    }
                    return;
                }
                if let Some((id, named)) = self.resolve_name_expr_with_id_in_context(context, name)
                {
                    if let Some(lambda) = crate::calculation::lambda::definition(named) {
                        if visited.lambdas.insert(id.clone()) {
                            let mut lambda_names = lambda.parameters().to_vec();
                            self.collect_dependency_targets(
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
                            self.collect_dependency_targets(
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
                if !self.builtin_arguments_are_reachable(name, args) {
                    return;
                }
                if is_let_function(name) {
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
                                engine.collect_dependency_targets(
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
                if walk_local_scope(
                    name,
                    args,
                    local_names,
                    self.calculation_limits().max_let_bindings(),
                    |arg, scope| {
                        self.collect_dependency_targets(context, arg, visited, scope, output);
                    },
                ) {
                    return;
                }
                for arg in args {
                    self.collect_dependency_targets(context, arg, visited, local_names, output);
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
                if !self.builtin_arguments_are_reachable(name, args) {
                    return false;
                }
                if matches!(
                    function_dependency_kind(name),
                    Some(DependencyKind::DynamicReference(_))
                ) {
                    return true;
                }
                let mut found = false;
                if walk_local_scope(
                    name,
                    args,
                    local_names,
                    self.calculation_limits().max_let_bindings(),
                    |arg, scope| {
                        found |= self
                            .expr_contains_dynamic_reference_function(context, arg, visited, scope);
                    },
                ) {
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
