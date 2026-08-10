use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::ast::{Expr, SheetPrefix};
use super::convert::cell_from_value;
use super::error::parse_error_detail;
use super::eval::{CompiledWorkbook, Engine, public_to_internal};
use super::functions::descriptor::{DependencyKind, ReferenceMetadataKind, Volatility};
use super::functions::{
    CallableShadow, DynamicFunction, Evaluator, builtin_invocation_arguments_are_reachable,
    callable_shadow_arguments_are_reachable, classify_callable_value, descriptor_sheet_span_policy,
    direct_builtin_callable, function_argument_is_callable, function_arguments_are_reachable,
    function_dependency_kind, function_evaluator, function_volatility, is_supported_function,
    normalize_name,
};
use super::lambda::definition;
use super::scope::DefinedLambdaId;
use super::scope::canonical_local_name;
use super::sheet_span::{ARRAY_EXPRESSION_POLICY, SheetSpanPolicy};
use super::value::{ErrorKind, Value};
use super::{
    CalculationCellId, CalculationCellResult, CalculationIssue, CalculationIssueCode,
    CalculationLimitKind, CalculationOptions, CalculationSnapshot, FormulaCapability,
    FormulaCapabilityEntry, FormulaCapabilityReport, FunctionSupport, FunctionUsageEntry,
    FunctionUsageReport, MaterializedCalculationCell, MaterializedResultOrigin,
};
use crate::{CellAddress, CellContent, DefinedNameScope, FormulaMetadata, WorkbookSnapshot};

#[derive(Clone, Copy)]
struct NameScanContext {
    sheet: usize,
    defined_name_scope: Option<DefinedNameScope>,
}

fn dynamic_function(name: &str) -> Option<DynamicFunction> {
    match function_evaluator(name) {
        Some(Evaluator::Dynamic(function)) => Some(function),
        _ => None,
    }
}

impl NameScanContext {
    const fn root(sheet: usize) -> Self {
        Self {
            sheet,
            defined_name_scope: None,
        }
    }

    const fn for_definition(self, defined_name_scope: DefinedNameScope) -> Self {
        Self {
            defined_name_scope: Some(defined_name_scope),
            ..self
        }
    }
}

fn typed_invocation_arguments_are_reachable(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    callee: &Expr,
    args: &[Expr],
    local_scope: &CapabilityScope,
) -> bool {
    builtin_invocation_arguments_are_reachable(callee, args, |name| {
        if let Some(shadow) = local_scope.callable_shadow(name) {
            return shadow;
        }
        engine.callable_shadow_for_name(sheet.sheet, sheet.defined_name_scope, name)
    })
}

fn typed_invocation_shadow(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    callee: &Expr,
    local_scope: &CapabilityScope,
) -> Option<(super::functions::BuiltinCallable, CallableShadow)> {
    let callable = direct_builtin_callable(callee)?;
    let name = callable.canonical_name();
    let shadow = local_scope.callable_shadow(name).unwrap_or_else(|| {
        engine.callable_shadow_for_name(sheet.sheet, sheet.defined_name_scope, name)
    });
    Some((callable, shadow))
}

fn call_shadow_arguments_are_reachable(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    name: &str,
    args: &[Expr],
    local_scope: &CapabilityScope,
) -> Option<bool> {
    call_shadow(engine, sheet, name, local_scope)
        .map(|shadow| callable_shadow_arguments_are_reachable(shadow, None, args.len()))
}

fn call_shadow(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    name: &str,
    local_scope: &CapabilityScope,
) -> Option<CallableShadow> {
    local_scope.callable_shadow(name).or_else(|| {
        engine
            .resolve_name_expr_with_id_for_scope(sheet.sheet, sheet.defined_name_scope, name)
            .map(|_| engine.callable_shadow_for_name(sheet.sheet, sheet.defined_name_scope, name))
    })
}

pub(super) fn scan_formula_capabilities(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> FormulaCapabilityReport {
    let engine = Engine::analyze(workbook, options);
    scan_with_engine(workbook, &engine)
}

const MAX_FUNCTION_USAGE_SAMPLES: usize = 8;

#[derive(Default)]
struct FunctionUsageAccumulator {
    call_count: u64,
    formula_count: u64,
    sample_cells: Vec<CalculationCellId>,
}

pub(super) fn scan_function_usage(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> FunctionUsageReport {
    let engine = Engine::analyze(workbook, options);
    let mut formula_count = 0_usize;
    let mut parsed_formula_count = 0_usize;
    let mut usage = BTreeMap::<String, FunctionUsageAccumulator>::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            if !matches!(cell.content(), CellContent::Formula(_)) {
                continue;
            }
            formula_count += 1;
            let internal_id = (
                sheet_index,
                cell.address().row().get(),
                cell.address().column().get(),
            );
            let Some(expr) = engine.parsed_expr(internal_id) else {
                continue;
            };
            parsed_formula_count += 1;
            let public_id = CalculationCellId::new(sheet.id(), cell.address());
            let mut calls = Vec::new();
            collect_function_calls(&engine, sheet_index, expr, &mut calls);
            let unique = calls.iter().cloned().collect::<BTreeSet<_>>();
            for name in calls {
                usage.entry(name).or_default().call_count += 1;
            }
            for name in unique {
                let entry = usage.entry(name).or_default();
                entry.formula_count += 1;
                if entry.sample_cells.len() < MAX_FUNCTION_USAGE_SAMPLES {
                    entry.sample_cells.push(public_id);
                }
            }
        }
    }
    let entries = usage
        .into_iter()
        .map(|(name, usage)| {
            let support = if is_supported_function(&name) {
                FunctionSupport::Supported
            } else {
                FunctionSupport::Unsupported
            };
            FunctionUsageEntry::new(
                name,
                support,
                usage.call_count,
                usage.formula_count,
                usage.sample_cells,
            )
        })
        .collect();
    FunctionUsageReport::new(entries, formula_count, parsed_formula_count)
}

fn collect_function_calls(
    engine: &Engine<'_>,
    sheet: usize,
    expr: &Expr,
    output: &mut Vec<String>,
) {
    collect_function_calls_in_scope(
        engine,
        NameScanContext::root(sheet),
        expr,
        output,
        &mut CapabilityScope::default(),
        &mut BTreeSet::new(),
    );
}

