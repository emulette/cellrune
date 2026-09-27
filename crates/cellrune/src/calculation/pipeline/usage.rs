use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod errors;

use super::{
    CapabilityScope, NameScanContext, call_shadow, call_shadow_arguments_are_reachable,
    dynamic_function, typed_invocation_arguments_are_reachable, typed_invocation_shadow,
};
use crate::calculation::ast::Expr;
use crate::calculation::eval::Engine;
use crate::calculation::functions::{
    CallableShadow, DynamicFunction, function_arguments_are_reachable, is_supported_function,
    normalize_name,
};
use crate::calculation::lambda::definition;
use crate::calculation::scope::DefinedLambdaId;
use crate::calculation::{
    CalculationCellId, CalculationOptions, FunctionSupport, FunctionUsageEntry, FunctionUsageReport,
};
use crate::{CellContent, WorkbookSnapshot};

const MAX_SAMPLES: usize = 8;
type Counts = BTreeMap<String, u64>;
// Definitions reset local bindings and resolve nested names in their own scope. The caller
// sheet remains part of the context, including for workbook-scoped definitions.
type NameKey = (usize, DefinedLambdaId);
type Memo = BTreeMap<NameKey, Arc<Counts>>;

#[derive(Default)]
struct Accumulator {
    calls: u64,
    formulas: u64,
    samples: Vec<CalculationCellId>,
}

pub(in crate::calculation) fn scan_function_usage(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> FunctionUsageReport {
    let engine = Engine::analyze(workbook, options);
    let mut formula_count = 0;
    let mut parsed_count = 0;
    let mut usage = BTreeMap::<String, Accumulator>::new();
    let mut memo = Memo::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            if !matches!(cell.content(), CellContent::Formula(_)) {
                continue;
            }
            formula_count += 1;
            let address = cell.address();
            let Some(expr) =
                engine.parsed_expr((sheet_index, address.row().get(), address.column().get()))
            else {
                continue;
            };
            parsed_count += 1;
            let id = CalculationCellId::new(sheet.id(), address);
            for (name, count) in collect(&engine, sheet_index, expr, &mut memo) {
                let entry = usage.entry(name).or_default();
                entry.calls = entry.calls.saturating_add(count);
                entry.formulas += 1;
                if entry.samples.len() < MAX_SAMPLES {
                    entry.samples.push(id);
                }
            }
        }
    }
    let entries = usage
        .into_iter()
        .map(|(name, entry)| {
            let support = if is_supported_function(&name) {
                FunctionSupport::Supported
            } else {
                FunctionSupport::Unsupported
            };
            FunctionUsageEntry::new(name, support, entry.calls, entry.formulas, entry.samples)
        })
        .collect();
    FunctionUsageReport::new(entries, formula_count, parsed_count)
}

enum Step<'a> {
    Expression(&'a Expr),
    Bind(&'a str, &'a Expr),
    RestoreScope(usize),
}

struct Frame<'a> {
    context: NameScanContext,
    scope: CapabilityScope,
    pending: Vec<Step<'a>>,
    counts: Counts,
    key: Option<NameKey>,
    cacheable: bool,
}

impl<'a> Frame<'a> {
    fn new(context: NameScanContext, expr: &'a Expr, key: Option<NameKey>) -> Self {
        Self {
            context,
            scope: CapabilityScope::default(),
            pending: vec![Step::Expression(expr)],
            counts: Counts::new(),
            key,
            cacheable: true,
        }
    }

    fn add_call(&mut self, name: String) {
        let count = self.counts.entry(name).or_default();
        *count = count.saturating_add(1);
    }

    fn push_arguments(&mut self, args: &'a [Expr]) {
        self.pending.extend(args.iter().rev().map(Step::Expression));
    }

    fn resolve(&self, engine: &'a Engine<'_>, name: &str) -> Option<(DefinedLambdaId, &'a Expr)> {
        engine.resolve_name_expr_with_id_for_scope(
            self.context.sheet,
            self.context.defined_name_scope,
            name,
        )
    }

