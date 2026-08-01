use std::collections::{BTreeMap, BTreeSet};

use super::{
    DefinedNameAnalysis, DefinedNameAnalysisError, DefinedNameAnalysisLimitKind,
    DefinedNameAnalysisOptions, DefinedNameDynamicKind, DefinedNameExternalReference,
    DefinedNameExternalTargetKind, DefinedNameInvalidReason, DefinedNameReferenceArea,
    DefinedNameSheetSpan, DefinedNameUnsupportedReason,
};
use crate::calculation::ast::{Expr, ExternalReferenceTarget, Reference, StructuredItem};
use crate::calculation::error::parse_error_detail;
use crate::calculation::functions::descriptor::{DependencyKind, DynamicReferenceKind};
use crate::calculation::functions::kernel::LegacyFunction;
use crate::calculation::functions::{
    DynamicFunction, Evaluator, function_arguments_are_reachable, function_dependency_kind,
    function_evaluator,
};
use crate::calculation::lambda::{definition, definition_from_args, is_local_name};
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::parser::{ParseError, parse_formula_with_limits};
use crate::calculation::reference_resolution::{
    intersect_reference_values, intersection_reference_work, range_reference_rect,
    resolve_reference_span, resolve_structured_reference, union_reference_values,
    validate_explicit_structured_reference_target,
};
use crate::calculation::runtime::{ReferenceArea, ReferenceValue};
use crate::calculation::scope::{canonical_local_name, resolve_defined_name_scoped};
use crate::calculation::value::ErrorKind;
use crate::{
    CellAddress, CellRange, DefinedName, DefinedNameScope, FormulaText, SheetId, WorkbookSnapshot,
};

pub(super) fn analyze(
    workbook: &WorkbookSnapshot,
    name: &str,
    current_sheet: Option<SheetId>,
    options: DefinedNameAnalysisOptions,
    cancelled: &impl Fn() -> bool,
) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
    if cancelled() {
        return Err(DefinedNameAnalysisError::cancelled());
    }
    let current_sheet_index = match current_sheet {
        Some(sheet_id) => Some(
            workbook
                .sheets()
                .iter()
                .position(|sheet| sheet.id() == sheet_id)
                .ok_or_else(|| DefinedNameAnalysisError::unknown_sheet(sheet_id))?,
        ),
        None => None,
    };
    let root = resolve_defined_name_scoped(workbook, current_sheet, None, name);
    let Some((root_index, _)) = root else {
        return Ok(DefinedNameAnalysis::NotFound);
    };
    let mut analyzer = Analyzer {
        workbook,
        options,
        cancelled,
        parsed: BTreeMap::new(),
        parsed_callables: BTreeMap::new(),
        validating: BTreeSet::new(),
        validated: BTreeSet::new(),
        validating_callables: BTreeSet::new(),
        validated_callables: BTreeSet::new(),
        classifying: BTreeSet::new(),
        classified: BTreeMap::new(),
        remaining_scan_nodes: options.max_scan_nodes(),
        reference_work: 0,
    };
    if let Some(invalid) = analyzer.validate_reachable(root_index, current_sheet_index)? {
        return Ok(invalid);
    }
    let outcome = analyzer.classify_definition(root_index, current_sheet_index)?;
    analyzer.public_result(outcome)
}

struct Analyzer<'workbook, 'cancel> {
    workbook: &'workbook WorkbookSnapshot,
    options: DefinedNameAnalysisOptions,
    cancelled: &'cancel dyn Fn() -> bool,
    parsed: BTreeMap<usize, Result<crate::calculation::syntax::ParsedFormula, ParseError>>,
    parsed_callables: BTreeMap<usize, bool>,
    validating: BTreeSet<usize>,
    validated: BTreeSet<(usize, Option<usize>)>,
    validating_callables: BTreeSet<(usize, Option<usize>)>,
    validated_callables: BTreeSet<(usize, Option<usize>)>,
    classifying: BTreeSet<(usize, Option<usize>)>,
    classified: BTreeMap<(usize, Option<usize>), Outcome>,
    remaining_scan_nodes: u64,
    reference_work: u64,
}

#[derive(Debug, Clone)]
enum Outcome {
    Static(ReferenceValue),
    Dynamic {
        kind: DefinedNameDynamicKind,
        formula: FormulaText,
    },
    Constant {
        formula: FormulaText,
    },
    External(DefinedNameExternalReference),
    Invalid {
        reason: DefinedNameInvalidReason,
        detail: Option<Box<str>>,
    },
    Unsupported {
        reason: DefinedNameUnsupportedReason,
        detail: Option<Box<str>>,
    },
}

enum ValidationTask {
    EnterDefinition {
        index: usize,
        context_sheet: Option<usize>,
        chain_depth: u64,
    },
    LeaveDefinition {
        index: usize,
        key: (usize, Option<usize>),
    },
    EnterCallable {
        index: usize,
        context_sheet: Option<usize>,
        chain_depth: u64,
    },
    LeaveCallable {
        key: (usize, Option<usize>),
    },
    Expr {
        expr: Expr,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        local_names: Vec<String>,
        chain_depth: u64,
    },
}

enum ClassificationTask {
    EnterDefinition {
        index: usize,
        context_sheet: Option<usize>,
        chain_depth: u64,
    },
    FinishDefinition {
        key: (usize, Option<usize>),
    },
    Expr {
        expr: Expr,
        formula: FormulaText,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        chain_depth: u64,
    },
    CombineUnion {
        formula: FormulaText,
    },
    CombineIntersection {
        formula: FormulaText,
    },
    CombineRange {
        formula: FormulaText,
    },
}

impl Analyzer<'_, '_> {
    fn check_cancelled(&self) -> Result<(), DefinedNameAnalysisError> {
        if (self.cancelled)() {
            Err(DefinedNameAnalysisError::cancelled())
        } else {
            Ok(())
        }
    }

