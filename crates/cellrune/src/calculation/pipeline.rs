use std::collections::{BTreeMap, BTreeSet};

use super::ast::{Expr, SheetPrefix};
use super::convert::cell_from_value;
use super::error::parse_error_detail;
use super::eval::{CompiledWorkbook, Engine, public_to_internal};
use super::functions::{is_supported_function, normalize_name};
use super::lambda::{canonical_parameter_name, definition as lambda_definition};
use super::value::{ErrorKind, Value};
use super::{
    CalculationCellId, CalculationCellResult, CalculationIssue, CalculationIssueCode,
    CalculationLimitKind, CalculationOptions, CalculationSnapshot, FormulaCapability,
    FormulaCapabilityEntry, FormulaCapabilityReport, FunctionSupport, FunctionUsageEntry,
    FunctionUsageReport, MaterializedCalculationCell, MaterializedResultOrigin,
};
use crate::{CellAddress, CellContent, FormulaMetadata, WorkbookSnapshot};

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
            collect_function_calls(expr, &mut calls);
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

fn collect_function_calls(expr: &Expr, output: &mut Vec<String>) {
    match expr {
        Expr::Call { name, args } => {
            output.push(normalize_name(name));
            for arg in args {
                collect_function_calls(arg, output);
            }
        }
        Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => collect_function_calls(inner, output),
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            collect_function_calls(left, output);
            collect_function_calls(right, output);
        }
        Expr::Array(rows) => {
            for item in rows.iter().flatten() {
                collect_function_calls(item, output);
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::Name(_)
        | Expr::Missing => {}
    }
}

fn scan_with_engine(workbook: &WorkbookSnapshot, engine: &Engine<'_>) -> FormulaCapabilityReport {
    let mut entries = Vec::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
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
                            Some(parse_error_detail(error.position, error.message)),
                        )),
                    },
                    None if has_name_cycle || has_name_limit => {}
                    None => match engine.parsed_expr(internal_id) {
                        Some(expr) => inspect_expr(
                            engine,
                            sheet_index,
                            expr,
                            &mut BTreeSet::new(),
                            &mut Vec::new(),
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
    FormulaCapabilityReport::new(entries)
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
    let snapshot = snapshot_from_engine(workbook, options, &engine);
    let compiled = engine.compiled();
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
        Engine::evaluate_compiled(workbook, options, compiled, previous, dirty, cancelled)?;
    let evaluated = engine.evaluated_cell_count();
    let snapshot = snapshot_from_engine(workbook, options, &engine);
    Ok((snapshot, evaluated))
}

fn snapshot_from_engine(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
    engine: &Engine<'_>,
) -> CalculationSnapshot {
    let report = scan_with_engine(workbook, engine);
    let unsupported: BTreeMap<CalculationCellId, CalculationIssue> = report
        .entries()
        .iter()
        .filter_map(|entry| match entry.capability() {
            FormulaCapability::Supported => None,
            FormulaCapability::Unsupported(issues) => {
                issues.first().cloned().map(|issue| (entry.cell(), issue))
            }
        })
        .collect();
    let direct_unavailable: BTreeSet<_> = workbook
        .sheets()
        .iter()
        .enumerate()
        .flat_map(|(sheet_index, sheet)| {
            unsupported.keys().filter_map(move |cell| {
                (cell.sheet_id() == sheet.id()).then_some((
                    sheet_index,
                    cell.address().row().get(),
                    cell.address().column().get(),
                ))
            })
        })
        .collect();
    let mut cells = BTreeMap::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            let CellContent::Formula(_) = cell.content() else {
                continue;
            };
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
                                    && contains_function(expr, "TODAY"))
                                    || (options.now_serial().is_none()
                                        && contains_function(expr, "NOW"))
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
    let materialized_cells = build_materialization_view(workbook, engine, &cells);
    let numeric_decimal_traces = cells
        .keys()
        .filter_map(|public_id| {
            let internal_id = public_to_internal(workbook, *public_id)?;
            engine
                .calculated_decimal_trace(internal_id)
                .map(|trace| (*public_id, trace))
        })
        .collect();
    CalculationSnapshot::new(
        cells,
        materialized_cells,
        numeric_decimal_traces,
        workbook,
        options,
    )
}