fn collect_function_calls_in_scope(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    expr: &Expr,
    output: &mut Vec<String>,
    local_scope: &mut CapabilityScope,
    active_names: &mut BTreeSet<DefinedLambdaId>,
) {
    match expr {
        Expr::Call { name, args } => {
            let normalized = normalize_name(name);
            if let Some(arguments_are_reachable) =
                call_shadow_arguments_are_reachable(engine, sheet, name, args, local_scope)
            {
                let shadow = call_shadow(engine, sheet, name, local_scope)
                    .expect("shadow reachability implies a shadow state");
                if shadow != CallableShadow::CyclicNonCallable
                    && let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                        sheet.sheet,
                        sheet.defined_name_scope,
                        name,
                    )
                    && active_names.insert(id.clone())
                {
                    collect_function_calls_in_scope(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        output,
                        &mut CapabilityScope::default(),
                        active_names,
                    );
                    active_names.remove(&id);
                }
                if arguments_are_reachable {
                    for arg in args {
                        collect_function_calls_in_scope(
                            engine,
                            sheet,
                            arg,
                            output,
                            local_scope,
                            active_names,
                        );
                    }
                }
                return;
            }
            let arguments_are_reachable = !is_supported_function(name)
                || function_arguments_are_reachable(
                    name,
                    args,
                    engine.calculation_limits().max_let_bindings(),
                );
            if dynamic_function(name) == Some(DynamicFunction::Let) {
                output.push(normalized);
                if !arguments_are_reachable {
                    return;
                }
                let previous_len = local_scope.len();
                if let Some((final_expr, pairs)) = args.split_last() {
                    for pair in pairs.chunks_exact(2) {
                        collect_function_calls_in_scope(
                            engine,
                            sheet,
                            &pair[1],
                            output,
                            local_scope,
                            active_names,
                        );
                        if let Expr::Name(binding_name) = &pair[0] {
                            local_scope.push_expression(engine, sheet, binding_name, &pair[1]);
                        }
                    }
                    collect_function_calls_in_scope(
                        engine,
                        sheet,
                        final_expr,
                        output,
                        local_scope,
                        active_names,
                    );
                }
                local_scope.truncate(previous_len);
                return;
            }
            if dynamic_function(name) == Some(DynamicFunction::Lambda) {
                output.push(normalized);
                if !arguments_are_reachable {
                    return;
                }
                if let Some(lambda) = definition(expr) {
                    let previous_len = local_scope.len();
                    for parameter in lambda.parameters() {
                        local_scope.push_parameter(parameter.clone());
                    }
                    collect_function_calls_in_scope(
                        engine,
                        sheet,
                        lambda.body(),
                        output,
                        local_scope,
                        active_names,
                    );
                    local_scope.truncate(previous_len);
                }
                return;
            }
            output.push(normalized);
            if !arguments_are_reachable {
                return;
            }
            for arg in args {
                collect_function_calls_in_scope(
                    engine,
                    sheet,
                    arg,
                    output,
                    local_scope,
                    active_names,
                );
            }
        }
        Expr::Invoke { callee, args } => {
            let cyclic_callee = typed_invocation_shadow(engine, sheet, callee, local_scope)
                .is_some_and(|(_, shadow)| shadow == CallableShadow::CyclicNonCallable);
            if !cyclic_callee {
                collect_function_calls_in_scope(
                    engine,
                    sheet,
                    callee,
                    output,
                    local_scope,
                    active_names,
                );
            }
            if !typed_invocation_arguments_are_reachable(engine, sheet, callee, args, local_scope) {
                return;
            }
            for arg in args {
                collect_function_calls_in_scope(
                    engine,
                    sheet,
                    arg,
                    output,
                    local_scope,
                    active_names,
                );
            }
        }
        Expr::ImplicitIntersection(inner)
        | Expr::SpillRef(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => {
            collect_function_calls_in_scope(
                engine,
                sheet,
                inner,
                output,
                local_scope,
                active_names,
            );
        }
        Expr::Binary { left, right, .. }
        | Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            collect_function_calls_in_scope(engine, sheet, left, output, local_scope, active_names);
            collect_function_calls_in_scope(
                engine,
                sheet,
                right,
                output,
                local_scope,
                active_names,
            );
        }
        Expr::Array(rows) => {
            for item in rows.iter().flatten() {
                collect_function_calls_in_scope(
                    engine,
                    sheet,
                    item,
                    output,
                    local_scope,
                    active_names,
                );
            }
        }
        Expr::Name(name) => {
            if local_scope.lookup(name).is_some() {
                return;
            }
            if let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                sheet.sheet,
                sheet.defined_name_scope,
                name,
            ) && active_names.insert(id.clone())
            {
                let mut defined_scope = CapabilityScope::default();
                collect_function_calls_in_scope(
                    engine,
                    sheet.for_definition(id.scope()),
                    named,
                    output,
                    &mut defined_scope,
                    active_names,
                );
                active_names.remove(&id);
            }
        }
        Expr::BuiltinCallable(callable) => {
            let name = callable.canonical_name();
            if local_scope.lookup(name).is_some() {
                return;
            }
            if let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                sheet.sheet,
                sheet.defined_name_scope,
                name,
            ) {
                if active_names.insert(id.clone()) {
                    let mut defined_scope = CapabilityScope::default();
                    collect_function_calls_in_scope(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        output,
                        &mut defined_scope,
                        active_names,
                    );
                    active_names.remove(&id);
                }
                return;
            }
            output.push(name.to_owned());
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
}

fn scan_with_engine(workbook: &WorkbookSnapshot, engine: &Engine<'_>) -> FormulaCapabilityReport {
    scan_with_engine_cancellable(workbook, engine, &|| false)
        .expect("non-cancellable capability scan cannot be cancelled")
}