    fn charge_scan_node(&mut self) -> Result<(), DefinedNameAnalysisError> {
        self.charge_scan_nodes(1)
    }

    fn charge_scan_nodes(&mut self, count: u64) -> Result<(), DefinedNameAnalysisError> {
        self.check_cancelled()?;
        self.remaining_scan_nodes =
            self.remaining_scan_nodes
                .checked_sub(count)
                .ok_or_else(|| {
                    DefinedNameAnalysisError::resource(DefinedNameAnalysisLimitKind::ScanNodes)
                })?;
        Ok(())
    }

    fn charge_reference_work(&mut self, work: u64) -> Result<(), DefinedNameAnalysisError> {
        self.check_cancelled()?;
        self.reference_work = self.reference_work.checked_add(work).ok_or_else(|| {
            DefinedNameAnalysisError::resource(DefinedNameAnalysisLimitKind::FunctionIterations)
        })?;
        if self.reference_work
            > self
                .options
                .calculation()
                .limits()
                .max_function_iterations()
        {
            return Err(DefinedNameAnalysisError::resource(
                DefinedNameAnalysisLimitKind::FunctionIterations,
            ));
        }
        Ok(())
    }

    fn check_area_count(&self, count: usize) -> Result<(), DefinedNameAnalysisError> {
        if u64::try_from(count).map_or(true, |count| {
            count > self.options.calculation().limits().max_reference_areas()
        }) {
            return Err(DefinedNameAnalysisError::resource(
                DefinedNameAnalysisLimitKind::ReferenceAreas,
            ));
        }
        Ok(())
    }

    fn definition(&self, index: usize) -> &DefinedName {
        &self.workbook.defined_names()[index]
    }

    fn parsed_expr(
        &mut self,
        index: usize,
    ) -> Result<Result<Expr, Outcome>, DefinedNameAnalysisError> {
        self.check_cancelled()?;
        if !self.parsed.contains_key(&index) {
            let formula = self.definition(index).formula();
            let parsed =
                parse_formula_with_limits(formula.as_str(), self.options.calculation().limits());
            self.parsed.insert(index, parsed);
        }
        match self
            .parsed
            .get(&index)
            .expect("the parsed definition was inserted")
        {
            Ok(parsed) => Ok(Ok(parsed.root().clone())),
            Err(error) => match error.limit {
                Some(limit) => Err(DefinedNameAnalysisError::resource(parse_limit(limit))),
                None => Ok(Err(Outcome::Invalid {
                    reason: DefinedNameInvalidReason::ParseError,
                    detail: Some(parse_error_detail(error).into_boxed_str()),
                })),
            },
        }
    }

    fn parsed_is_callable(
        &mut self,
        index: usize,
    ) -> Result<Result<bool, Outcome>, DefinedNameAnalysisError> {
        self.check_cancelled()?;
        if let Some(callable) = self.parsed_callables.get(&index) {
            return Ok(Ok(*callable));
        }
        if !self.parsed.contains_key(&index) {
            let formula = self.definition(index).formula();
            let parsed =
                parse_formula_with_limits(formula.as_str(), self.options.calculation().limits());
            self.parsed.insert(index, parsed);
        }
        match self
            .parsed
            .get(&index)
            .expect("the parsed definition was inserted")
        {
            Ok(parsed) => {
                let callable = definition(parsed.root()).is_some();
                self.parsed_callables.insert(index, callable);
                Ok(Ok(callable))
            }
            Err(error) => match error.limit {
                Some(limit) => Err(DefinedNameAnalysisError::resource(parse_limit(limit))),
                None => Ok(Err(Outcome::Invalid {
                    reason: DefinedNameInvalidReason::ParseError,
                    detail: Some(parse_error_detail(error).into_boxed_str()),
                })),
            },
        }
    }

    fn lookup_name(
        &self,
        context_sheet: Option<usize>,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
    ) -> Option<(usize, &DefinedName)> {
        resolve_defined_name_scoped(
            self.workbook,
            context_sheet.map(|index| self.workbook.sheets()[index].id()),
            lookup_scope,
            name,
        )
    }

