use std::collections::{BTreeMap, BTreeSet};

use super::{Engine, EvalContext};
use crate::calculation::ast::Expr;
use crate::calculation::functions::{
    CallableShadow, DynamicFunction, Evaluator, builtin_invocation_arguments_are_reachable,
    callable_shadow_arguments_are_reachable, classify_callable_value, direct_builtin_callable,
    function_arguments_are_reachable, function_evaluator,
};
use crate::calculation::lambda::definition;
use crate::calculation::runtime::CellId;
use crate::calculation::scope::{
    DefinedLambdaId, canonical_local_name, resolve_defined_name_scoped,
};
use crate::{DefinedName, DefinedNameScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameGraphStatus {
    Supported,
    Cycle,
    LimitExceeded,
}

fn dynamic_function(name: &str) -> Option<DynamicFunction> {
    match function_evaluator(name) {
        Some(Evaluator::Dynamic(function)) => Some(function),
        _ => None,
    }
}

impl Engine<'_> {
    pub(in crate::calculation) fn callable_shadow_for_name(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
    ) -> CallableShadow {
        self.callable_shadow_for_name_inner(sheet, lookup_scope, name, &mut BTreeSet::new())
    }

    fn callable_shadow_for_name_inner(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
        active: &mut BTreeSet<DefinedLambdaId>,
    ) -> CallableShadow {
        let Some((index, defined_name)) =
            self.resolve_defined_name_scoped(sheet, lookup_scope, name)
        else {
            return CallableShadow::Unshadowed;
        };
        let id = DefinedLambdaId::from_defined_name(defined_name);
        if !active.insert(id.clone()) {
            return CallableShadow::CyclicNonCallable;
        }
        let state = self
            .defined_name_asts
            .get(index)
            .and_then(Option::as_ref)
            .map_or(CallableShadow::Unknown, |parsed| {
                let mut resolve =
                    |nested: &str| -> Result<CallableShadow, std::convert::Infallible> {
                        Ok(self.callable_shadow_for_name_inner(
                            sheet,
                            Some(id.scope()),
                            nested,
                            active,
                        ))
                    };
                classify_callable_value(
                    parsed.root(),
                    &[],
                    self.calculation_limits().max_let_bindings(),
                    &mut resolve,
                )
                .expect("static callable classification is infallible")
            });
        active.remove(&id);
        state
    }

    pub(super) fn classify_name_graphs(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ()> {
        for (cell, parsed) in self.asts.iter() {
            if cancelled() {
                return Err(());
            }
            match self.inspect_name_graph(cell.0, parsed.root(), cancelled)? {
                NameGraphStatus::Supported => {}
                NameGraphStatus::Cycle => {
                    std::sync::Arc::make_mut(&mut self.name_cycle_cells).insert(*cell);
                }
                NameGraphStatus::LimitExceeded => {
                    std::sync::Arc::make_mut(&mut self.name_limit_cells).insert(*cell);
                }
            }
        }
        Ok(())
    }

    fn inspect_name_graph(
        &self,
        sheet: usize,
        root: &Expr,
        cancelled: &impl Fn() -> bool,
    ) -> Result<NameGraphStatus, ()> {
        let mut pending: Vec<(String, bool, u64, Option<DefinedNameScope>)> = self
            .name_references_for_scope(sheet, None, root, cancelled)?
            .into_iter()
            .map(|name| (name, false, 1, None))
            .collect();
        let mut active_values = BTreeSet::<DefinedLambdaId>::new();
        let mut active_callables = BTreeSet::<DefinedLambdaId>::new();
        let mut value_depths = BTreeMap::<DefinedLambdaId, u64>::new();
        let mut callable_depths = BTreeMap::<DefinedLambdaId, u64>::new();
        while let Some((name, expanded, depth, lookup_scope)) = pending.pop() {
            if cancelled() {
                return Err(());
            }
            let Some((defined_name_index, defined_name)) =
                self.resolve_defined_name_scoped(sheet, lookup_scope, &name)
            else {
                continue;
            };
            let key = DefinedLambdaId::from_defined_name(defined_name);
            let Some(expr) = self.defined_name_asts[defined_name_index].as_ref() else {
                continue;
            };
            if definition(expr.root()).is_some() {
                if expanded {
                    active_callables.remove(&key);
                    continue;
                }
                // Callable recursion is legal and is bounded by the runtime lambda budget.
                // Only a back-edge on the current expansion path is skipped here.
                if active_callables.contains(&key)
                    || callable_depths.get(&key).is_some_and(|seen| *seen >= depth)
                {
                    continue;
                }
                if depth > self.options.limits().max_formula_nesting_depth() {
                    return Ok(NameGraphStatus::LimitExceeded);
                }
                callable_depths.insert(key.clone(), depth);
                active_callables.insert(key);
                pending.push((name, true, depth, lookup_scope));
                pending.extend(
                    self.name_references_for_scope(
                        sheet,
                        Some(defined_name.scope()),
                        expr.root(),
                        cancelled,
                    )?
                    .into_iter()
                    .map(|child| (child, false, depth + 1, Some(defined_name.scope()))),
                );
                continue;
            }
            if expanded {
                active_values.remove(&key);
                continue;
            }
            if active_values.contains(&key) {
                return Ok(NameGraphStatus::Cycle);
            }
            if value_depths.get(&key).is_some_and(|seen| *seen >= depth) {
                continue;
            }
            if depth > self.options.limits().max_formula_nesting_depth() {
                return Ok(NameGraphStatus::LimitExceeded);
            }
            value_depths.insert(key.clone(), depth);
            active_values.insert(key);
            pending.push((name, true, depth, lookup_scope));
            pending.extend(
                self.name_references_for_scope(
                    sheet,
                    Some(defined_name.scope()),
                    expr.root(),
                    cancelled,
                )?
                .into_iter()
                .map(|child| (child, false, depth + 1, Some(defined_name.scope()))),
            );
        }
        Ok(NameGraphStatus::Supported)
    }

    fn name_references_for_scope(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        expr: &Expr,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<String>, ()> {
        let mut names = Vec::new();
        collect_name_references(
            self,
            sheet,
            lookup_scope,
            expr,
            &mut Vec::new(),
            &mut names,
            cancelled,
        )?;
        Ok(names)
    }

    pub(in crate::calculation) fn has_name_cycle(&self, cell: CellId) -> bool {
        self.name_cycle_cells.contains(&cell)
    }

    pub(in crate::calculation) fn has_name_limit(&self, cell: CellId) -> bool {
        self.name_limit_cells.contains(&cell)
    }

    pub(in crate::calculation) fn resolve_name_expr_with_id_in_context(
        &self,
        context: EvalContext<'_>,
        name: &str,
    ) -> Option<(DefinedLambdaId, &Expr)> {
        self.resolve_name_expr_with_id_for_scope(
            context.sheet(),
            context.defined_name_scope(),
            name,
        )
    }

    pub(in crate::calculation) fn resolve_name_expr_with_id_for_scope(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
    ) -> Option<(DefinedLambdaId, &Expr)> {
        let (index, defined_name) = self.resolve_defined_name_scoped(sheet, lookup_scope, name)?;
        let id = DefinedLambdaId::from_defined_name(defined_name);
        let expr = self.defined_name_asts.get(index)?.as_ref()?;
        Some((id, expr.root()))
    }

    fn resolve_defined_name_scoped(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
    ) -> Option<(usize, &DefinedName)> {
        resolve_defined_name_scoped(
            self.workbook,
            self.workbook.sheets().get(sheet).map(|sheet| sheet.id()),
            lookup_scope,
            name,
        )
    }
}

fn collect_name_references(
    engine: &Engine<'_>,
    sheet: usize,
    lookup_scope: Option<DefinedNameScope>,
    expr: &Expr,
    local_names: &mut Vec<LocalNameEntry>,
    names: &mut Vec<String>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), ()> {
    if cancelled() {
        return Err(());
    }
    match expr {
        Expr::Name(name) => {
            if local_name_entry(name, local_names).is_none() {
                names.push(name.clone());
            }
        }
        Expr::BuiltinCallable(callable) => {
            let name = callable.canonical_name();
            if local_name_entry(name, local_names).is_none()
                && engine
                    .resolve_defined_name_scoped(sheet, lookup_scope, name)
                    .is_some()
            {
                names.push(name.to_owned());
            }
        }
        Expr::Call { name, args } => {
            if let Some(local) = local_name_entry(name, local_names) {
                if callable_shadow_arguments_are_reachable(local.callable_shadow, None, args.len())
                {
                    for arg in args {
                        collect_name_references(
                            engine,
                            sheet,
                            lookup_scope,
                            arg,
                            local_names,
                            names,
                            cancelled,
                        )?;
                    }
                }
                return Ok(());
            }
            if engine
                .resolve_defined_name_scoped(sheet, lookup_scope, name)
                .is_some()
            {
                let shadow = engine.callable_shadow_for_name(sheet, lookup_scope, name);
                if shadow != CallableShadow::CyclicNonCallable {
                    names.push(name.clone());
                }
                if callable_shadow_arguments_are_reachable(shadow, None, args.len()) {
                    for arg in args {
                        collect_name_references(
                            engine,
                            sheet,
                            lookup_scope,
                            arg,
                            local_names,
                            names,
                            cancelled,
                        )?;
                    }
                }
                return Ok(());
            }
            if function_evaluator(name).is_some()
                && !function_arguments_are_reachable(
                    name,
                    args,
                    engine.calculation_limits().max_let_bindings(),
                )
            {
                return Ok(());
            }
            match dynamic_function(name) {
                Some(DynamicFunction::Let) => {
                    let previous_len = local_names.len();
                    if let Some((final_expr, pairs)) = args.split_last() {
                        for pair in pairs.chunks_exact(2) {
                            collect_name_references(
                                engine,
                                sheet,
                                lookup_scope,
                                &pair[1],
                                local_names,
                                names,
                                cancelled,
                            )?;
                            if let Expr::Name(binding_name) = &pair[0] {
                                local_names.push(LocalNameEntry {
                                    name: canonical_local_name(binding_name),
                                    callable_shadow: classify_local_callable_value(
                                        engine,
                                        sheet,
                                        lookup_scope,
                                        &pair[1],
                                        local_names,
                                    ),
                                });
                            }
                        }
                        collect_name_references(
                            engine,
                            sheet,
                            lookup_scope,
                            final_expr,
                            local_names,
                            names,
                            cancelled,
                        )?;
                    }
                    local_names.truncate(previous_len);
                    return Ok(());
                }
                Some(DynamicFunction::Lambda) => {
                    let Some(lambda) = definition(expr) else {
                        return Ok(());
                    };
                    let previous_len = local_names.len();
                    local_names.extend(lambda.parameters().iter().map(|parameter| {
                        LocalNameEntry {
                            name: parameter.clone(),
                            callable_shadow: CallableShadow::Unknown,
                        }
                    }));
                    collect_name_references(
                        engine,
                        sheet,
                        lookup_scope,
                        lambda.body(),
                        local_names,
                        names,
                        cancelled,
                    )?;
                    local_names.truncate(previous_len);
                    return Ok(());
                }
                Some(DynamicFunction::Map) => {
                    let Some((lambda_expr, array_exprs)) = args.split_last() else {
                        return Ok(());
                    };
                    let Some(lambda) = definition(lambda_expr) else {
                        return Ok(());
                    };
                    for arg in array_exprs {
                        collect_name_references(
                            engine,
                            sheet,
                            lookup_scope,
                            arg,
                            local_names,
                            names,
                            cancelled,
                        )?;
                    }
                    let previous_len = local_names.len();
                    local_names.extend(lambda.parameters().iter().map(|parameter| {
                        LocalNameEntry {
                            name: parameter.clone(),
                            callable_shadow: CallableShadow::DefinitelyNonCallable,
                        }
                    }));
                    collect_name_references(
                        engine,
                        sheet,
                        lookup_scope,
                        lambda.body(),
                        local_names,
                        names,
                        cancelled,
                    )?;
                    local_names.truncate(previous_len);
                    return Ok(());
                }
                _ => {}
            }
            for arg in args {
                collect_name_references(
                    engine,
                    sheet,
                    lookup_scope,
                    arg,
                    local_names,
                    names,
                    cancelled,
                )?;
            }
        }
        Expr::Invoke { callee, args } => {
            let cyclic_callee = direct_builtin_callable(callee).is_some_and(|callable| {
                let shadow = local_name_entry(callable.canonical_name(), local_names).map_or_else(
                    || {
                        engine.callable_shadow_for_name(
                            sheet,
                            lookup_scope,
                            callable.canonical_name(),
                        )
                    },
                    |local| local.callable_shadow,
                );
                shadow == CallableShadow::CyclicNonCallable
            });
            if !cyclic_callee {
                collect_name_references(
                    engine,
                    sheet,
                    lookup_scope,
                    callee,
                    local_names,
                    names,
                    cancelled,
                )?;
            }
            if !builtin_invocation_arguments_are_reachable(callee, args, |name| {
                if let Some(local) = local_name_entry(name, local_names) {
                    return local.callable_shadow;
                }
                engine.callable_shadow_for_name(sheet, lookup_scope, name)
            }) {
                return Ok(());
            }
            for arg in args {
                collect_name_references(
                    engine,
                    sheet,
                    lookup_scope,
                    arg,
                    local_names,
                    names,
                    cancelled,
                )?;
            }
        }
        Expr::ImplicitIntersection(inner)
        | Expr::SpillRef(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => {
            collect_name_references(
                engine,
                sheet,
                lookup_scope,
                inner,
                local_names,
                names,
                cancelled,
            )?;
        }
        Expr::Binary { left, right, .. }
        | Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right } => {
            collect_name_references(
                engine,
                sheet,
                lookup_scope,
                left,
                local_names,
                names,
                cancelled,
            )?;
            collect_name_references(
                engine,
                sheet,
                lookup_scope,
                right,
                local_names,
                names,
                cancelled,
            )?;
        }
        Expr::Range { start, end } => {
            collect_name_references(
                engine,
                sheet,
                lookup_scope,
                start,
                local_names,
                names,
                cancelled,
            )?;
            collect_name_references(
                engine,
                sheet,
                lookup_scope,
                end,
                local_names,
                names,
                cancelled,
            )?;
        }
        Expr::Array(rows) => {
            for element in rows.iter().flatten() {
                collect_name_references(
                    engine,
                    sheet,
                    lookup_scope,
                    element,
                    local_names,
                    names,
                    cancelled,
                )?;
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Missing => {}
    }
    Ok(())
}

#[derive(Debug)]
struct LocalNameEntry {
    name: String,
    callable_shadow: CallableShadow,
}

fn classify_local_callable_value(
    engine: &Engine<'_>,
    sheet: usize,
    lookup_scope: Option<DefinedNameScope>,
    expr: &Expr,
    local_names: &[LocalNameEntry],
) -> CallableShadow {
    let locals = local_names
        .iter()
        .map(|local| (local.name.clone(), local.callable_shadow))
        .collect::<Vec<_>>();
    let mut resolve = |name: &str| -> Result<CallableShadow, std::convert::Infallible> {
        Ok(engine.callable_shadow_for_name(sheet, lookup_scope, name))
    };
    classify_callable_value(
        expr,
        &locals,
        engine.calculation_limits().max_let_bindings(),
        &mut resolve,
    )
    .expect("static callable classification is infallible")
}

fn local_name_entry<'scope>(
    name: &str,
    scope: &'scope [LocalNameEntry],
) -> Option<&'scope LocalNameEntry> {
    let key = canonical_local_name(name);
    scope.iter().rev().find(|entry| entry.name == key)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::Engine;
    use crate::{
        CalculationOptions, CellAddress, DefinedName, DefinedNameScope, FormulaText, SheetId,
        WorkbookDraft,
    };

    #[test]
    fn name_graph_expansion_polls_cancellation_between_nodes() {
        let mut draft = WorkbookDraft::new();
        draft
            .set_defined_name(
                DefinedName::new(
                    "Reader",
                    DefinedNameScope::Workbook,
                    formula("Target"),
                    false,
                )
                .expect("defined name"),
            )
            .expect("defined name edit");
        draft
            .set_defined_name(
                DefinedName::new("Target", DefinedNameScope::Workbook, formula("1"), false)
                    .expect("defined name"),
            )
            .expect("defined name edit");
        let sheet = SheetId::new(1).expect("default sheet ID");
        draft
            .set_cell_formula(sheet, address("A1"), formula("Reader"))
            .expect("formula");
        let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
        let root = engine.parsed_expr((0, 1, 1)).expect("parsed formula");
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        };

        assert_eq!(engine.inspect_name_graph(0, root, &cancelled), Err(()));
        assert!(polls.get() >= 2);
    }

    fn address(value: &str) -> CellAddress {
        CellAddress::from_a1(value).expect("valid test address")
    }

    fn formula(value: &str) -> FormulaText {
        FormulaText::from_xlsx(value).expect("valid test formula")
    }
}