fn scan_with_engine_cancellable(
    workbook: &WorkbookSnapshot,
    engine: &Engine<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<FormulaCapabilityReport, ()> {
    let mut entries = Vec::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let id = CalculationCellId::new(sheet.id(), cell.address());
            let internal_id = (
                sheet_index,
                cell.address().row().get(),
                cell.address().column().get(),
            );
            let mut issues = Vec::new();
            if engine.dependency_limit_exceeded() {
                issues.push(resource_limit_issue(CalculationLimitKind::DependencyEdges));
            }
            let has_name_cycle = engine.has_name_cycle(internal_id);
            let has_name_limit = engine.has_name_limit(internal_id);
            if has_name_cycle {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedName,
                    None,
                ));
            }
            if has_name_limit {
                issues.push(resource_limit_issue(
                    CalculationLimitKind::FormulaNestingDepth,
                ));
            }
            let supported_metadata = match formula.metadata() {
                FormulaMetadata::Normal | FormulaMetadata::Shared { .. } => true,
                FormulaMetadata::Array { .. } => formula
                    .metadata()
                    .legacy_array_range_at(cell.address())
                    .is_some(),
                FormulaMetadata::DynamicArray { .. } => formula
                    .metadata()
                    .dynamic_array_range_at(cell.address())
                    .is_some(),
                FormulaMetadata::DataTable { .. } => false,
            };
            if !supported_metadata {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedExpression,
                    None,
                ));
            }
            match formula.text() {
                None => issues.push(CalculationIssue::new(
                    CalculationIssueCode::MissingFormulaText,
                    None,
                )),
                Some(_) => match engine.parse_failure(internal_id) {
                    Some(error) => match error.limit {
                        Some(limit) => issues.push(resource_limit_issue(limit)),
                        None => issues.push(CalculationIssue::new(
                            CalculationIssueCode::ParseError,
                            Some(parse_error_detail(error)),
                        )),
                    },
                    None if has_name_cycle || has_name_limit => {}
                    None => match engine.parsed_expr(internal_id) {
                        Some(expr) => inspect_expr(
                            engine,
                            NameScanContext::root(sheet_index),
                            expr,
                            CapabilityInspectionPolicy::new(ARRAY_EXPRESSION_POLICY, false),
                            &mut HashSet::new(),
                            &mut CapabilityScope::default(),
                            &mut issues,
                        ),
                        None => issues.push(CalculationIssue::new(
                            CalculationIssueCode::ParseError,
                            None,
                        )),
                    },
                },
            }
            issues.sort_by(|left, right| {
                (left.code(), left.detail()).cmp(&(right.code(), right.detail()))
            });
            issues.dedup();
            let capability = if issues.is_empty() {
                FormulaCapability::Supported
            } else {
                FormulaCapability::Unsupported(issues)
            };
            entries.push(FormulaCapabilityEntry::new(id, capability));
        }
    }
    Ok(FormulaCapabilityReport::new(entries))
}