    fn validate_reachable(
        &mut self,
        root_index: usize,
        root_sheet: Option<usize>,
    ) -> Result<Option<DefinedNameAnalysis>, DefinedNameAnalysisError> {
        let mut tasks = vec![ValidationTask::EnterDefinition {
            index: root_index,
            context_sheet: root_sheet,
            chain_depth: 1,
        }];
        while let Some(task) = tasks.pop() {
            self.check_cancelled()?;
            match task {
                ValidationTask::EnterDefinition {
                    index,
                    context_sheet,
                    chain_depth,
                } => {
                    let key = (index, context_sheet);
                    if self.validated.contains(&key) {
                        continue;
                    }
                    if chain_depth > self.options.max_name_chain_depth() {
                        return Err(DefinedNameAnalysisError::resource(
                            DefinedNameAnalysisLimitKind::NameChainDepth,
                        ));
                    }
                    if !self.validating.insert(index) {
                        return Ok(Some(DefinedNameAnalysis::Invalid {
                            reason: DefinedNameInvalidReason::CircularReference,
                            detail: Some(self.definition(index).name().to_owned().into_boxed_str()),
                        }));
                    }
                    let scope = self.definition(index).scope();
                    let expr = match self.parsed_expr(index)? {
                        Ok(expr) => expr,
                        Err(Outcome::Invalid { reason, detail }) => {
                            return Ok(Some(DefinedNameAnalysis::Invalid { reason, detail }));
                        }
                        Err(_) => unreachable!("parsing produces only invalid outcomes"),
                    };
                    tasks.push(ValidationTask::LeaveDefinition { index, key });
                    tasks.push(ValidationTask::Expr {
                        expr,
                        context_sheet,
                        lookup_scope: scope,
                        local_names: Vec::new(),
                        chain_depth,
                    });
                }
                ValidationTask::LeaveDefinition { index, key } => {
                    self.validating.remove(&index);
                    self.validated.insert(key);
                }
                ValidationTask::EnterCallable {
                    index,
                    context_sheet,
                    chain_depth,
                } => {
                    let key = (index, context_sheet);
                    if self.validated_callables.contains(&key)
                        || !self.validating_callables.insert(key)
                    {
                        continue;
                    }
                    if chain_depth > self.options.max_name_chain_depth() {
                        return Err(DefinedNameAnalysisError::resource(
                            DefinedNameAnalysisLimitKind::NameChainDepth,
                        ));
                    }
                    let named = match self.parsed_expr(index)? {
                        Ok(named) => named,
                        Err(Outcome::Invalid { reason, detail }) => {
                            return Ok(Some(DefinedNameAnalysis::Invalid { reason, detail }));
                        }
                        Err(_) => unreachable!("parsing produces only invalid outcomes"),
                    };
                    let Some(lambda) = definition(&named) else {
                        self.validating_callables.remove(&key);
                        continue;
                    };
                    let parameters = lambda.parameters().to_vec();
                    let body = lambda.body().clone();
                    self.charge_scan_nodes(
                        u64::try_from(parameters.len())
                            .unwrap_or(u64::MAX)
                            .saturating_add(1),
                    )?;
                    tasks.push(ValidationTask::LeaveCallable { key });
                    tasks.push(ValidationTask::Expr {
                        expr: body,
                        context_sheet,
                        lookup_scope: self.definition(index).scope(),
                        local_names: parameters,
                        chain_depth,
                    });
                }
                ValidationTask::LeaveCallable { key } => {
                    self.validating_callables.remove(&key);
                    self.validated_callables.insert(key);
                }
                ValidationTask::Expr {
                    expr,
                    context_sheet,
                    lookup_scope,
                    local_names,
                    chain_depth,
                } => {
                    self.charge_scan_node()?;
                    if let Some(invalid) = self.validate_expr_task(
                        expr,
                        context_sheet,
                        lookup_scope,
                        local_names,
                        chain_depth,
                        &mut tasks,
                    )? {
                        return Ok(Some(invalid));
                    }
                }
            }
        }
        Ok(None)
    }