fn build_materialization_view(
    workbook: &WorkbookSnapshot,
    engine: &Engine<'_>,
    cells: &BTreeMap<CalculationCellId, CalculationCellResult>,
) -> BTreeMap<CalculationCellId, MaterializedCalculationCell> {
    let mut materialized = cells
        .iter()
        .map(|(cell, result)| {
            (
                *cell,
                MaterializedCalculationCell::new(
                    MaterializedResultOrigin::DirectFormula,
                    result.clone(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        for cell in sheet.cells() {
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
    materialized
}

fn resource_limit_issue(limit: CalculationLimitKind) -> CalculationIssue {
    CalculationIssue::new(
        CalculationIssueCode::ResourceLimitExceeded,
        Some(limit.detail().to_owned()),
    )
}

fn contains_function(expr: &Expr, expected: &str) -> bool {
    match expr {
        Expr::Call { name, args } => {
            normalize_name(name).eq_ignore_ascii_case(expected)
                || args.iter().any(|arg| contains_function(arg, expected))
        }
        Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => contains_function(inner, expected),
        Expr::Binary { left, right, .. } => {
            contains_function(left, expected) || contains_function(right, expected)
        }
        Expr::Range { start, end } => {
            contains_function(start, expected) || contains_function(end, expected)
        }
        Expr::Array(rows) => rows
            .iter()
            .flatten()
            .any(|element| contains_function(element, expected)),
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::Name(_)
        | Expr::Missing => false,
    }
}

fn inspect_expr(
    engine: &Engine<'_>,
    sheet: usize,
    expr: &Expr,
    names: &mut BTreeSet<String>,
    local_names: &mut Vec<String>,
    issues: &mut Vec<CalculationIssue>,
) {
    match expr {
        Expr::Call { name, args } => {
            if !is_supported_function(name) {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedFunction,
                    Some(name.to_ascii_uppercase()),
                ));
            }
            if normalize_name(name) == "MAP"
                && let Some((lambda_expr, array_exprs)) = args.split_last()
                && let Some(lambda) = lambda_definition(lambda_expr)
            {
                for arg in array_exprs {
                    inspect_expr(engine, sheet, arg, names, local_names, issues);
                }
                let previous_local_count = local_names.len();
                local_names.extend(lambda.parameters().iter().cloned());
                inspect_expr(engine, sheet, lambda.body(), names, local_names, issues);
                local_names.truncate(previous_local_count);
                return;
            }
            for arg in args {
                inspect_expr(engine, sheet, arg, names, local_names, issues);
            }
        }
        Expr::Name(name) => {
            let local_key = canonical_parameter_name(name);
            if local_names.iter().rev().any(|local| local == &local_key) {
                return;
            }
            let key = name.to_ascii_lowercase();
            if names.insert(key) {
                match engine.resolve_name_expr(sheet, name) {
                    Some(named) => inspect_expr(engine, sheet, named, names, local_names, issues),
                    None => issues.push(CalculationIssue::new(
                        CalculationIssueCode::UnsupportedName,
                        Some(name.clone()),
                    )),
                }
            }
        }
        Expr::Array(rows) => {
            for row in rows {
                for element in row {
                    inspect_expr(engine, sheet, element, names, local_names, issues);
                }
            }
        }
        Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => {
            inspect_expr(engine, sheet, inner, names, local_names, issues);
        }
        Expr::Binary { left, right, .. } => {
            inspect_expr(engine, sheet, left, names, local_names, issues);
            inspect_expr(engine, sheet, right, names, local_names, issues);
        }
        Expr::Range { start, end } => {
            inspect_expr(engine, sheet, start, names, local_names, issues);
            inspect_expr(engine, sheet, end, names, local_names, issues);
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
            {
                issues.push(CalculationIssue::new(
                    CalculationIssueCode::UnsupportedSheetRange,
                    Some(detail),
                ));
            }
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) | Expr::Missing => {}
    }
}