    fn visit(
        &mut self,
        engine: &'a Engine<'_>,
        expr: &'a Expr,
    ) -> Option<(DefinedLambdaId, &'a Expr)> {
        match expr {
            Expr::Call { name, args } => {
                if let Some(reachable) = call_shadow_arguments_are_reachable(
                    engine,
                    self.context,
                    name,
                    args,
                    &self.scope,
                ) {
                    if reachable {
                        self.push_arguments(args);
                    }
                    if call_shadow(engine, self.context, name, &self.scope)
                        != Some(CallableShadow::CyclicNonCallable)
                    {
                        return self.resolve(engine, name);
                    }
                    return None;
                }
                self.add_call(normalize_name(name));
                if is_supported_function(name)
                    && !function_arguments_are_reachable(
                        name,
                        args,
                        engine.calculation_limits().max_let_bindings(),
                    )
                {
                    return None;
                }
                match dynamic_function(name) {
                    Some(DynamicFunction::Let) => {
                        let previous = self.scope.len();
                        self.pending.push(Step::RestoreScope(previous));
                        if let Some((last, pairs)) = args.split_last() {
                            self.pending.push(Step::Expression(last));
                            for pair in pairs.chunks_exact(2).rev() {
                                if let Expr::Name(binding) = &pair[0] {
                                    self.pending.push(Step::Bind(binding, &pair[1]));
                                }
                                self.pending.push(Step::Expression(&pair[1]));
                            }
                        }
                    }
                    Some(DynamicFunction::Lambda) => {
                        if let Some(lambda) = definition(expr) {
                            self.pending.push(Step::RestoreScope(self.scope.len()));
                            for parameter in lambda.parameters() {
                                self.scope.push_parameter(parameter.clone());
                            }
                            self.pending.push(Step::Expression(lambda.body()));
                        }
                    }
                    _ => self.push_arguments(args),
                }
            }
            Expr::Invoke { callee, args } => {
                if typed_invocation_arguments_are_reachable(
                    engine,
                    self.context,
                    callee,
                    args,
                    &self.scope,
                ) {
                    self.push_arguments(args);
                }
                let cyclic = typed_invocation_shadow(engine, self.context, callee, &self.scope)
                    .is_some_and(|(_, shadow)| shadow == CallableShadow::CyclicNonCallable);
                if !cyclic {
                    self.pending.push(Step::Expression(callee));
                }
            }
            Expr::ImplicitIntersection(inner)
            | Expr::SpillRef(inner)
            | Expr::Paren(inner)
            | Expr::Unary { operand: inner, .. } => self.pending.push(Step::Expression(inner)),
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                self.pending.push(Step::Expression(right));
                self.pending.push(Step::Expression(left));
            }
            Expr::Array(rows) => {
                self.pending.extend(
                    rows.iter()
                        .rev()
                        .flat_map(|row| row.iter().rev())
                        .map(Step::Expression),
                );
            }
            Expr::Name(name) => {
                if self.scope.lookup(name).is_none() {
                    return self.resolve(engine, name);
                }
            }
            Expr::BuiltinCallable(callable) => {
                let name = callable.canonical_name();
                if self.scope.lookup(name).is_none() {
                    if let Some(named) = self.resolve(engine, name) {
                        return Some(named);
                    }
                    self.add_call(name.to_owned());
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
        None
    }
}

fn merge(target: &mut Counts, source: &Counts) {
    for (name, count) in source {
        let total = target.entry(name.clone()).or_default();
        *total = total.saturating_add(*count);
    }
}

fn collect<'a>(engine: &'a Engine<'_>, sheet: usize, expr: &'a Expr, memo: &mut Memo) -> Counts {
    let mut frames = vec![Frame::new(NameScanContext::root(sheet), expr, None)];
    let mut active = BTreeSet::new();
    loop {
        let frame = frames.last_mut().expect(errors::ROOT_FRAME);
        let Some(step) = frame.pending.pop() else {
            let frame = frames.pop().expect(errors::COMPLETED_FRAME);
            let Some(key) = frame.key else {
                return frame.counts;
            };
            active.remove(&key.1);
            let counts = Arc::new(frame.counts);
            if frame.cacheable {
                memo.insert(key, Arc::clone(&counts));
            }
            merge(
                &mut frames.last_mut().expect(errors::PARENT_FRAME).counts,
                &counts,
            );
            continue;
        };
        let named = match step {
            Step::Expression(expr) => frame.visit(engine, expr),
            Step::Bind(name, expr) => {
                frame
                    .scope
                    .push_expression(engine, frame.context, name, expr);
                None
            }
            Step::RestoreScope(len) => {
                frame.scope.truncate(len);
                None
            }
        };
        let Some((id, expr)) = named else {
            continue;
        };
        if active.contains(&id) {
            // A cut path depends on its active ancestors. None of those unfinished summaries
            // is reusable from another entry point, even if it has already counted some calls.
            for frame in &mut frames {
                frame.cacheable = false;
            }
            continue;
        }
        let context = frame.context.for_definition(id.scope());
        let key = (context.sheet, id.clone());
        if let Some(counts) = memo.get(&key) {
            merge(&mut frame.counts, counts);
        } else {
            active.insert(id);
            frames.push(Frame::new(context, expr, Some(key)));
        }
    }
}