    fn validate_expr_task(
        &mut self,
        expr: Expr,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        local_names: Vec<String>,
        chain_depth: u64,
        tasks: &mut Vec<ValidationTask>,
    ) -> Result<Option<DefinedNameAnalysis>, DefinedNameAnalysisError> {
        match expr {
            Expr::Name(name) if !is_local_name(&name, &local_names) => {
                let Some((index, _)) = self.lookup_name(context_sheet, Some(lookup_scope), &name)
                else {
                    return Ok(Some(DefinedNameAnalysis::Invalid {
                        reason: DefinedNameInvalidReason::UnresolvedName,
                        detail: Some(name.into_boxed_str()),
                    }));
                };
                let callable = match self.parsed_is_callable(index)? {
                    Ok(callable) => callable,
                    Err(Outcome::Invalid { reason, detail }) => {
                        return Ok(Some(DefinedNameAnalysis::Invalid { reason, detail }));
                    }
                    Err(_) => unreachable!("parsing produces only invalid outcomes"),
                };
                if callable {
                    tasks.push(ValidationTask::EnterCallable {
                        index,
                        context_sheet,
                        chain_depth: chain_depth + 1,
                    });
                } else {
                    tasks.push(ValidationTask::EnterDefinition {
                        index,
                        context_sheet,
                        chain_depth: chain_depth + 1,
                    });
                }
                Ok(None)
            }
            Expr::QualifiedName { sheet, name } => {
                if sheet.end_name.is_some() {
                    return Ok(None);
                }
                let Some(sheet_index) = self.workbook.sheet_index_by_name(&sheet.name) else {
                    return Ok(Some(DefinedNameAnalysis::Invalid {
                        reason: DefinedNameInvalidReason::InvalidReference,
                        detail: Some(sheet.name.clone().into_boxed_str()),
                    }));
                };
                let sheet_id = self.workbook.sheets()[sheet_index].id();
                let Some((index, _)) = self.lookup_name(
                    Some(sheet_index),
                    Some(DefinedNameScope::Sheet(sheet_id)),
                    &name,
                ) else {
                    return Ok(Some(DefinedNameAnalysis::Invalid {
                        reason: DefinedNameInvalidReason::UnresolvedName,
                        detail: Some(name.to_string().into_boxed_str()),
                    }));
                };
                let callable = match self.parsed_is_callable(index)? {
                    Ok(callable) => callable,
                    Err(Outcome::Invalid { reason, detail }) => {
                        return Ok(Some(DefinedNameAnalysis::Invalid { reason, detail }));
                    }
                    Err(_) => unreachable!("parsing produces only invalid outcomes"),
                };
                if callable {
                    tasks.push(ValidationTask::EnterCallable {
                        index,
                        context_sheet: Some(sheet_index),
                        chain_depth: chain_depth + 1,
                    });
                } else {
                    tasks.push(ValidationTask::EnterDefinition {
                        index,
                        context_sheet: Some(sheet_index),
                        chain_depth: chain_depth + 1,
                    });
                }
                Ok(None)
            }
            Expr::Call { name, args } => {
                let evaluator = function_evaluator(&name);
                if !is_local_name(&name, &local_names)
                    && let Some((index, _)) =
                        self.lookup_name(context_sheet, Some(lookup_scope), &name)
                {
                    let callable = match self.parsed_is_callable(index)? {
                        Ok(callable) => callable,
                        Err(Outcome::Invalid { reason, detail }) => {
                            return Ok(Some(DefinedNameAnalysis::Invalid { reason, detail }));
                        }
                        Err(_) => unreachable!("parsing produces only invalid outcomes"),
                    };
                    Self::push_validation_exprs(
                        tasks,
                        args,
                        context_sheet,
                        lookup_scope,
                        &local_names,
                        chain_depth,
                    );
                    if callable {
                        tasks.push(ValidationTask::EnterCallable {
                            index,
                            context_sheet,
                            chain_depth: chain_depth + 1,
                        });
                    } else {
                        tasks.push(ValidationTask::EnterDefinition {
                            index,
                            context_sheet,
                            chain_depth: chain_depth + 1,
                        });
                    }
                    return Ok(None);
                }
                if is_local_name(&name, &local_names) {
                    Self::push_validation_exprs(
                        tasks,
                        args,
                        context_sheet,
                        lookup_scope,
                        &local_names,
                        chain_depth,
                    );
                    return Ok(None);
                }
                if evaluator.is_some()
                    && !function_arguments_are_reachable(
                        &name,
                        &args,
                        self.options.calculation().limits().max_let_bindings(),
                    )
                {
                    return Ok(None);
                }
                if evaluator == Some(Evaluator::Dynamic(DynamicFunction::Lambda)) {
                    if let Some(lambda) = definition_from_args(&args) {
                        let mut lambda_locals = local_names;
                        lambda_locals.extend(lambda.parameters().iter().cloned());
                        self.charge_scan_nodes(
                            u64::try_from(lambda.parameters().len()).unwrap_or(u64::MAX),
                        )?;
                        tasks.push(ValidationTask::Expr {
                            expr: lambda.body().clone(),
                            context_sheet,
                            lookup_scope,
                            local_names: lambda_locals,
                            chain_depth,
                        });
                    } else {
                        Self::push_validation_exprs(
                            tasks,
                            args,
                            context_sheet,
                            lookup_scope,
                            &local_names,
                            chain_depth,
                        );
                    }
                    return Ok(None);
                }
                if evaluator == Some(Evaluator::Dynamic(DynamicFunction::Let)) {
                    Self::push_let_validation(
                        tasks,
                        args,
                        context_sheet,
                        lookup_scope,
                        local_names,
                        chain_depth,
                    );
                    return Ok(None);
                }
                Self::push_validation_exprs(
                    tasks,
                    args,
                    context_sheet,
                    lookup_scope,
                    &local_names,
                    chain_depth,
                );
                Ok(None)
            }
            Expr::Invoke { callee, args } => {
                Self::push_validation_exprs(
                    tasks,
                    args,
                    context_sheet,
                    lookup_scope,
                    &local_names,
                    chain_depth,
                );
                tasks.push(ValidationTask::Expr {
                    expr: *callee,
                    context_sheet,
                    lookup_scope,
                    local_names,
                    chain_depth,
                });
                Ok(None)
            }
            Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            }
            | Expr::Binary { left, right, .. } => {
                tasks.push(ValidationTask::Expr {
                    expr: *right,
                    context_sheet,
                    lookup_scope,
                    local_names: local_names.clone(),
                    chain_depth,
                });
                tasks.push(ValidationTask::Expr {
                    expr: *left,
                    context_sheet,
                    lookup_scope,
                    local_names,
                    chain_depth,
                });
                Ok(None)
            }
            Expr::SpillRef(inner)
            | Expr::ImplicitIntersection(inner)
            | Expr::Unary { operand: inner, .. }
            | Expr::Paren(inner) => {
                tasks.push(ValidationTask::Expr {
                    expr: *inner,
                    context_sheet,
                    lookup_scope,
                    local_names,
                    chain_depth,
                });
                Ok(None)
            }
            Expr::Array(rows) => {
                Self::push_validation_exprs(
                    tasks,
                    rows.into_iter().flatten().collect(),
                    context_sheet,
                    lookup_scope,
                    &local_names,
                    chain_depth,
                );
                Ok(None)
            }
            Expr::Ref(reference) => {
                let sheet = match &reference.sheet {
                    Some(prefix) => {
                        let Some(sheet) = self.workbook.sheet_index_by_name(&prefix.name) else {
                            return Ok(Some(invalid_reference_analysis(Some(&prefix.name))));
                        };
                        Some(sheet)
                    }
                    None => context_sheet,
                };
                if let Some(sheet) = sheet
                    && resolve_reference_span(self.workbook, sheet, &reference).is_err()
                {
                    return Ok(Some(invalid_reference_analysis(
                        reference.sheet.as_ref().map(|prefix| prefix.name.as_str()),
                    )));
                }
                Ok(None)
            }
            Expr::StructuredRef(reference) => {
                if reference.table.is_some()
                    && let Err(error) =
                        validate_explicit_structured_reference_target(self.workbook, &reference)
                {
                    return Ok(Some(invalid_reference_analysis(Some(error.as_str()))));
                }
                if reference.table.is_some() && !reference.items.contains(&StructuredItem::ThisRow)
                {
                    let Some(sheet) =
                        context_sheet.or_else(|| (!self.workbook.sheets().is_empty()).then_some(0))
                    else {
                        return Ok(Some(invalid_reference_analysis(None)));
                    };
                    if let Err(error) =
                        resolve_structured_reference(self.workbook, (sheet, 1, 1), &reference)
                    {
                        return Ok(Some(invalid_reference_analysis(Some(error.as_str()))));
                    }
                }
                Ok(None)
            }
            Expr::ErrorLit(ErrorKind::Ref) => Ok(Some(invalid_reference_analysis(Some(
                ErrorKind::Ref.as_str(),
            )))),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::ExternalReference(_)
            | Expr::Name(_)
            | Expr::Missing => Ok(None),
        }
    }

    fn push_validation_exprs(
        tasks: &mut Vec<ValidationTask>,
        exprs: Vec<Expr>,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        local_names: &[String],
        chain_depth: u64,
    ) {
        for expr in exprs.into_iter().rev() {
            tasks.push(ValidationTask::Expr {
                expr,
                context_sheet,
                lookup_scope,
                local_names: local_names.to_vec(),
                chain_depth,
            });
        }
    }

    fn push_let_validation(
        tasks: &mut Vec<ValidationTask>,
        args: Vec<Expr>,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        mut local_names: Vec<String>,
        chain_depth: u64,
    ) {
        let Some((final_expr, pairs)) = args.split_last() else {
            return;
        };
        let mut exprs = Vec::new();
        for pair in pairs.chunks_exact(2) {
            exprs.push((pair[1].clone(), local_names.clone()));
            if let Expr::Name(name) = &pair[0] {
                local_names.push(canonical_local_name(name));
            }
        }
        exprs.push((final_expr.clone(), local_names));
        for (expr, local_names) in exprs.into_iter().rev() {
            tasks.push(ValidationTask::Expr {
                expr,
                context_sheet,
                lookup_scope,
                local_names,
                chain_depth,
            });
        }
    }

    fn classify_definition(
        &mut self,
        root_index: usize,
        root_sheet: Option<usize>,
    ) -> Result<Outcome, DefinedNameAnalysisError> {
        let mut tasks = vec![ClassificationTask::EnterDefinition {
            index: root_index,
            context_sheet: root_sheet,
            chain_depth: 1,
        }];
        let mut outcomes = Vec::new();
        while let Some(task) = tasks.pop() {
            self.check_cancelled()?;
            match task {
                ClassificationTask::EnterDefinition {
                    index,
                    context_sheet,
                    chain_depth,
                } => {
                    let key = (index, context_sheet);
                    if let Some(outcome) = self.classified.get(&key) {
                        outcomes.push(outcome.clone());
                        continue;
                    }
                    if chain_depth > self.options.max_name_chain_depth() {
                        return Err(DefinedNameAnalysisError::resource(
                            DefinedNameAnalysisLimitKind::NameChainDepth,
                        ));
                    }
                    if !self.classifying.insert(key) {
                        outcomes.push(Outcome::Invalid {
                            reason: DefinedNameInvalidReason::CircularReference,
                            detail: Some(self.definition(index).name().to_owned().into_boxed_str()),
                        });
                        continue;
                    }
                    let scope = self.definition(index).scope();
                    let formula = self.definition(index).formula().clone();
                    let expr = match self.parsed_expr(index)? {
                        Ok(expr) => expr,
                        Err(outcome) => {
                            self.classifying.remove(&key);
                            outcomes.push(outcome);
                            continue;
                        }
                    };
                    tasks.push(ClassificationTask::FinishDefinition { key });
                    tasks.push(ClassificationTask::Expr {
                        expr,
                        formula,
                        context_sheet,
                        lookup_scope: scope,
                        chain_depth,
                    });
                }
                ClassificationTask::FinishDefinition { key } => {
                    let outcome = outcomes
                        .last()
                        .expect("definition expression produces one outcome")
                        .clone();
                    self.classifying.remove(&key);
                    self.classified.insert(key, outcome);
                }
                ClassificationTask::Expr {
                    expr,
                    formula,
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                } => {
                    self.classify_expr_task(
                        expr,
                        formula,
                        context_sheet,
                        lookup_scope,
                        chain_depth,
                        &mut tasks,
                        &mut outcomes,
                    )?;
                }
                ClassificationTask::CombineUnion { formula } => {
                    let right = outcomes.pop().expect("right union operand is classified");
                    let left = outcomes.pop().expect("left union operand is classified");
                    outcomes.push(self.union(left, right, &formula)?);
                }
                ClassificationTask::CombineIntersection { formula } => {
                    let right = outcomes
                        .pop()
                        .expect("right intersection operand is classified");
                    let left = outcomes
                        .pop()
                        .expect("left intersection operand is classified");
                    outcomes.push(self.intersection(left, right, &formula)?);
                }
                ClassificationTask::CombineRange { formula } => {
                    let end = outcomes.pop().expect("range end is classified");
                    let start = outcomes.pop().expect("range start is classified");
                    outcomes.push(self.range(start, end, &formula)?);
                }
            }
        }
        assert_eq!(
            outcomes.len(),
            1,
            "classification task stack must produce exactly one result"
        );
        Ok(outcomes.pop().expect("one classification result exists"))
    }

    #[allow(clippy::too_many_arguments)]
    fn classify_expr_task(
        &mut self,
        expr: Expr,
        formula: FormulaText,
        context_sheet: Option<usize>,
        lookup_scope: DefinedNameScope,
        chain_depth: u64,
        tasks: &mut Vec<ClassificationTask>,
        outcomes: &mut Vec<Outcome>,
    ) -> Result<(), DefinedNameAnalysisError> {
        self.charge_scan_node()?;
        match expr {
            Expr::Paren(inner) => tasks.push(ClassificationTask::Expr {
                expr: *inner,
                formula,
                context_sheet,
                lookup_scope,
                chain_depth,
            }),
            Expr::Ref(reference) => {
                let sheet = match &reference.sheet {
                    Some(prefix) => {
                        let Some(sheet) = self.workbook.sheet_index_by_name(&prefix.name) else {
                            outcomes.push(invalid_reference(Some(&prefix.name)));
                            return Ok(());
                        };
                        sheet
                    }
                    None => {
                        let Some(sheet) = context_sheet else {
                            outcomes.push(context_dependent());
                            return Ok(());
                        };
                        sheet
                    }
                };
                outcomes.push(
                    resolve_reference_span(self.workbook, sheet, &reference)
                        .map(ReferenceValue::from_span)
                        .map(Outcome::Static)
                        .unwrap_or_else(|_| {
                            invalid_reference(
                                reference.sheet.as_ref().map(|prefix| prefix.name.as_str()),
                            )
                        }),
                );
            }
            Expr::StructuredRef(reference) => {
                if reference.table.is_none() || reference.items.contains(&StructuredItem::ThisRow) {
                    outcomes.push(context_dependent());
                    return Ok(());
                }
                let context = (context_sheet.unwrap_or(0), 1, 1);
                outcomes.push(
                    match resolve_structured_reference(self.workbook, context, &reference) {
                        Ok(reference) => Outcome::Static(reference),
                        Err(error) => invalid_reference(Some(error.as_str())),
                    },
                );
            }
            Expr::ReferenceUnion { left, right } => {
                tasks.push(ClassificationTask::CombineUnion {
                    formula: formula.clone(),
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *right,
                    formula: formula.clone(),
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *left,
                    formula,
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
            }
            Expr::ReferenceIntersection { left, right } => {
                tasks.push(ClassificationTask::CombineIntersection {
                    formula: formula.clone(),
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *right,
                    formula: formula.clone(),
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *left,
                    formula,
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
            }
            Expr::Range { start, end } => {
                tasks.push(ClassificationTask::CombineRange {
                    formula: formula.clone(),
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *end,
                    formula: formula.clone(),
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
                tasks.push(ClassificationTask::Expr {
                    expr: *start,
                    formula,
                    context_sheet,
                    lookup_scope,
                    chain_depth,
                });
            }
            Expr::SpillRef(_) => outcomes.push(Outcome::Dynamic {
                kind: DefinedNameDynamicKind::Spill,
                formula,
            }),
            Expr::ExternalReference(reference) => {
                outcomes.push(Outcome::External(external_reference(&reference)));
            }
            Expr::QualifiedName { sheet, name } => {
                if sheet.end_name.is_some() {
                    outcomes.push(Outcome::Unsupported {
                        reason: DefinedNameUnsupportedReason::UnsupportedExpression,
                        detail: Some(
                            sheet
                                .sheet_range_detail()
                                .unwrap_or_default()
                                .into_boxed_str(),
                        ),
                    });
                    return Ok(());
                }
                let Some(sheet_index) = self.workbook.sheet_index_by_name(&sheet.name) else {
                    outcomes.push(invalid_reference(Some(&sheet.name)));
                    return Ok(());
                };
                let scope = DefinedNameScope::Sheet(self.workbook.sheets()[sheet_index].id());
                let Some((index, _)) = self.lookup_name(Some(sheet_index), Some(scope), &name)
                else {
                    outcomes.push(Outcome::Invalid {
                        reason: DefinedNameInvalidReason::UnresolvedName,
                        detail: Some(name.to_string().into_boxed_str()),
                    });
                    return Ok(());
                };
                tasks.push(ClassificationTask::EnterDefinition {
                    index,
                    context_sheet: Some(sheet_index),
                    chain_depth: chain_depth + 1,
                });
            }
            Expr::Name(name) => {
                let Some((index, _)) = self.lookup_name(context_sheet, Some(lookup_scope), &name)
                else {
                    outcomes.push(Outcome::Invalid {
                        reason: DefinedNameInvalidReason::UnresolvedName,
                        detail: Some(name.into_boxed_str()),
                    });
                    return Ok(());
                };
                tasks.push(ClassificationTask::EnterDefinition {
                    index,
                    context_sheet,
                    chain_depth: chain_depth + 1,
                });
            }
            Expr::Call { name, .. } => {
                if self
                    .lookup_name(context_sheet, Some(lookup_scope), &name)
                    .is_some()
                {
                    outcomes.push(non_reference(&formula));
                    return Ok(());
                }
                match function_dependency_kind(&name) {
                    Some(DependencyKind::DynamicReference(DynamicReferenceKind::Offset)) => {
                        outcomes.push(Outcome::Dynamic {
                            kind: DefinedNameDynamicKind::Offset,
                            formula,
                        })
                    }
                    Some(DependencyKind::DynamicReference(DynamicReferenceKind::Indirect)) => {
                        outcomes.push(Outcome::Dynamic {
                            kind: DefinedNameDynamicKind::Indirect,
                            formula,
                        })
                    }
                    _ if matches!(
                        function_evaluator(&name),
                        Some(Evaluator::Legacy(LegacyFunction::Index))
                            | Some(Evaluator::Dynamic(DynamicFunction::Let))
                    ) =>
                    {
                        outcomes.push(Outcome::Unsupported {
                            reason: DefinedNameUnsupportedReason::ContextDependent,
                            detail: Some(formula.as_str().to_owned().into_boxed_str()),
                        })
                    }
                    _ => outcomes.push(non_reference(&formula)),
                }
            }
            expr @ (Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Array(_)) => {
                if self.constant_syntax_bounded(&expr)? {
                    outcomes.push(Outcome::Constant { formula });
                } else {
                    outcomes.push(non_reference(&formula));
                }
            }
            Expr::ImplicitIntersection(_) | Expr::Invoke { .. } | Expr::Missing => {
                outcomes.push(non_reference(&formula))
            }
        }
        Ok(())
    }

    fn constant_syntax_bounded(&mut self, expr: &Expr) -> Result<bool, DefinedNameAnalysisError> {
        let mut pending = Vec::new();
        match expr {
            Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => pending.push(inner.as_ref()),
            Expr::Binary { left, right, .. } => {
                pending.push(right.as_ref());
                pending.push(left.as_ref());
            }
            Expr::Array(rows) => pending.extend(rows.iter().flatten()),
            Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) => {}
            _ => return Ok(false),
        }
        while let Some(expr) = pending.pop() {
            self.charge_scan_node()?;
            match expr {
                Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
                    pending.push(inner.as_ref());
                }
                Expr::Binary { left, right, .. } => {
                    pending.push(right.as_ref());
                    pending.push(left.as_ref());
                }
                Expr::Array(rows) => pending.extend(rows.iter().flatten()),
                Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn union(
        &mut self,
        left: Outcome,
        right: Outcome,
        formula: &FormulaText,
    ) -> Result<Outcome, DefinedNameAnalysisError> {
        match (left, right) {
            (Outcome::Static(left), Outcome::Static(right)) => {
                self.check_area_count(left.area_count().saturating_add(right.area_count()))?;
                match union_reference_values(&left, &right) {
                    Ok(reference) => Ok(Outcome::Static(reference)),
                    Err(error) => self.reference_error_outcome(error),
                }
            }
            (Outcome::Dynamic { .. }, Outcome::Dynamic { .. }) => Ok(Outcome::Dynamic {
                kind: DefinedNameDynamicKind::Mixed,
                formula: formula.clone(),
            }),
            (Outcome::Dynamic { kind, .. }, Outcome::Static(_))
            | (Outcome::Static(_), Outcome::Dynamic { kind, .. }) => Ok(Outcome::Dynamic {
                kind,
                formula: formula.clone(),
            }),
            (external @ Outcome::External(_), _) | (_, external @ Outcome::External(_)) => {
                Ok(external)
            }
            (invalid @ Outcome::Invalid { .. }, _) | (_, invalid @ Outcome::Invalid { .. }) => {
                Ok(invalid)
            }
            (unsupported @ Outcome::Unsupported { .. }, _)
            | (_, unsupported @ Outcome::Unsupported { .. }) => Ok(unsupported),
            (Outcome::Constant { .. }, _) | (_, Outcome::Constant { .. }) => {
                Ok(invalid_reference(Some("#VALUE!")))
            }
        }
    }

    fn intersection(
        &mut self,
        left: Outcome,
        right: Outcome,
        formula: &FormulaText,
    ) -> Result<Outcome, DefinedNameAnalysisError> {
        match (left, right) {
            (Outcome::Static(left), Outcome::Static(right)) => {
                let work = match intersection_reference_work(&left, &right) {
                    Ok(work) => work,
                    Err(error) => return self.reference_error_outcome(error),
                };
                self.charge_reference_work(work)?;
                let max_areas = self.options.calculation().limits().max_reference_areas();
                let cancelled = self.cancelled;
                match intersect_reference_values(&left, &right, max_areas, || {
                    if cancelled() {
                        Err(crate::calculation::value::ErrorKind::ResourceLimit(
                            CalculationLimitKind::FunctionIterations,
                        ))
                    } else {
                        Ok(())
                    }
                }) {
                    Ok(reference) => Ok(Outcome::Static(reference)),
                    Err(error) => self.reference_error_outcome(error),
                }
            }
            (Outcome::Dynamic { .. }, Outcome::Dynamic { .. }) => Ok(Outcome::Dynamic {
                kind: DefinedNameDynamicKind::Mixed,
                formula: formula.clone(),
            }),
            (Outcome::Dynamic { kind, .. }, Outcome::Static(_))
            | (Outcome::Static(_), Outcome::Dynamic { kind, .. }) => Ok(Outcome::Dynamic {
                kind,
                formula: formula.clone(),
            }),
            (external @ Outcome::External(_), _) | (_, external @ Outcome::External(_)) => {
                Ok(external)
            }
            (invalid @ Outcome::Invalid { .. }, _) | (_, invalid @ Outcome::Invalid { .. }) => {
                Ok(invalid)
            }
            (unsupported @ Outcome::Unsupported { .. }, _)
            | (_, unsupported @ Outcome::Unsupported { .. }) => Ok(unsupported),
            _ => Ok(invalid_reference(Some("#VALUE!"))),
        }
    }

    fn range(
        &self,
        start: Outcome,
        end: Outcome,
        formula: &FormulaText,
    ) -> Result<Outcome, DefinedNameAnalysisError> {
        match (start, end) {
            (Outcome::Static(start), Outcome::Static(end)) => {
                match range_reference_rect(&start, &end) {
                    Ok(rect) => Ok(Outcome::Static(ReferenceValue::from_rect(rect))),
                    Err(error) => self.reference_error_outcome(error),
                }
            }
            (Outcome::Dynamic { .. }, Outcome::Dynamic { .. }) => Ok(Outcome::Dynamic {
                kind: DefinedNameDynamicKind::Mixed,
                formula: formula.clone(),
            }),
            (Outcome::Dynamic { kind, .. }, Outcome::Static(_))
            | (Outcome::Static(_), Outcome::Dynamic { kind, .. }) => Ok(Outcome::Dynamic {
                kind,
                formula: formula.clone(),
            }),
            (external @ Outcome::External(_), _) | (_, external @ Outcome::External(_)) => {
                Ok(external)
            }
            (invalid @ Outcome::Invalid { .. }, _) | (_, invalid @ Outcome::Invalid { .. }) => {
                Ok(invalid)
            }
            (unsupported @ Outcome::Unsupported { .. }, _)
            | (_, unsupported @ Outcome::Unsupported { .. }) => Ok(unsupported),
            _ => Ok(invalid_reference(Some("#VALUE!"))),
        }
    }

    fn reference_error_outcome(
        &self,
        error: crate::calculation::value::ErrorKind,
    ) -> Result<Outcome, DefinedNameAnalysisError> {
        match error {
            crate::calculation::value::ErrorKind::ResourceLimit(limit) => {
                if (self.cancelled)() {
                    Err(DefinedNameAnalysisError::cancelled())
                } else {
                    Err(DefinedNameAnalysisError::resource(parse_limit(limit)))
                }
            }
            error => Ok(invalid_reference(Some(error.as_str()))),
        }
    }

    fn public_result(
        &self,
        outcome: Outcome,
    ) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
        match outcome {
            Outcome::Static(reference) => self.public_reference(reference),
            Outcome::Dynamic { kind, formula } => {
                Ok(DefinedNameAnalysis::DynamicFormula { kind, formula })
            }
            Outcome::Constant { formula } => Ok(DefinedNameAnalysis::Constant { formula }),
            Outcome::External(detail) => Ok(DefinedNameAnalysis::ExternalReference { detail }),
            Outcome::Invalid { reason, detail } => {
                Ok(DefinedNameAnalysis::Invalid { reason, detail })
            }
            Outcome::Unsupported { reason, detail } => {
                Ok(DefinedNameAnalysis::Unsupported { reason, detail })
            }
        }
    }

    fn public_reference(
        &self,
        reference: ReferenceValue,
    ) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
        if reference.area_count() == 0 {
            return Ok(DefinedNameAnalysis::EmptyReference);
        }
        self.check_area_count(reference.area_count())?;
        let areas = reference
            .areas()
            .iter()
            .map(|area| self.public_area(area))
            .collect::<Result<Vec<_>, _>>()?;
        match areas.as_slice() {
            [DefinedNameReferenceArea::Rectangular { sheet_id, range }] => {
                Ok(DefinedNameAnalysis::Rectangular {
                    sheet_id: *sheet_id,
                    range: *range,
                })
            }
            [DefinedNameReferenceArea::ThreeDimensional { sheet_span, range }] => {
                Ok(DefinedNameAnalysis::ThreeDimensional {
                    sheet_span: *sheet_span,
                    range: *range,
                })
            }
            _ => Ok(DefinedNameAnalysis::NonRectangular { areas }),
        }
    }

    fn public_area(
        &self,
        area: &ReferenceArea,
    ) -> Result<DefinedNameReferenceArea, DefinedNameAnalysisError> {
        let (sheet_start, sheet_end) = area.sheet_bounds();
        let rect = area.template_rect();
        let range = CellRange::new(
            CellAddress::from_indices(rect.row_start, rect.col_start).map_err(|_| {
                DefinedNameAnalysisError::resource(DefinedNameAnalysisLimitKind::ReferenceAreas)
            })?,
            CellAddress::from_indices(rect.row_end, rect.col_end).map_err(|_| {
                DefinedNameAnalysisError::resource(DefinedNameAnalysisLimitKind::ReferenceAreas)
            })?,
        )
        .map_err(|_| {
            DefinedNameAnalysisError::resource(DefinedNameAnalysisLimitKind::ReferenceAreas)
        })?;
        let start = self.workbook.sheets()[sheet_start].id();
        let end = self.workbook.sheets()[sheet_end].id();
        if area.is_sheet_span() {
            Ok(DefinedNameReferenceArea::ThreeDimensional {
                sheet_span: DefinedNameSheetSpan::new(start, end),
                range,
            })
        } else {
            Ok(DefinedNameReferenceArea::Rectangular {
                sheet_id: start,
                range,
            })
        }
    }
}

fn parse_limit(kind: CalculationLimitKind) -> DefinedNameAnalysisLimitKind {
    match kind {
        CalculationLimitKind::FormulaTokens => DefinedNameAnalysisLimitKind::FormulaTokens,
        CalculationLimitKind::FormulaSourceBytes => {
            DefinedNameAnalysisLimitKind::FormulaSourceBytes
        }
        CalculationLimitKind::FormulaAstNodes => DefinedNameAnalysisLimitKind::FormulaAstNodes,
        CalculationLimitKind::FormulaNestingDepth => {
            DefinedNameAnalysisLimitKind::FormulaNestingDepth
        }
        CalculationLimitKind::ReferenceAreas => DefinedNameAnalysisLimitKind::ReferenceAreas,
        CalculationLimitKind::FunctionIterations => {
            DefinedNameAnalysisLimitKind::FunctionIterations
        }
        CalculationLimitKind::DependencyEdges
        | CalculationLimitKind::ArrayCells
        | CalculationLimitKind::TextBytes
        | CalculationLimitKind::LetBindings
        | CalculationLimitKind::LambdaDepth
        | CalculationLimitKind::LambdaInvocations => DefinedNameAnalysisLimitKind::FormulaAstNodes,
    }
}

fn invalid_reference(detail: Option<&str>) -> Outcome {
    Outcome::Invalid {
        reason: DefinedNameInvalidReason::InvalidReference,
        detail: detail.map(|detail| detail.to_owned().into_boxed_str()),
    }
}

fn invalid_reference_analysis(detail: Option<&str>) -> DefinedNameAnalysis {
    DefinedNameAnalysis::Invalid {
        reason: DefinedNameInvalidReason::InvalidReference,
        detail: detail.map(|detail| detail.to_owned().into_boxed_str()),
    }
}

fn context_dependent() -> Outcome {
    Outcome::Unsupported {
        reason: DefinedNameUnsupportedReason::ContextDependent,
        detail: None,
    }
}

fn non_reference(formula: &FormulaText) -> Outcome {
    Outcome::Unsupported {
        reason: DefinedNameUnsupportedReason::NonReferenceExpression,
        detail: Some(formula.as_str().to_owned().into_boxed_str()),
    }
}

fn external_reference(
    reference: &crate::calculation::ast::ExternalWorkbookReference,
) -> DefinedNameExternalReference {
    let (target, target_text) = match &reference.target {
        ExternalReferenceTarget::Reference(body) => (
            DefinedNameExternalTargetKind::Reference,
            Reference {
                sheet: None,
                body: *body,
            }
            .to_string(),
        ),
        ExternalReferenceTarget::DefinedName(name) => {
            (DefinedNameExternalTargetKind::DefinedName, name.to_string())
        }
        ExternalReferenceTarget::StructuredReference(structured) => (
            DefinedNameExternalTargetKind::StructuredReference,
            structured.to_string(),
        ),
    };
    let (locator, workbook) = external_workbook_parts(&reference.workbook);
    DefinedNameExternalReference::new(
        locator,
        workbook,
        reference.sheet.clone(),
        reference.sheet_end.clone(),
        target,
        target_text.into_boxed_str(),
    )
}

fn external_workbook_parts(workbook: &str) -> (Option<Box<str>>, Box<str>) {
    let Some(without_end) = workbook.strip_suffix(']') else {
        return (None, workbook.to_owned().into_boxed_str());
    };
    let Some(open) = without_end.rfind('[') else {
        return (None, workbook.to_owned().into_boxed_str());
    };
    let locator = &without_end[..open];
    let workbook = &without_end[open + 1..];
    (
        (!locator.is_empty()).then(|| locator.to_owned().into_boxed_str()),
        workbook.to_owned().into_boxed_str(),
    )
}
