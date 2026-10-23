use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::dependency_areas::AreaNodes;
use super::{Engine, EvalContext, EvaluationBudget};
use crate::calculation::ast::{Expr, SheetPrefix, StructuredReference};
use crate::calculation::functions::descriptor::{DependencyKind, DynamicReferenceKind};
use crate::calculation::functions::kernel::LegacyFunction;
use crate::calculation::functions::{
    CallableArity, CallableShadow, DynamicFunction, Evaluator,
    builtin_invocation_arguments_are_reachable, callable_shadow_arguments_are_reachable,
    direct_builtin_callable, function_arguments_are_reachable, function_dependency_kind,
    function_evaluator, normalize_name, prepare_evaluator_arguments, with_let_scope,
};
use crate::calculation::graph::{DependencyGraph, DependencyNode};
use crate::calculation::lambda::{is_local_name, walk_local_scope};
use crate::calculation::runtime::{Rect, RectSpan, ReferenceArea, ReferenceValue};
use crate::calculation::scope::{CallableValue, DefinedLambdaId, ScopeValue};
use crate::{CellAddress, CellContent, DefinedNameScope};
use crate::{SheetId, Table, TableId, WorkbookSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TableTopologyRevision([u8; 32]);

fn is_let_function(name: &str) -> bool {
    function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Let))
}

/// How far the area of a range operator operand can reach before any cell is evaluated.
enum RangeExtent {
    /// The area does not depend on cell values.
    Fixed(Rect),
    /// The area follows cell values but always lies inside this rectangle, as an INDEX end does.
    Bounded(Rect),
    /// The area follows cell values beyond any static bound.
    ValueDependent,
    /// The operand is not a reference.
    Invalid,
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
    FormulaContent(super::CellId),
    /// Every cell a reference end that moves with values can reach. Impact tracking watches it;
    /// calculation order follows only the cells the reference covers when it is collected.
    ImpactArea(RectSpan),
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
            Self::Cell((sheet, row, column))
            | Self::SpillAnchor((sheet, row, column))
            | Self::FormulaContent((sheet, row, column)) => Some(RectSpan::single(Rect {
                sheet: *sheet,
                row_start: *row,
                col_start: *column,
                row_end: *row,
                col_end: *column,
                whole_rows: false,
            })),
            Self::Area(span) => Some(span.clone()),
            Self::TableIdentity(_) | Self::ImpactArea(_) => None,
        }
    }
}