pub(super) fn calculate_workbook(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> CalculationSnapshot {
    let engine = Engine::evaluate(workbook, options);
    snapshot_from_engine(workbook, options, &engine)
}

pub(super) fn calculate_and_compile(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    cancelled: impl Fn() -> bool,
) -> Result<(CalculationSnapshot, CompiledWorkbook, usize), ()> {
    let engine = Engine::evaluate_cancellable(workbook, options, &cancelled)?;
    let evaluated = engine.evaluated_cell_count();
    let snapshot = snapshot_from_engine_cancellable(workbook, options, &engine, &cancelled)?;
    let compiled = engine.compiled(&cancelled)?;
    Ok((snapshot, compiled, evaluated))
}

pub(super) fn calculate_from_compiled(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    compiled: &CompiledWorkbook,
    previous: Option<&CalculationSnapshot>,
    dirty: Option<&BTreeSet<CalculationCellId>>,
    cancelled: impl Fn() -> bool,
) -> Result<(CalculationSnapshot, usize), ()> {
    let engine =
        Engine::evaluate_compiled(workbook, options, compiled, previous, dirty, &cancelled)?;
    let evaluated = engine.evaluated_cell_count();
    let snapshot = match (previous, dirty) {
        (Some(previous), Some(dirty)) => snapshot_from_incremental_engine_cancellable(
            workbook, options, &engine, previous, dirty, &cancelled,
        )?,
        _ => snapshot_from_engine_cancellable(workbook, options, &engine, &cancelled)?,
    };
    Ok((snapshot, evaluated))
}

fn snapshot_from_incremental_engine_cancellable(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    engine: &Engine<'_>,
    previous: &CalculationSnapshot,
    dirty: &BTreeSet<CalculationCellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<CalculationSnapshot, ()> {
    let mut cells = BTreeMap::new();
    for public_id in dirty {
        if cancelled() {
            return Err(());
        }
        let Some(internal_id) = public_to_internal(workbook, *public_id) else {
            continue;
        };
        let result = if engine.cycle_cells.contains(&internal_id) {
            CalculationCellResult::Unavailable(CalculationIssue::new(
                CalculationIssueCode::CircularReference,
                None,
            ))
        } else if engine.blocked_cells.contains(&internal_id) {
            CalculationCellResult::Unavailable(CalculationIssue::new(
                CalculationIssueCode::BlockedByUpstream,
                None,
            ))
        } else {
            match engine.cell_value(internal_id) {
                Value::Error(ErrorKind::ResourceLimit(limit)) => {
                    CalculationCellResult::Unavailable(resource_limit_issue(limit))
                }
                Value::Error(ErrorKind::Unsupported) => {
                    let missing_volatile_input =
                        engine.parsed_expr(internal_id).is_some_and(|expr| {
                            (options.today_serial().is_none()
                                && contains_volatility(
                                    engine,
                                    internal_id.0,
                                    expr,
                                    Volatility::Today,
                                ))
                                || (options.now_serial().is_none()
                                    && contains_volatility(
                                        engine,
                                        internal_id.0,
                                        expr,
                                        Volatility::Now,
                                    ))
                        });
                    let code = if missing_volatile_input {
                        CalculationIssueCode::VolatileInputMissing
                    } else if engine.has_unavailable_dependency(internal_id, &BTreeSet::new()) {
                        CalculationIssueCode::BlockedByUpstream
                    } else {
                        CalculationIssueCode::UnsupportedExpression
                    };
                    CalculationCellResult::Unavailable(CalculationIssue::new(code, None))
                }
                value => CalculationCellResult::Value(cell_from_value(value)),
            }
        };
        cells.insert(*public_id, result);
    }
    let materialized =
        build_incremental_materialization_cancellable(workbook, engine, &cells, cancelled)?;
    let mut traces = BTreeMap::new();
    for public_id in materialized.keys() {
        if cancelled() {
            return Err(());
        }
        let Some(internal_id) = public_to_internal(workbook, *public_id) else {
            continue;
        };
        if let Some(trace) = engine.calculated_decimal_trace(internal_id) {
            traces.insert(*public_id, trace);
        }
    }
    previous.apply_incremental_patch_cancellable(
        super::IncrementalCalculationPatch::new(
            dirty,
            cells,
            materialized,
            traces,
            workbook,
            options,
        ),
        cancelled,
    )
}

fn build_incremental_materialization_cancellable(
    workbook: &WorkbookSnapshot,
    engine: &Engine<'_>,
    cells: &BTreeMap<CalculationCellId, CalculationCellResult>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<CalculationCellId, MaterializedCalculationCell>, ()> {
    let mut materialized = BTreeMap::new();
    for (anchor, anchor_result) in cells {
        if cancelled() {
            return Err(());
        }
        materialized.insert(
            *anchor,
            MaterializedCalculationCell::new(
                MaterializedResultOrigin::DirectFormula,
                anchor_result.clone(),
            ),
        );
        let Some(internal_anchor) = public_to_internal(workbook, *anchor) else {
            continue;
        };
        let Some(sheet) = workbook.sheets().get(internal_anchor.0) else {
            continue;
        };
        let Some(cell) = sheet.cell(anchor.address()) else {
            continue;
        };
        let CellContent::Formula(formula) = cell.content() else {
            continue;
        };
        let (range, origin) = match formula.metadata() {
            FormulaMetadata::Array { range, .. } if range.start() == anchor.address() => (
                *range,
                MaterializedResultOrigin::LegacyArray {
                    anchor: *anchor,
                    range: *range,
                },
            ),
            FormulaMetadata::DynamicArray { .. } => {
                let Some(resolved) = engine.dynamic_spill(internal_anchor) else {
                    continue;
                };
                let range = crate::CellRange::new(
                    CellAddress::from_indices(resolved.row_start, resolved.col_start)
                        .expect("resolved dynamic start is valid"),
                    CellAddress::from_indices(resolved.row_end, resolved.col_end)
                        .expect("resolved dynamic end is valid"),
                )
                .expect("resolved dynamic range is ordered");
                (
                    range,
                    MaterializedResultOrigin::DynamicSpill {
                        anchor: *anchor,
                        range,
                    },
                )
            }
            FormulaMetadata::Normal
            | FormulaMetadata::Shared { .. }
            | FormulaMetadata::Array { .. }
            | FormulaMetadata::DataTable { .. } => continue,
        };
        for row in range.start().row().get()..=range.end().row().get() {
            for column in range.start().column().get()..=range.end().column().get() {
                if cancelled() {
                    return Err(());
                }
                let address = CellAddress::from_indices(row, column)
                    .expect("validated array range produces valid addresses");
                let id = CalculationCellId::new(sheet.id(), address);
                let result = if id == *anchor {
                    anchor_result.clone()
                } else {
                    match anchor_result {
                        CalculationCellResult::Unavailable(issue) => {
                            CalculationCellResult::Unavailable(issue.clone())
                        }
                        CalculationCellResult::Value(_) => CalculationCellResult::Value(
                            cell_from_value(engine.cell_value((internal_anchor.0, row, column))),
                        ),
                    }
                };
                materialized.insert(id, MaterializedCalculationCell::new(origin, result));
            }
        }
    }
    Ok(materialized)
}

fn snapshot_from_engine(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    engine: &Engine<'_>,
) -> CalculationSnapshot {
    snapshot_from_engine_cancellable(workbook, options, engine, &|| false)
        .expect("non-cancellable snapshot construction cannot be cancelled")
}

fn snapshot_from_engine_cancellable(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    engine: &Engine<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<CalculationSnapshot, ()> {
    let report = scan_with_engine_cancellable(workbook, engine, cancelled)?;
    let mut unsupported = BTreeMap::new();
    for entry in report.entries() {
        if cancelled() {
            return Err(());
        }
        if let FormulaCapability::Unsupported(issues) = entry.capability()
            && let Some(issue) = issues.first()
        {
            unsupported.insert(entry.cell(), issue.clone());
        }
    }
    let mut direct_unavailable = BTreeSet::new();
    for cell in unsupported.keys() {
        if cancelled() {
            return Err(());
        }
        if let Some(cell) = public_to_internal(workbook, *cell) {
            direct_unavailable.insert(cell);
        }
    }
    let mut cells = BTreeMap::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            let CellContent::Formula(_) = cell.content() else {
                continue;
            };
            #[cfg(test)]
            super::work_counter::formula_snapshot_scan();
            let public_id = CalculationCellId::new(sheet.id(), cell.address());
            let internal_id = (
                sheet_index,
                cell.address().row().get(),
                cell.address().column().get(),
            );
            let result = if let Some(retained) = engine.retained_result(internal_id) {
                retained.clone()
            } else if let Some(issue) = unsupported.get(&public_id) {
                CalculationCellResult::Unavailable(issue.clone())
            } else if engine.cycle_cells.contains(&internal_id) {
                CalculationCellResult::Unavailable(CalculationIssue::new(
                    CalculationIssueCode::CircularReference,
                    None,
                ))
            } else if engine.blocked_cells.contains(&internal_id) {
                CalculationCellResult::Unavailable(CalculationIssue::new(
                    CalculationIssueCode::BlockedByUpstream,
                    None,
                ))
            } else {
                match engine.cell_value(internal_id) {
                    Value::Error(ErrorKind::ResourceLimit(limit)) => {
                        CalculationCellResult::Unavailable(resource_limit_issue(limit))
                    }
                    Value::Error(ErrorKind::Unsupported) => {
                        let missing_volatile_input =
                            engine.parsed_expr(internal_id).is_some_and(|expr| {
                                (options.today_serial().is_none()
                                    && contains_volatility(
                                        engine,
                                        sheet_index,
                                        expr,
                                        Volatility::Today,
                                    ))
                                    || (options.now_serial().is_none()
                                        && contains_volatility(
                                            engine,
                                            sheet_index,
                                            expr,
                                            Volatility::Now,
                                        ))
                            });
                        let code = if missing_volatile_input {
                            CalculationIssueCode::VolatileInputMissing
                        } else if engine
                            .has_unavailable_dependency(internal_id, &direct_unavailable)
                        {
                            CalculationIssueCode::BlockedByUpstream
                        } else {
                            CalculationIssueCode::UnsupportedExpression
                        };
                        CalculationCellResult::Unavailable(CalculationIssue::new(code, None))
                    }
                    value => CalculationCellResult::Value(cell_from_value(value)),
                }
            };
            cells.insert(public_id, result);
        }
    }
    let materialized_cells =
        build_materialization_view_cancellable(workbook, engine, &cells, cancelled)?;
    // Keyed off the materialized view, not `cells`: a legacy-array or dynamic-spill member is not
    // a formula cell, so keying off `cells` would drop its trace here while `seed_previous_results`
    // still restores its value — and an incremental recalculation would then answer differently
    // from the full calculation of the same workbook.
    let mut numeric_decimal_traces = BTreeMap::new();
    for public_id in materialized_cells.keys() {
        if cancelled() {
            return Err(());
        }
        let Some(internal_id) = public_to_internal(workbook, *public_id) else {
            continue;
        };
        if let Some(trace) = engine.calculated_decimal_trace(internal_id) {
            numeric_decimal_traces.insert(*public_id, trace);
        }
    }
    CalculationSnapshot::new_cancellable(
        cells,
        materialized_cells,
        numeric_decimal_traces,
        workbook,
        options,
        cancelled,
    )
}

fn build_materialization_view_cancellable(
    workbook: &WorkbookSnapshot,
    engine: &Engine<'_>,
    cells: &BTreeMap<CalculationCellId, CalculationCellResult>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<CalculationCellId, MaterializedCalculationCell>, ()> {
    let mut materialized = BTreeMap::new();
    for (cell, result) in cells {
        if cancelled() {
            return Err(());
        }
        materialized.insert(
            *cell,
            MaterializedCalculationCell::new(
                MaterializedResultOrigin::DirectFormula,
                result.clone(),
            ),
        );
    }
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let anchor = CalculationCellId::new(sheet.id(), cell.address());
            let Some(anchor_result) = cells.get(&anchor) else {
                continue;
            };
            let (range, origin) = match formula.metadata() {
                FormulaMetadata::Array { range, .. } if range.start() == cell.address() => (
                    *range,
                    MaterializedResultOrigin::LegacyArray {
                        anchor,
                        range: *range,
                    },
                ),
                FormulaMetadata::DynamicArray { .. } => {
                    let Some(resolved) = engine.dynamic_spill((
                        sheet_index,
                        cell.address().row().get(),
                        cell.address().column().get(),
                    )) else {
                        continue;
                    };
                    let range = crate::CellRange::new(
                        CellAddress::from_indices(resolved.row_start, resolved.col_start)
                            .expect("resolved dynamic start is valid"),
                        CellAddress::from_indices(resolved.row_end, resolved.col_end)
                            .expect("resolved dynamic end is valid"),
                    )
                    .expect("resolved dynamic range is ordered");
                    (
                        range,
                        MaterializedResultOrigin::DynamicSpill { anchor, range },
                    )
                }
                FormulaMetadata::Normal
                | FormulaMetadata::Shared { .. }
                | FormulaMetadata::Array { .. }
                | FormulaMetadata::DataTable { .. } => continue,
            };
            for row in range.start().row().get()..=range.end().row().get() {
                for column in range.start().column().get()..=range.end().column().get() {
                    if cancelled() {
                        return Err(());
                    }
                    let address = CellAddress::from_indices(row, column)
                        .expect("validated array range produces valid addresses");
                    let id = CalculationCellId::new(sheet.id(), address);
                    let result = if id == anchor {
                        anchor_result.clone()
                    } else {
                        match anchor_result {
                            CalculationCellResult::Unavailable(issue) => {
                                CalculationCellResult::Unavailable(issue.clone())
                            }
                            CalculationCellResult::Value(_) => {
                                CalculationCellResult::Value(super::convert::cell_from_value(
                                    engine.cell_value((sheet_index, row, column)),
                                ))
                            }
                        }
                    };
                    materialized.insert(id, MaterializedCalculationCell::new(origin, result));
                }
            }
        }
    }
    Ok(materialized)
}

fn resource_limit_issue(limit: CalculationLimitKind) -> CalculationIssue {
    CalculationIssue::new(
        CalculationIssueCode::ResourceLimitExceeded,
        Some(limit.detail().to_owned()),
    )
}

fn contains_volatility(
    engine: &Engine<'_>,
    sheet: usize,
    expr: &Expr,
    expected: Volatility,
) -> bool {
    expr_contains_volatility(
        engine,
        NameScanContext::root(sheet),
        expr,
        expected,
        &mut BTreeSet::new(),
        &mut CapabilityScope::default(),
    )
}

fn expr_contains_volatility(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    expr: &Expr,
    expected: Volatility,
    names: &mut BTreeSet<DefinedLambdaId>,
    local_scope: &mut CapabilityScope,
) -> bool {
    match expr {
        Expr::Call { name, args } => {
            if let Some(arguments_are_reachable) =
                call_shadow_arguments_are_reachable(engine, sheet, name, args, local_scope)
            {
                let mut found = false;
                let shadow = call_shadow(engine, sheet, name, local_scope)
                    .expect("shadow reachability implies a shadow state");
                if shadow != CallableShadow::CyclicNonCallable
                    && let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                        sheet.sheet,
                        sheet.defined_name_scope,
                        name,
                    )
                    && names.insert(id.clone())
                {
                    found |= expr_contains_volatility(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        expected,
                        names,
                        &mut CapabilityScope::default(),
                    );
                }
                return found
                    || (arguments_are_reachable
                        && args.iter().any(|arg| {
                            expr_contains_volatility(
                                engine,
                                sheet,
                                arg,
                                expected,
                                names,
                                local_scope,
                            )
                        }));
            }
            if is_supported_function(name)
                && !function_arguments_are_reachable(
                    name,
                    args,
                    engine.calculation_limits().max_let_bindings(),
                )
            {
                return false;
            }
            if function_volatility(name) == Some(expected) {
                return true;
            }
            match dynamic_function(name) {
                Some(DynamicFunction::Let) => {
                    let previous_len = local_scope.len();
                    let mut found = false;
                    if let Some((final_expr, pairs)) = args.split_last() {
                        for pair in pairs.chunks_exact(2) {
                            found |= expr_contains_volatility(
                                engine,
                                sheet,
                                &pair[1],
                                expected,
                                names,
                                local_scope,
                            );
                            if let Expr::Name(binding_name) = &pair[0] {
                                local_scope.push_expression(engine, sheet, binding_name, &pair[1]);
                            }
                        }
                        found |= expr_contains_volatility(
                            engine,
                            sheet,
                            final_expr,
                            expected,
                            names,
                            local_scope,
                        );
                    }
                    local_scope.truncate(previous_len);
                    return found;
                }
                Some(DynamicFunction::Lambda) => {
                    let Some(lambda) = definition(expr) else {
                        return false;
                    };
                    let previous_len = local_scope.len();
                    for parameter in lambda.parameters() {
                        local_scope.push_parameter(parameter.clone());
                    }
                    let found = expr_contains_volatility(
                        engine,
                        sheet,
                        lambda.body(),
                        expected,
                        names,
                        local_scope,
                    );
                    local_scope.truncate(previous_len);
                    return found;
                }
                Some(DynamicFunction::Map) => {
                    let Some((lambda_expr, array_exprs)) = args.split_last() else {
                        return false;
                    };
                    let Some(lambda) = definition(lambda_expr) else {
                        return false;
                    };
                    let mut found = array_exprs.iter().any(|arg| {
                        expr_contains_volatility(engine, sheet, arg, expected, names, local_scope)
                    });
                    let previous_len = local_scope.len();
                    for parameter in lambda.parameters() {
                        local_scope.push_parameter(parameter.clone());
                    }
                    found |= expr_contains_volatility(
                        engine,
                        sheet,
                        lambda.body(),
                        expected,
                        names,
                        local_scope,
                    );
                    local_scope.truncate(previous_len);
                    return found;
                }
                _ => {}
            }
            args.iter().any(|arg| {
                expr_contains_volatility(engine, sheet, arg, expected, names, local_scope)
            })
        }
        Expr::Invoke { callee, args } => {
            let cyclic_callee = typed_invocation_shadow(engine, sheet, callee, local_scope)
                .is_some_and(|(_, shadow)| shadow == CallableShadow::CyclicNonCallable);
            let callee_is_volatile = !cyclic_callee
                && expr_contains_volatility(engine, sheet, callee, expected, names, local_scope);
            callee_is_volatile
                || (typed_invocation_arguments_are_reachable(
                    engine,
                    sheet,
                    callee,
                    args,
                    local_scope,
                ) && args.iter().any(|arg| {
                    expr_contains_volatility(engine, sheet, arg, expected, names, local_scope)
                }))
        }
        Expr::ImplicitIntersection(inner)
        | Expr::SpillRef(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => {
            expr_contains_volatility(engine, sheet, inner, expected, names, local_scope)
        }
        Expr::Binary { left, right, .. }
        | Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right } => {
            expr_contains_volatility(engine, sheet, left, expected, names, local_scope)
                || expr_contains_volatility(engine, sheet, right, expected, names, local_scope)
        }
        Expr::Range { start, end } => {
            expr_contains_volatility(engine, sheet, start, expected, names, local_scope)
                || expr_contains_volatility(engine, sheet, end, expected, names, local_scope)
        }
        Expr::Array(rows) => rows.iter().flatten().any(|element| {
            expr_contains_volatility(engine, sheet, element, expected, names, local_scope)
        }),
        Expr::Name(name) => {
            if local_scope.lookup(name).is_some() {
                return false;
            }
            engine
                .resolve_name_expr_with_id_for_scope(sheet.sheet, sheet.defined_name_scope, name)
                .is_some_and(|(id, named)| {
                    if !names.insert(id.clone()) {
                        return false;
                    }
                    expr_contains_volatility(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        expected,
                        names,
                        &mut CapabilityScope::default(),
                    )
                })
        }
        Expr::BuiltinCallable(callable) => {
            let name = callable.canonical_name();
            if local_scope.lookup(name).is_some() {
                return false;
            }
            engine
                .resolve_name_expr_with_id_for_scope(sheet.sheet, sheet.defined_name_scope, name)
                .is_some_and(|(id, named)| {
                    if !names.insert(id.clone()) {
                        return false;
                    }
                    expr_contains_volatility(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        expected,
                        names,
                        &mut CapabilityScope::default(),
                    )
                })
        }
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

#[derive(Clone)]
struct CapabilityBinding {
    name: String,
    expression: Option<Expr>,
    callable_shadow: CallableShadow,
}

#[derive(Default)]
struct CapabilityScope {
    entries: Vec<CapabilityBinding>,
}

impl CapabilityScope {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
    }

    fn push_parameter(&mut self, name: String) {
        self.entries.push(CapabilityBinding {
            name,
            expression: None,
            callable_shadow: CallableShadow::Unknown,
        });
    }

    fn push_expression(
        &mut self,
        engine: &Engine<'_>,
        sheet: NameScanContext,
        name: &str,
        value: &Expr,
    ) {
        let locals = self
            .entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.callable_shadow))
            .collect::<Vec<_>>();
        let mut resolve = |name: &str| -> Result<CallableShadow, std::convert::Infallible> {
            Ok(engine.callable_shadow_for_name(sheet.sheet, sheet.defined_name_scope, name))
        };
        let callable_shadow = classify_callable_value(
            value,
            &locals,
            engine.calculation_limits().max_let_bindings(),
            &mut resolve,
        )
        .expect("static callable classification is infallible");
        self.entries.push(CapabilityBinding {
            name: canonical_local_name(name),
            expression: Some(value.clone()),
            callable_shadow,
        });
    }

    fn lookup(&self, name: &str) -> Option<Option<Expr>> {
        let key = canonical_local_name(name);
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.name == key)
            .map(|entry| entry.expression.clone())
    }

    fn callable_shadow(&self, name: &str) -> Option<CallableShadow> {
        let key = canonical_local_name(name);
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.name == key)
            .map(|entry| entry.callable_shadow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CapabilityInspectionPolicy {
    sheet_span: SheetSpanPolicy,
    suppress_missing_names: bool,
}

impl CapabilityInspectionPolicy {
    const fn new(sheet_span: SheetSpanPolicy, suppress_missing_names: bool) -> Self {
        Self {
            sheet_span,
            suppress_missing_names,
        }
    }

    const fn with_sheet_span(self, sheet_span: SheetSpanPolicy) -> Self {
        Self { sheet_span, ..self }
    }
}

fn inspect_expr(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    expr: &Expr,
    policy: CapabilityInspectionPolicy,
    // Keyed by policy as well as by stable defined-name identity: the sheet-range diagnosis
    // depends on the context a name
    // is reached from, so `SUM(N)+COUNTBLANK(N)` must expand `N` under both policies. Keying by
    // name alone let whichever operand came first decide the whole formula's classification.
    // Re-expansion cannot duplicate issues because the caller sorts and dedups them per cell.
    names: &mut HashSet<(DefinedLambdaId, CapabilityInspectionPolicy)>,
    local_scope: &mut CapabilityScope,
    issues: &mut Vec<CalculationIssue>,
) {
    match expr {
        Expr::Call { name, args } => {
            let normalized = normalize_name(name);
            if let Some(arguments_are_reachable) =
                call_shadow_arguments_are_reachable(engine, sheet, name, args, local_scope)
            {
                if let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                    sheet.sheet,
                    sheet.defined_name_scope,
                    name,
                ) && names.insert((id.clone(), policy))
                {
                    inspect_expr(
                        engine,
                        sheet.for_definition(id.scope()),
                        named,
                        policy,
                        names,
                        &mut CapabilityScope::default(),
                        issues,
                    );
                }
                if arguments_are_reachable {
                    let shadow = call_shadow(engine, sheet, name, local_scope)
                        .expect("shadow reachability implies a shadow state");
                    let sheet_span = match shadow {
                        CallableShadow::Callable(super::functions::CallableArity::Builtin(
                            callable,
                        )) => descriptor_sheet_span_policy(callable.canonical_name())
                            .unwrap_or(ARRAY_EXPRESSION_POLICY),
                        CallableShadow::Unshadowed
                        | CallableShadow::Callable(_)
                        | CallableShadow::Unknown
                        | CallableShadow::DefinitelyNonCallable
                        | CallableShadow::CyclicNonCallable => ARRAY_EXPRESSION_POLICY,
                    };
                    let argument_policy = policy.with_sheet_span(sheet_span);
                    for arg in args {
                        inspect_expr(
                            engine,
                            sheet,
                            arg,
                            argument_policy,
                            names,
                            local_scope,
                            issues,
                        );
                    }
                }
                return;
            }
            if !is_supported_function(name) {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedFunction,
                    Some(name.to_ascii_uppercase()),
                ));
            }
            if is_supported_function(name)
                && !function_arguments_are_reachable(
                    name,
                    args,
                    engine.calculation_limits().max_let_bindings(),
                )
            {
                return;
            }
            if dynamic_function(name) == Some(DynamicFunction::Let) {
                inspect_let(engine, sheet, args, policy, names, local_scope, issues);
                return;
            }
            if dynamic_function(name) == Some(DynamicFunction::Lambda)
                && let Some(lambda) = definition(expr)
            {
                let previous_len = local_scope.len();
                for parameter in lambda.parameters() {
                    local_scope.push_parameter(parameter.clone());
                }
                inspect_expr(
                    engine,
                    sheet,
                    lambda.body(),
                    policy,
                    names,
                    local_scope,
                    issues,
                );
                local_scope.truncate(previous_len);
                return;
            }
            if dynamic_function(name) == Some(DynamicFunction::Lambda) {
                return;
            }
            let argument_policy =
                descriptor_sheet_span_policy(&normalized).unwrap_or(SheetSpanPolicy::Unsupported);
            let suppresses_missing_names = matches!(
                function_dependency_kind(&normalized),
                Some(DependencyKind::ReferenceMetadataOnly(
                    ReferenceMetadataKind::Predicate
                ))
            );
            let argument_policy = CapabilityInspectionPolicy::new(
                argument_policy,
                policy.suppress_missing_names || suppresses_missing_names,
            );
            if dynamic_function(name) == Some(DynamicFunction::Map)
                && let Some((lambda_expr, array_exprs)) = args.split_last()
                && let Some(lambda) = definition(lambda_expr)
            {
                for arg in array_exprs {
                    inspect_expr(
                        engine,
                        sheet,
                        arg,
                        argument_policy,
                        names,
                        local_scope,
                        issues,
                    );
                }
                let previous_len = local_scope.len();
                for parameter in lambda.parameters() {
                    local_scope.push_parameter(parameter.clone());
                }
                inspect_expr(
                    engine,
                    sheet,
                    lambda.body(),
                    argument_policy,
                    names,
                    local_scope,
                    issues,
                );
                local_scope.truncate(previous_len);
                return;
            }
            for (index, arg) in args.iter().enumerate() {
                let argument_policy =
                    if function_argument_is_callable(&normalized, index, args.len())
                        && callable_argument_names_known_function(arg)
                    {
                        CapabilityInspectionPolicy::new(argument_policy.sheet_span, true)
                    } else {
                        argument_policy
                    };
                inspect_expr(
                    engine,
                    sheet,
                    arg,
                    argument_policy,
                    names,
                    local_scope,
                    issues,
                );
            }
        }
        Expr::Invoke { callee, args } => {
            let cyclic_callee = typed_invocation_shadow(engine, sheet, callee, local_scope)
                .is_some_and(|(_, shadow)| shadow == CallableShadow::CyclicNonCallable);
            if !cyclic_callee {
                inspect_expr(engine, sheet, callee, policy, names, local_scope, issues);
            }
            if !typed_invocation_arguments_are_reachable(engine, sheet, callee, args, local_scope) {
                return;
            }
            let argument_policy = typed_invocation_shadow(engine, sheet, callee, local_scope)
                .and_then(|(callable, shadow)| match shadow {
                    CallableShadow::Unshadowed => Some(callable),
                    CallableShadow::Callable(super::functions::CallableArity::Builtin(
                        callable,
                    )) => Some(callable),
                    CallableShadow::Callable(_)
                    | CallableShadow::Unknown
                    | CallableShadow::DefinitelyNonCallable
                    | CallableShadow::CyclicNonCallable => None,
                })
                .and_then(|callable| descriptor_sheet_span_policy(callable.canonical_name()))
                .unwrap_or(ARRAY_EXPRESSION_POLICY);
            let argument_policy = policy.with_sheet_span(argument_policy);
            for arg in args {
                inspect_expr(
                    engine,
                    sheet,
                    arg,
                    argument_policy,
                    names,
                    local_scope,
                    issues,
                );
            }
        }
        Expr::Name(name) => {
            if let Some(binding) = local_scope.lookup(name) {
                if let Some(binding) = binding {
                    inspect_expr(engine, sheet, &binding, policy, names, local_scope, issues);
                }
                return;
            }
            match engine.resolve_name_expr_with_id_for_scope(
                sheet.sheet,
                sheet.defined_name_scope,
                name,
            ) {
                Some((id, named)) => {
                    if names.insert((id.clone(), policy)) {
                        let mut defined_scope = CapabilityScope::default();
                        inspect_expr(
                            engine,
                            sheet.for_definition(id.scope()),
                            named,
                            policy,
                            names,
                            &mut defined_scope,
                            issues,
                        );
                    }
                }
                None if policy.suppress_missing_names => {}
                None => issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedName,
                    Some(name.clone()),
                )),
            }
        }
        Expr::BuiltinCallable(callable) => {
            let name = callable.canonical_name();
            if let Some(binding) = local_scope.lookup(name) {
                if let Some(binding) = binding {
                    inspect_expr(engine, sheet, &binding, policy, names, local_scope, issues);
                }
                return;
            }
            if let Some((id, named)) = engine.resolve_name_expr_with_id_for_scope(
                sheet.sheet,
                sheet.defined_name_scope,
                name,
            ) && names.insert((id.clone(), policy))
            {
                let mut defined_scope = CapabilityScope::default();
                inspect_expr(
                    engine,
                    sheet.for_definition(id.scope()),
                    named,
                    policy,
                    names,
                    &mut defined_scope,
                    issues,
                );
            }
        }
        Expr::Array(rows) => {
            for row in rows {
                for element in row {
                    inspect_expr(
                        engine,
                        sheet,
                        element,
                        policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                        names,
                        local_scope,
                        issues,
                    );
                }
            }
        }
        Expr::SpillRef(inner) => {
            inspect_expr(
                engine,
                sheet,
                inner,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
        }
        Expr::ImplicitIntersection(inner) | Expr::Unary { operand: inner, .. } => {
            inspect_expr(
                engine,
                sheet,
                inner,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
        }
        Expr::Paren(inner) => {
            inspect_expr(engine, sheet, inner, policy, names, local_scope, issues);
        }
        Expr::ReferenceUnion { left, right } | Expr::ReferenceIntersection { left, right } => {
            inspect_expr(engine, sheet, left, policy, names, local_scope, issues);
            inspect_expr(engine, sheet, right, policy, names, local_scope, issues);
        }
        Expr::Binary { left, right, .. } => {
            inspect_expr(
                engine,
                sheet,
                left,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
            inspect_expr(
                engine,
                sheet,
                right,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
        }
        Expr::Range { start, end } => {
            inspect_expr(
                engine,
                sheet,
                start,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
            inspect_expr(
                engine,
                sheet,
                end,
                policy.with_sheet_span(ARRAY_EXPRESSION_POLICY),
                names,
                local_scope,
                issues,
            );
        }
        Expr::Ref(reference) => {
            // One reference carries one diagnosis, and the workbook prefix is the outer one: a
            // reader told to remove a 3-D range from a formula that has none is routed to the
            // wrong remedy. Evaluation resolves the prefix in the same order.
            if let Some(detail) = reference
                .sheet
                .as_ref()
                .and_then(SheetPrefix::external_workbook_detail)
            {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedExpression,
                    Some(detail),
                ));
            } else if let Some(detail) = reference
                .sheet
                .as_ref()
                .and_then(SheetPrefix::sheet_range_detail)
                .filter(|_| matches!(policy.sheet_span, SheetSpanPolicy::Unsupported))
            {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedSheetRange,
                    Some(detail),
                ));
            }
        }
        Expr::StructuredRef(_) => {}
        Expr::ExternalReference(reference) => {
            let mut detail = reference.workbook.to_string();
            if let Some(sheet) = &reference.sheet {
                detail.push_str(sheet);
                if let Some(sheet_end) = &reference.sheet_end {
                    detail.push(':');
                    detail.push_str(sheet_end);
                }
            }
            issues.push(CalculationIssue::new(
                CalculationIssueCode::UnsupportedExpression,
                Some(detail),
            ));
        }
        Expr::QualifiedName { name, .. } => {
            issues.push(CalculationIssue::new(
                CalculationIssueCode::UnsupportedName,
                Some(name.to_string()),
            ));
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) | Expr::Missing => {}
    }
}

fn callable_argument_names_known_function(expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => is_supported_function(name),
        Expr::Paren(inner) => callable_argument_names_known_function(inner),
        _ => false,
    }
}

fn inspect_let(
    engine: &Engine<'_>,
    sheet: NameScanContext,
    args: &[Expr],
    result_policy: CapabilityInspectionPolicy,
    names: &mut HashSet<(DefinedLambdaId, CapabilityInspectionPolicy)>,
    local_scope: &mut CapabilityScope,
    issues: &mut Vec<CalculationIssue>,
) {
    let previous_len = local_scope.len();
    let Some((final_expr, pairs)) = args.split_last() else {
        return;
    };
    for pair in pairs.chunks_exact(2) {
        inspect_expr(
            engine,
            sheet,
            &pair[1],
            result_policy.with_sheet_span(SheetSpanPolicy::CollectAcrossSheets),
            names,
            local_scope,
            issues,
        );
        if let Expr::Name(name) = &pair[0] {
            local_scope.push_expression(engine, sheet, name, &pair[1]);
        }
    }
    inspect_expr(
        engine,
        sheet,
        final_expr,
        result_policy,
        names,
        local_scope,
        issues,
    );
    local_scope.truncate(previous_len);
}