fn compare_targets(left: &DependencyTarget, right: &DependencyTarget) -> Ordering {
    let rank = |target: &DependencyTarget| match target {
        DependencyTarget::Cell(_) => 0_u8,
        DependencyTarget::Area(_) => 1,
        DependencyTarget::TableIdentity(_) => 2,
        DependencyTarget::SpillAnchor(_) => 3,
        DependencyTarget::FormulaContent(_) => 4,
        DependencyTarget::ImpactArea(_) => 5,
    };
    rank(left)
        .cmp(&rank(right))
        .then_with(|| match (left, right) {
            (DependencyTarget::Cell(left), DependencyTarget::Cell(right))
            | (DependencyTarget::SpillAnchor(left), DependencyTarget::SpillAnchor(right))
            | (DependencyTarget::FormulaContent(left), DependencyTarget::FormulaContent(right)) => {
                left.cmp(right)
            }
            (DependencyTarget::Area(left), DependencyTarget::Area(right))
            | (DependencyTarget::ImpactArea(left), DependencyTarget::ImpactArea(right)) => {
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

/// The dependency graph of every formula, the areas built for it, and the formulas left out
/// because the graph reached `max_dependency_edges`.
pub(super) struct CollectedDependencies {
    pub(super) graph: DependencyGraph,
    pub(super) areas: AreaNodes,
    pub(super) limited: BTreeSet<super::CellId>,
}

impl CollectedDependencies {
    fn limited_only(limited: BTreeSet<super::CellId>) -> Self {
        Self {
            graph: DependencyGraph::default(),
            areas: AreaNodes::default(),
            limited,
        }
    }
}

#[derive(Default)]
struct VisitedDefinitions {
    values: BTreeSet<DefinedLambdaId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceSelectionMode {
    ReferenceValue,
    FormulaMetadata,
}

fn callable_shadow_from_scope_value(value: &ScopeValue) -> CallableShadow {
    match value {
        ScopeValue::Callable(CallableValue::Lambda(closure)) => {
            CallableShadow::Callable(CallableArity::Exact(closure.parameters.len()))
        }
        ScopeValue::Callable(CallableValue::Builtin(callable)) => {
            CallableShadow::Callable(CallableArity::Builtin(*callable))
        }
        ScopeValue::Missing
        | ScopeValue::Scalar(_)
        | ScopeValue::Array(_)
        | ScopeValue::Reference(_) => CallableShadow::DefinitelyNonCallable,
    }
}

impl Engine<'_> {
    /// Resolves one name-like expression under the dependency walk's shared scope rules.
    ///
    /// Each dependency pass still owns its output and short-circuit policy. This helper only
    /// centralizes the semantic boundary that must not drift between passes: bound/local names
    /// are opaque, defined names are followed once, their bindings are cleared, and lookup
    /// continues in the definition's scope with a fresh local-name stack.
    fn reachable_defined_name<'engine, 'scope>(
        &'engine self,
        context: EvalContext<'scope>,
        name: &str,
        visited: &mut VisitedDefinitions,
        local_names: &[String],
    ) -> Option<(EvalContext<'scope>, &'engine Expr)> {
        if context.binding(name).is_some() || is_local_name(name, local_names) {
            return None;
        }
        let (id, named) = self.resolve_name_expr_with_id_in_context(context, name)?;
        if !visited.values.insert(id.clone()) {
            return None;
        }
        Some((
            context
                .without_bindings()
                .with_defined_name_scope(Some(id.scope())),
            named,
        ))
    }

    /// Resolves a sheet-qualified name such as `Data!Rate` to its definition, in the definition's
    /// own scope, unless the definition is already being expanded.
    fn reachable_qualified_name<'engine, 'scope>(
        &'engine self,
        context: EvalContext<'scope>,
        sheet: &SheetPrefix,
        name: &str,
        visited: &mut VisitedDefinitions,
    ) -> Option<(EvalContext<'scope>, &'engine Expr)> {
        if sheet.end_name.is_some() {
            return None;
        }
        let sheet_index = self.workbook.sheet_index_by_name(&sheet.name)?;
        let scope = DefinedNameScope::Sheet(self.workbook.sheets()[sheet_index].id());
        let (id, named) =
            self.resolve_name_expr_with_id_for_scope(sheet_index, Some(scope), name)?;
        if !visited.values.insert(id.clone()) {
            return None;
        }
        Some((
            context
                .without_bindings()
                .with_defined_name_scope(Some(id.scope())),
            named,
        ))
    }

    fn builtin_arguments_are_reachable(&self, name: &str, args: &[Expr]) -> bool {
        function_evaluator(name).is_none()
            || function_arguments_are_reachable(
                name,
                args,
                self.calculation_limits().max_let_bindings(),
            )
    }

    fn builtin_invocation_arguments_are_reachable(
        &self,
        context: EvalContext<'_>,
        callee: &Expr,
        args: &[Expr],
        local_names: &[String],
    ) -> bool {
        crate::calculation::stack::grow(|| {
            builtin_invocation_arguments_are_reachable(callee, args, |name| {
                if let Some(value) = context.binding(name) {
                    return match value {
                        ScopeValue::Callable(CallableValue::Lambda(closure)) => {
                            CallableShadow::Callable(CallableArity::Exact(closure.parameters.len()))
                        }
                        ScopeValue::Callable(CallableValue::Builtin(callable)) => {
                            CallableShadow::Callable(CallableArity::Builtin(*callable))
                        }
                        ScopeValue::Missing
                        | ScopeValue::Scalar(_)
                        | ScopeValue::Array(_)
                        | ScopeValue::Reference(_) => CallableShadow::DefinitelyNonCallable,
                    };
                }
                if is_local_name(name, local_names) {
                    return CallableShadow::Unknown;
                }
                self.callable_shadow_for_name(context.sheet(), context.defined_name_scope(), name)
            })
        })
    }

    fn builtin_invocation_callee_is_reachable(
        &self,
        context: EvalContext<'_>,
        callee: &Expr,
        local_names: &[String],
    ) -> bool {
        let Some(callable) = direct_builtin_callable(callee) else {
            return true;
        };
        let name = callable.canonical_name();
        let shadow = if let Some(value) = context.binding(name) {
            callable_shadow_from_scope_value(value)
        } else if is_local_name(name, local_names) {
            CallableShadow::Unknown
        } else {
            self.callable_shadow_for_name(context.sheet(), context.defined_name_scope(), name)
        };
        shadow != CallableShadow::CyclicNonCallable
    }

    fn shadowed_call_arguments_are_reachable(
        &self,
        context: EvalContext<'_>,
        name: &str,
        argument_count: usize,
        local_names: &[String],
    ) -> Option<(CallableShadow, bool)> {
        if let Some(value) = context.binding(name) {
            let shadow = callable_shadow_from_scope_value(value);
            return Some((
                shadow,
                callable_shadow_arguments_are_reachable(shadow, None, argument_count),
            ));
        }
        if is_local_name(name, local_names) {
            return Some((CallableShadow::Unknown, true));
        }
        self.resolve_name_expr_with_id_in_context(context, name)
            .map(|_| {
                let shadow = self.callable_shadow_for_name(
                    context.sheet(),
                    context.defined_name_scope(),
                    name,
                );
                (
                    shadow,
                    callable_shadow_arguments_are_reachable(shadow, None, argument_count),
                )
            })
    }

    pub(super) fn dependencies_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<CollectedDependencies, ()> {
        self.collect_dependencies(true, cancelled)
    }

    pub(super) fn dependency_limit_cells_for_analysis(&self) -> BTreeSet<super::CellId> {
        self.collect_dependencies(false, &|| false)
            .expect("non-cancellable dependency collection cannot be cancelled")
            .limited
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
        for (cell, parsed) in self.asts.iter() {
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

    /// Bounds the areas a range operator operand can take, whatever the cell values are.
    ///
    /// An INDEX end always lands inside its first argument, so that argument bounds it even though
    /// the cell it selects follows values. Ends computed by LET, a LAMBDA, OFFSET, INDIRECT, or a
    /// spill have no such bound. `visited` holds the defined names being expanded, so a cyclic
    /// name ends the expansion.
    fn range_extent(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        local_names: &[String],
        visited: &mut Vec<DefinedLambdaId>,
    ) -> RangeExtent {
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
            if context.is_cancelled() {
                return RangeExtent::Invalid;
            }
            match expr {
                Expr::Paren(inner) => self.range_extent(context, inner, local_names, visited),
                Expr::Range { start, end }
                | Expr::ReferenceUnion {
                    left: start,
                    right: end,
                } => {
                    let start = self.range_extent(context, start, local_names, visited);
                    let end = self.range_extent(context, end, local_names, visited);
                    let (start_rect, end_rect, fixed) = match (start, end) {
                        (RangeExtent::Fixed(start), RangeExtent::Fixed(end)) => (start, end, true),
                        (
                            RangeExtent::Fixed(start) | RangeExtent::Bounded(start),
                            RangeExtent::Fixed(end) | RangeExtent::Bounded(end),
                        ) => (start, end, false),
                        (RangeExtent::ValueDependent, _) | (_, RangeExtent::ValueDependent) => {
                            return RangeExtent::ValueDependent;
                        }
                        _ => return RangeExtent::Invalid,
                    };
                    match ReferenceValue::from_areas(vec![
                        ReferenceArea::Rect(start_rect),
                        ReferenceArea::Rect(end_rect),
                    ])
                    .bounding_rect()
                    {
                        Ok(rect) if fixed => RangeExtent::Fixed(rect),
                        Ok(rect) => RangeExtent::Bounded(rect),
                        Err(_) => RangeExtent::Invalid,
                    }
                }
                // An intersection lies inside its left operand.
                Expr::ReferenceIntersection { left, .. } => {
                    self.range_extent(context, left, local_names, visited)
                }
                Expr::Call { name, args } => {
                    if is_local_name(name, local_names)
                        || self
                            .resolve_name_expr_with_id_in_context(context, name)
                            .is_some()
                    {
                        return RangeExtent::ValueDependent;
                    }
                    match function_evaluator(name) {
                        Some(Evaluator::Legacy(LegacyFunction::Index)) => {
                            let Some(prepared) = prepare_evaluator_arguments(
                                Evaluator::Legacy(LegacyFunction::Index),
                                args,
                            ) else {
                                return RangeExtent::Invalid;
                            };
                            match self.range_extent(context, &prepared[0], local_names, visited) {
                                RangeExtent::Fixed(rect) | RangeExtent::Bounded(rect) => {
                                    RangeExtent::Bounded(rect)
                                }
                                other => other,
                            }
                        }
                        Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
                            RangeExtent::ValueDependent
                        }
                        _ if matches!(
                            function_dependency_kind(name),
                            Some(DependencyKind::DynamicReference(_))
                        ) =>
                        {
                            RangeExtent::ValueDependent
                        }
                        _ => RangeExtent::Invalid,
                    }
                }
                Expr::Invoke { .. } | Expr::SpillRef(_) => RangeExtent::ValueDependent,
                Expr::Name(name)
                    if is_local_name(name, local_names) || context.binding(name).is_some() =>
                {
                    RangeExtent::ValueDependent
                }
                Expr::Name(name) => {
                    let Some((id, named)) =
                        self.resolve_name_expr_with_id_in_context(context, name)
                    else {
                        return RangeExtent::Invalid;
                    };
                    if visited.contains(&id) {
                        return RangeExtent::Invalid;
                    }
                    visited.push(id.clone());
                    let extent = self.range_extent(
                        context
                            .without_bindings()
                            .with_defined_name_scope(Some(id.scope())),
                        named,
                        &[],
                        visited,
                    );
                    visited.pop();
                    extent
                }
                Expr::Ref(_) | Expr::StructuredRef(_) => self
                    .resolve_reference_value_expr(context, expr)
                    .and_then(|reference| reference.bounding_rect())
                    .map_or(RangeExtent::Invalid, RangeExtent::Fixed),
                _ => RangeExtent::Invalid,
            }
        })
    }

    /// Reports whether a formula or an array result lies in `rect`.
    fn rect_holds_calculated_cells(&self, rect: &Rect) -> bool {
        let rows = (rect.sheet, rect.row_start, 0)..=(rect.sheet, rect.row_end, u32::MAX);
        let in_columns = |cell: &super::CellId| (rect.col_start..=rect.col_end).contains(&cell.2);
        self.asts
            .range(rows.clone())
            .any(|(cell, _)| in_columns(cell))
            || self
                .parse_failures
                .range(rows)
                .any(|(cell, _)| in_columns(cell))
            || self
                .array_regions
                .iter()
                .any(|region| rects_intersect(&region.rect, rect))
    }

    /// Reports whether a formula's references depend on values, as dynamic references do.
    pub(super) fn formula_contains_dynamic_reference(
        &self,
        cell: super::CellId,
        cancelled: &impl Fn() -> bool,
    ) -> bool {
        self.asts.get(&cell).is_some_and(|parsed| {
            let budget = EvaluationBudget::default();
            self.expr_contains_dynamic_reference_function(
                false,
                EvalContext::for_cancellable(cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
            )
        })
    }

    /// Reports whether the workbook holds a dynamic array formula without a declared spill range,
    /// whose spill area is known only after it is evaluated.
    pub(super) fn has_undeclared_dynamic_arrays(
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
        Ok(false)
    }

    /// Reports whether a formula's references depend on values in a way incremental calculation
    /// cannot follow: a dynamic reference, or a range whose moving end can reach a formula or an
    /// array result.
    pub(super) fn formula_is_incrementally_unstable(
        &self,
        cell: super::CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        let Some(parsed) = self.asts.get(&cell) else {
            return Ok(false);
        };
        let budget = EvaluationBudget::default();
        let found = self.expr_contains_dynamic_reference_function(
            true,
            EvalContext::for_cancellable(cell, &budget, cancelled),
            parsed.root(),
            &mut VisitedDefinitions::default(),
            &mut Vec::new(),
        );
        if cancelled() {
            return Err(());
        }
        Ok(found)
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
        for cell in self.asts.keys() {
            if cancelled() {
                return Err(());
            }
            result.insert(*cell, self.formula_dependency_targets(*cell, cancelled)?);
        }
        Ok(result)
    }

    /// Returns the dependency targets of the formula at `cell`, in target order.
    pub(super) fn formula_dependency_targets(
        &self,
        cell: super::CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<DependencyTarget>, ()> {
        let mut targets = Vec::new();
        if let Some(parsed) = self.asts.get(&cell) {
            let budget = EvaluationBudget::default();
            self.collect_dependency_targets(
                EvalContext::for_cancellable(cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut targets,
            );
        }
        if cancelled() {
            return Err(());
        }
        targets.sort_by(compare_targets);
        targets.dedup();
        Ok(targets)
    }

    pub(super) fn target_dependencies(
        &self,
        cell: super::CellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<DependencyTarget>, ()> {
        let mut targets = Vec::new();
        if !self.name_cycle_cells.contains(&cell)
            && !self.name_limit_cells.contains(&cell)
            && let Some(parsed) = self.asts.get(&cell)
        {
            let budget = EvaluationBudget::default();
            self.collect_dependency_targets(
                EvalContext::for_cancellable(cell, &budget, cancelled),
                parsed.root(),
                &mut VisitedDefinitions::default(),
                &mut Vec::new(),
                &mut targets,
            );
        }
        if cancelled() {
            return Err(());
        }
        targets.sort_by(compare_targets);
        targets.dedup();
        Ok(targets)
    }

    fn expr_has_unresolved_dynamic_dependency(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
    ) -> bool {
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
            if context.is_cancelled() {
                return true;
            }
            match expr {
                Expr::Call { name, args } => {
                    if let Some((shadow, arguments_are_reachable)) = self
                        .shadowed_call_arguments_are_reachable(
                            context,
                            name,
                            args.len(),
                            local_names,
                        )
                    {
                        let mut found = false;
                        if shadow != CallableShadow::CyclicNonCallable
                            && let Some((id, named)) =
                                self.resolve_name_expr_with_id_in_context(context, name)
                            && visited.values.insert(id.clone())
                        {
                            found |= self.expr_has_unresolved_dynamic_dependency(
                                context
                                    .without_bindings()
                                    .with_defined_name_scope(Some(id.scope())),
                                named,
                                visited,
                                &mut Vec::new(),
                            );
                        }
                        return found
                            || (arguments_are_reachable
                                && args.iter().any(|arg| {
                                    self.expr_has_unresolved_dynamic_dependency(
                                        context,
                                        arg,
                                        visited,
                                        local_names,
                                    )
                                }));
                    }
                    if !self.builtin_arguments_are_reachable(name, args) {
                        return false;
                    }
                    let normalized = normalize_name(name);
                    if is_let_function(name) {
                        let mut found = false;
                        let result = with_let_scope(
                            self,
                            context,
                            args,
                            |engine, scoped, arg, final_arg| {
                                found |= engine.expr_has_unresolved_dynamic_dependency(
                                    scoped,
                                    arg,
                                    visited,
                                    local_names,
                                );
                                final_arg.then_some(())
                            },
                        );
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
                            found |= self.expr_has_unresolved_dynamic_dependency(
                                context, arg, visited, scope,
                            );
                        },
                    ) {
                        return found;
                    }
                    args.iter().any(|arg| {
                        self.expr_has_unresolved_dynamic_dependency(
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    })
                }
                Expr::Invoke { callee, args } => {
                    let callee_has_dependency =
                        self.builtin_invocation_callee_is_reachable(context, callee, local_names)
                            && self.expr_has_unresolved_dynamic_dependency(
                                context,
                                callee,
                                visited,
                                local_names,
                            );
                    callee_has_dependency
                        || (self.builtin_invocation_arguments_are_reachable(
                            context,
                            callee,
                            args,
                            local_names,
                        ) && args.iter().any(|arg| {
                            self.expr_has_unresolved_dynamic_dependency(
                                context,
                                arg,
                                visited,
                                local_names,
                            )
                        }))
                }
                Expr::Name(name) => {
                    let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    else {
                        return false;
                    };
                    self.expr_has_unresolved_dynamic_dependency(
                        defined_context,
                        named,
                        visited,
                        &mut Vec::new(),
                    )
                }
                Expr::BuiltinCallable(callable) => {
                    let name = callable.canonical_name();
                    let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    else {
                        return false;
                    };
                    self.expr_has_unresolved_dynamic_dependency(
                        defined_context,
                        named,
                        visited,
                        &mut Vec::new(),
                    )
                }
                Expr::Range { .. }
                    if matches!(
                        self.range_extent(context, expr, local_names, &mut Vec::new()),
                        RangeExtent::Bounded(_) | RangeExtent::ValueDependent
                    ) && self.resolve_rect_expr(context, expr).is_err() =>
                {
                    true
                }
                Expr::ImplicitIntersection(inner)
                | Expr::SpillRef(inner)
                | Expr::Paren(inner)
                | Expr::Unary { operand: inner, .. } => self
                    .expr_has_unresolved_dynamic_dependency(context, inner, visited, local_names),
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
                    self.expr_has_unresolved_dynamic_dependency(
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
                | Expr::Missing => false,
                Expr::QualifiedName { sheet, name } => self
                    .reachable_qualified_name(context, sheet, name, visited)
                    .is_some_and(|(defined_context, named)| {
                        self.expr_has_unresolved_dynamic_dependency(
                            defined_context,
                            named,
                            visited,
                            &mut Vec::new(),
                        )
                    }),
            }
        })
    }

    /// Collects every formula's dependencies until their total reaches `max_dependency_edges`.
    ///
    /// The formula that would cross the limit and every formula after it are returned instead of
    /// collected: their dependencies are unknown, so they report the limit, and formulas that
    /// depend on them read that error. Formulas collected before the limit calculate normally.
    ///
    /// A formula counts the edges it adds to the graph: one per node it waits for and one per
    /// member of each area it builds. An area another formula already built adds nothing but the
    /// edge to it. A formula never counts more than the distinct formula cells it waits for, the
    /// count before ranges were stored as shared areas.
    fn collect_dependencies(
        &self,
        retain_graph: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<CollectedDependencies, ()> {
        let mut graph = DependencyGraph::default();
        let mut areas = AreaNodes::default();
        if cancelled() {
            return Err(());
        }
        let mut edge_count = 0_u64;
        let mut formulas = self.asts.iter();
        while let Some((cell, parsed)) = formulas.next() {
            if cancelled() {
                return Err(());
            }
            if self.name_cycle_cells.contains(cell) || self.name_limit_cells.contains(cell) {
                graph.insert(*cell, Vec::new());
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
            let area_edges = graph.area_edges();
            let nodes = self.dependency_nodes(&targets, &mut graph, &mut areas, cancelled)?;
            let added = nodes.len() as u64 + (graph.area_edges() - area_edges);
            let edges = if graph.area_edges() == area_edges
                && nodes
                    .iter()
                    .all(|node| matches!(node, DependencyNode::Cell(_)))
            {
                added
            } else {
                graph.count_cells(&nodes, added)
            };
            match edge_count
                .checked_add(edges)
                .filter(|total| *total <= self.options.limits().max_dependency_edges())
            {
                Some(total) => edge_count = total,
                None => {
                    let mut limited = BTreeSet::from([*cell]);
                    for (cell, _) in formulas {
                        if cancelled() {
                            return Err(());
                        }
                        limited.insert(*cell);
                    }
                    if !retain_graph {
                        return Ok(CollectedDependencies::limited_only(limited));
                    }
                    for cell in &limited {
                        graph.insert(*cell, Vec::new());
                    }
                    return Ok(CollectedDependencies {
                        graph,
                        areas,
                        limited,
                    });
                }
            }
            graph.insert(*cell, nodes);
        }
        if !retain_graph {
            return Ok(CollectedDependencies::limited_only(BTreeSet::new()));
        }
        Ok(CollectedDependencies {
            graph,
            areas,
            limited: BTreeSet::new(),
        })
    }

    /// Returns the nodes a formula with `targets` waits for, building the areas its multi-cell
    /// ranges need.
    pub(super) fn dependency_nodes(
        &self,
        targets: &[DependencyTarget],
        graph: &mut DependencyGraph,
        areas: &mut AreaNodes,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<DependencyNode>, ()> {
        let mut nodes = Vec::new();
        for target in targets {
            match target {
                DependencyTarget::Cell(cell) | DependencyTarget::SpillAnchor(cell) => {
                    self.push_cell_dependency(*cell, &mut nodes, cancelled)?;
                }
                DependencyTarget::TableIdentity(_)
                | DependencyTarget::FormulaContent(_)
                | DependencyTarget::ImpactArea(_) => {}
                DependencyTarget::Area(span) => {
                    for rect in span.rects() {
                        if cancelled() {
                            return Err(());
                        }
                        if rect.is_single_cell() {
                            self.push_cell_dependency(
                                (rect.sheet, rect.row_start, rect.col_start),
                                &mut nodes,
                                cancelled,
                            )?;
                        } else if let Some(node) = areas.range(self, graph, &rect, cancelled)? {
                            nodes.push(node);
                        }
                    }
                }
            }
        }
        nodes.sort_unstable();
        nodes.dedup();
        Ok(nodes)
    }

    /// Adds the formula at `cell` and the array formula covering it, if any, to `nodes`.
    fn push_cell_dependency(
        &self,
        cell: super::CellId,
        nodes: &mut Vec<DependencyNode>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        if self.workbook.sheets()[cell.0]
            .formula_addresses()
            .contains(&CellAddress::from_indices(cell.1, cell.2).expect("resolved cell address"))
        {
            nodes.push(DependencyNode::Cell(cell));
        }
        if let Some(owner) = self.cancellable_array_owner(cell, cancelled)? {
            nodes.push(DependencyNode::Cell(owner));
        }
        Ok(())
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
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
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
                        ReferenceSelectionMode::ReferenceValue,
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
                        ReferenceSelectionMode::ReferenceValue,
                        expr,
                        &mut selection_names,
                        local_names,
                        output,
                    );
                }
                Expr::Range { start, end } => {
                    // Calculation follows the cells the range covers now. An end such as INDEX
                    // moves with cell values, so impact tracking also watches every cell it can
                    // reach, and formulas with such a range count as dynamic references.
                    if let Ok(rect) = self.resolve_rect_expr(context, expr) {
                        output.push(DependencyTarget::from_span(RectSpan::single(rect)));
                    }
                    if let RangeExtent::Bounded(rect) =
                        self.range_extent(context, expr, local_names, &mut Vec::new())
                    {
                        output.push(DependencyTarget::ImpactArea(RectSpan::single(rect)));
                    }
                    self.collect_dependency_targets(context, start, visited, local_names, output);
                    self.collect_dependency_targets(context, end, visited, local_names, output);
                }
                Expr::Name(name) => {
                    if let Some(binding) = context.binding(name) {
                        if let ScopeValue::Reference(reference) = binding {
                            output.extend(
                                reference
                                    .areas()
                                    .iter()
                                    .map(|area| DependencyTarget::from_span(area.as_span())),
                            );
                        }
                        return;
                    }
                    if let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    {
                        self.collect_dependency_targets(
                            defined_context,
                            named,
                            visited,
                            &mut Vec::new(),
                            output,
                        );
                    }
                }
                Expr::BuiltinCallable(callable) => {
                    let name = callable.canonical_name();
                    if let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    {
                        self.collect_dependency_targets(
                            defined_context,
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
                            ReferenceSelectionMode::ReferenceValue,
                            inner,
                            &mut selection_names,
                            local_names,
                            output,
                        );
                    } else {
                        self.collect_dependency_targets(
                            context,
                            inner,
                            visited,
                            local_names,
                            output,
                        );
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
                    if let Some((shadow, arguments_are_reachable)) = self
                        .shadowed_call_arguments_are_reachable(
                            context,
                            name,
                            args.len(),
                            local_names,
                        )
                    {
                        if shadow != CallableShadow::CyclicNonCallable
                            && let Some((id, named)) =
                                self.resolve_name_expr_with_id_in_context(context, name)
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
                        if arguments_are_reachable {
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
                    if let Some(DependencyKind::ReferenceMetadataOnly(kind)) =
                        function_dependency_kind(&normalized)
                    {
                        let mut selection_names = VisitedDefinitions::default();
                        for arg in args {
                            if matches!(
                            kind,
                            crate::calculation::functions::descriptor::ReferenceMetadataKind::FormulaPredicate
                                | crate::calculation::functions::descriptor::ReferenceMetadataKind::FormulaText
                        ) && let Ok(reference) = self.resolve_reference_value_expr(context, arg)
                            && let Ok(rect) = reference.single_rect()
                        {
                            output.push(DependencyTarget::FormulaContent((
                                rect.sheet,
                                rect.row_start,
                                rect.col_start,
                            )));
                        }
                            self.collect_reference_selection_inputs(
                                context,
                                ReferenceSelectionMode::FormulaMetadata,
                                arg,
                                &mut selection_names,
                                local_names,
                                output,
                            );
                        }
                        return;
                    }
                    if is_let_function(name) {
                        let _ = with_let_scope(
                            self,
                            context,
                            args,
                            |engine, scoped, arg, final_arg| {
                                engine.collect_dependency_targets(
                                    scoped,
                                    arg,
                                    visited,
                                    local_names,
                                    output,
                                );
                                final_arg.then_some(())
                            },
                        );
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
                    if self.builtin_invocation_callee_is_reachable(context, callee, local_names) {
                        self.collect_dependency_targets(
                            context,
                            callee,
                            visited,
                            local_names,
                            output,
                        );
                    }
                    if !self.builtin_invocation_arguments_are_reachable(
                        context,
                        callee,
                        args,
                        local_names,
                    ) {
                        return;
                    }
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
                | Expr::Missing => {}
                // Calculation reports a sheet-qualified name as unsupported, but impact tracking
                // still follows the cells its definition reads.
                Expr::QualifiedName { sheet, name } => {
                    if let Some((defined_context, named)) =
                        self.reachable_qualified_name(context, sheet, name, visited)
                    {
                        self.collect_dependency_targets(
                            defined_context,
                            named,
                            visited,
                            &mut Vec::new(),
                            output,
                        );
                    }
                }
            }
        })
    }

    fn collect_reference_selection_inputs(
        &self,
        context: EvalContext<'_>,
        mode: ReferenceSelectionMode,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
        output: &mut Vec<DependencyTarget>,
    ) {
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
            if context.is_cancelled() {
                return;
            }
            match expr {
                Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                    self.collect_reference_selection_inputs(
                        context,
                        mode,
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
                        mode,
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
                        mode,
                        start,
                        visited,
                        local_names,
                        output,
                    );
                    self.collect_reference_selection_inputs(
                        context,
                        mode,
                        end,
                        visited,
                        local_names,
                        output,
                    );
                }
                Expr::Name(name) => {
                    if let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    {
                        self.collect_reference_selection_inputs(
                            defined_context,
                            mode,
                            named,
                            visited,
                            &mut Vec::new(),
                            output,
                        );
                    }
                }
                Expr::BuiltinCallable(callable) => {
                    let name = callable.canonical_name();
                    if let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    {
                        self.collect_reference_selection_inputs(
                            defined_context,
                            mode,
                            named,
                            visited,
                            &mut Vec::new(),
                            output,
                        );
                    }
                }
                Expr::Call { name, args } => {
                    if let Some((shadow, arguments_are_reachable)) = self
                        .shadowed_call_arguments_are_reachable(
                            context,
                            name,
                            args.len(),
                            local_names,
                        )
                    {
                        if shadow != CallableShadow::CyclicNonCallable
                            && let Some((id, named)) =
                                self.resolve_name_expr_with_id_in_context(context, name)
                            && visited.values.insert(id.clone())
                        {
                            self.collect_reference_selection_inputs(
                                context
                                    .without_bindings()
                                    .with_defined_name_scope(Some(id.scope())),
                                mode,
                                named,
                                visited,
                                &mut Vec::new(),
                                output,
                            );
                        }
                        if arguments_are_reachable {
                            for arg in args {
                                self.collect_reference_selection_inputs(
                                    context,
                                    mode,
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
                        let _ = with_let_scope(
                            self,
                            context,
                            args,
                            |engine, scoped, arg, final_arg| {
                                if final_arg
                                    || (mode == ReferenceSelectionMode::FormulaMetadata
                                        && engine.resolve_reference_value_expr(scoped, arg).is_ok())
                                {
                                    engine.collect_reference_selection_inputs(
                                        scoped,
                                        mode,
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
                            },
                        );
                        return;
                    }
                    if mode == ReferenceSelectionMode::FormulaMetadata
                        && function_evaluator(name)
                            == Some(Evaluator::Legacy(LegacyFunction::Index))
                    {
                        if let Some((source, selectors)) = args.split_first() {
                            self.collect_reference_selection_inputs(
                                context,
                                mode,
                                source,
                                visited,
                                local_names,
                                output,
                            );
                            for selector in selectors {
                                self.collect_dependency_targets(
                                    context,
                                    selector,
                                    visited,
                                    local_names,
                                    output,
                                );
                            }
                        }
                        return;
                    }
                    if mode == ReferenceSelectionMode::FormulaMetadata
                        && function_dependency_kind(name)
                            == Some(DependencyKind::DynamicReference(
                                DynamicReferenceKind::Offset,
                            ))
                    {
                        if let Some((base, selectors)) = args.split_first() {
                            self.collect_reference_selection_inputs(
                                context,
                                mode,
                                base,
                                visited,
                                local_names,
                                output,
                            );
                            for selector in selectors {
                                self.collect_dependency_targets(
                                    context,
                                    selector,
                                    visited,
                                    local_names,
                                    output,
                                );
                            }
                        }
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
                    if self.builtin_invocation_callee_is_reachable(context, callee, local_names) {
                        self.collect_reference_selection_inputs(
                            context,
                            mode,
                            callee,
                            visited,
                            local_names,
                            output,
                        );
                    }
                    if !self.builtin_invocation_arguments_are_reachable(
                        context,
                        callee,
                        args,
                        local_names,
                    ) {
                        return;
                    }
                    for arg in args {
                        self.collect_reference_selection_inputs(
                            context,
                            mode,
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
        })
    }

    /// Reports whether `expr` holds a reference whose cells depend on values.
    ///
    /// With `bounded_range_ends`, a range with an end such as INDEX also counts when a formula or
    /// an array result lies in its reach: calculation order follows only the cells the range
    /// covers when its dependencies are collected, so those cells may be read out of order once
    /// the end moves. Literal cells in the reach are safe to read in any order.
    fn expr_contains_dynamic_reference_function(
        &self,
        bounded_range_ends: bool,
        context: EvalContext<'_>,
        expr: &Expr,
        visited: &mut VisitedDefinitions,
        local_names: &mut Vec<String>,
    ) -> bool {
        let context = context.enter(expr);
        crate::calculation::stack::grow(|| {
            if context.is_cancelled() {
                return true;
            }
            match expr {
                Expr::Call { name, args } => {
                    if let Some((shadow, arguments_are_reachable)) = self
                        .shadowed_call_arguments_are_reachable(
                            context,
                            name,
                            args.len(),
                            local_names,
                        )
                    {
                        let mut found = false;
                        if shadow != CallableShadow::CyclicNonCallable
                            && let Some((id, named)) =
                                self.resolve_name_expr_with_id_in_context(context, name)
                            && visited.values.insert(id.clone())
                        {
                            found |= self.expr_contains_dynamic_reference_function(
                                bounded_range_ends,
                                context
                                    .without_bindings()
                                    .with_defined_name_scope(Some(id.scope())),
                                named,
                                visited,
                                &mut Vec::new(),
                            );
                        }
                        return found
                            || (arguments_are_reachable
                                && args.iter().any(|arg| {
                                    self.expr_contains_dynamic_reference_function(
                                        bounded_range_ends,
                                        context,
                                        arg,
                                        visited,
                                        local_names,
                                    )
                                }));
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
                            found |= self.expr_contains_dynamic_reference_function(
                                bounded_range_ends,
                                context,
                                arg,
                                visited,
                                scope,
                            );
                        },
                    ) {
                        return found;
                    }
                    args.iter().any(|arg| {
                        self.expr_contains_dynamic_reference_function(
                            bounded_range_ends,
                            context,
                            arg,
                            visited,
                            local_names,
                        )
                    })
                }
                Expr::Invoke { callee, args } => {
                    let callee_contains_dynamic =
                        self.builtin_invocation_callee_is_reachable(context, callee, local_names)
                            && self.expr_contains_dynamic_reference_function(
                                bounded_range_ends,
                                context,
                                callee,
                                visited,
                                local_names,
                            );
                    callee_contains_dynamic
                        || (self.builtin_invocation_arguments_are_reachable(
                            context,
                            callee,
                            args,
                            local_names,
                        ) && args.iter().any(|arg| {
                            self.expr_contains_dynamic_reference_function(
                                bounded_range_ends,
                                context,
                                arg,
                                visited,
                                local_names,
                            )
                        }))
                }
                Expr::Name(name) => {
                    let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    else {
                        return false;
                    };
                    self.expr_contains_dynamic_reference_function(
                        bounded_range_ends,
                        defined_context,
                        named,
                        visited,
                        &mut Vec::new(),
                    )
                }
                Expr::BuiltinCallable(callable) => {
                    let name = callable.canonical_name();
                    let Some((defined_context, named)) =
                        self.reachable_defined_name(context, name, visited, local_names)
                    else {
                        return false;
                    };
                    self.expr_contains_dynamic_reference_function(
                        bounded_range_ends,
                        defined_context,
                        named,
                        visited,
                        &mut Vec::new(),
                    )
                }
                Expr::Range { .. }
                    if match self.range_extent(context, expr, local_names, &mut Vec::new()) {
                        RangeExtent::Bounded(rect) => {
                            bounded_range_ends && self.rect_holds_calculated_cells(&rect)
                        }
                        RangeExtent::ValueDependent => true,
                        RangeExtent::Fixed(_) | RangeExtent::Invalid => false,
                    } =>
                {
                    true
                }
                Expr::ImplicitIntersection(inner)
                | Expr::SpillRef(inner)
                | Expr::Paren(inner)
                | Expr::Unary { operand: inner, .. } => self
                    .expr_contains_dynamic_reference_function(
                        bounded_range_ends,
                        context,
                        inner,
                        visited,
                        local_names,
                    ),
                Expr::Binary { left, right, .. }
                | Expr::ReferenceUnion { left, right }
                | Expr::ReferenceIntersection { left, right }
                | Expr::Range {
                    start: left,
                    end: right,
                } => {
                    self.expr_contains_dynamic_reference_function(
                        bounded_range_ends,
                        context,
                        left,
                        visited,
                        local_names,
                    ) || self.expr_contains_dynamic_reference_function(
                        bounded_range_ends,
                        context,
                        right,
                        visited,
                        local_names,
                    )
                }
                Expr::Array(rows) => rows.iter().flatten().any(|element| {
                    self.expr_contains_dynamic_reference_function(
                        bounded_range_ends,
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
                | Expr::Missing => false,
                Expr::QualifiedName { sheet, name } => self
                    .reachable_qualified_name(context, sheet, name, visited)
                    .is_some_and(|(defined_context, named)| {
                        self.expr_contains_dynamic_reference_function(
                            bounded_range_ends,
                            defined_context,
                            named,
                            visited,
                            &mut Vec::new(),
                        )
                    }),
            }
        })
    }
}

pub(super) fn rects_intersect(left: &Rect, right: &Rect) -> bool {
    left.sheet == right.sheet
        && left.row_start <= right.row_end
        && right.row_start <= left.row_end
        && left.col_start <= right.col_end
        && right.col_start <= left.col_end
}

#[cfg(test)]
#[path = "dependency_tests.rs"]
mod tests;
