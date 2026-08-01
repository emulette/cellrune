use std::borrow::Cow;
use std::collections::BTreeSet;

use super::ast::Expr;
use super::eval::{Engine, EvalContext};
use super::lambda::{LocalNamePolicy, definition_from_args, validate_local_name};
use super::operators::element_at;
use super::runtime::Array;
use super::scope::ArrayEvaluation;
use super::sheet_span::SheetSpanPolicy;
use super::value::{ErrorKind, Value};
pub(in crate::calculation) use descriptor::FunctionResultKind;
use descriptor::{CompatibilityVersion, DependencyKind, FunctionDescriptor};
use kernel::ArrayEvaluator;
pub(in crate::calculation) use kernel::{DynamicFunction, Evaluator};

mod aggregate;
mod array;
mod calendar;
mod combinatorics;
mod contract;
mod date;
mod date_additional;
pub(super) mod descriptor;
mod dynamic;
mod engineering;
mod financial;
mod financial_additional;
mod information;
pub(in crate::calculation) mod kernel;
mod legacy;
mod logical;
mod lookup;
mod math;
mod modern_array;
mod statistical;
mod statistical_additional;
mod sum_of_squares;
mod text;
mod text_additional;
mod trigonometry;
mod util;

pub(super) use dynamic::{
    helper_array_with_trace, helper_scalar_with_trace, invoke_lambda, lambda_scope_value,
    let_reference, let_scope_value, map_scalar_with_trace, reduce_scope_value, with_let_scope,
};

struct CallArguments<'formula> {
    expressions: Cow<'formula, [Expr]>,
}

impl<'formula> CallArguments<'formula> {
    fn prepare(descriptor: FunctionDescriptor, expressions: &'formula [Expr]) -> Self {
        let contract = descriptor.call_contract();
        let mut prepared = Cow::Borrowed(expressions);
        for position in 0..prepared.len() {
            if !matches!(prepared.as_ref()[position], Expr::Missing) {
                continue;
            }
            let Some(value) = contract.default_at(position, contract::DefaultTrigger::Missing)
            else {
                continue;
            };
            if let Some(expression) = materialize_default(value, prepared.as_ref()) {
                prepared.to_mut()[position] = expression;
            }
        }
        if let Some(maximum) = contract.maximum_arity() {
            for position in prepared.len()..usize::from(maximum) {
                let Some(value) = contract.default_at(position, contract::DefaultTrigger::Absent)
                else {
                    break;
                };
                let Some(expression) = materialize_default(value, prepared.as_ref()) else {
                    break;
                };
                prepared.to_mut().push(expression);
            }
        }
        Self {
            expressions: prepared,
        }
    }

    fn as_slice(&self) -> &[Expr] {
        &self.expressions
    }
}

fn materialize_default(value: contract::ArgumentDefaultValue, args: &[Expr]) -> Option<Expr> {
    match value {
        contract::ArgumentDefaultValue::Number(number) => Some(Expr::number(number)),
        contract::ArgumentDefaultValue::Logical(logical) => Some(Expr::Logical(logical)),
        contract::ArgumentDefaultValue::NotAvailable => Some(Expr::ErrorLit(ErrorKind::NA)),
        contract::ArgumentDefaultValue::CalculationError => Some(Expr::ErrorLit(ErrorKind::Calc)),
        contract::ArgumentDefaultValue::CriteriaRange => args.first().cloned(),
        // These defaults describe evaluator context rather than a value expression. Preserving the
        // original omission lets the typed kernel distinguish an omitted argument from an explicit
        // expression with the same source value.
        contract::ArgumentDefaultValue::CallerReference
        | contract::ArgumentDefaultValue::EmptyCollection
        | contract::ArgumentDefaultValue::IndexColumn
        | contract::ArgumentDefaultValue::LinkLocation
        | contract::ArgumentDefaultValue::LookupVector
        | contract::ArgumentDefaultValue::NoPadding
        | contract::ArgumentDefaultValue::NoSheetQualifier
        | contract::ArgumentDefaultValue::NoUpperBound
        | contract::ArgumentDefaultValue::AllOccurrences
        | contract::ArgumentDefaultValue::SourceRows
        | contract::ArgumentDefaultValue::SourceColumns => None,
    }
}

pub(in crate::calculation) fn call_function_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> super::scope::ScalarEvaluation {
    if let Some(value) = callable_call_scope(engine, context, name, args) {
        return engine.scalar_from_scope(context, &value);
    }
    let Some(descriptor) = descriptor::resolve(name) else {
        return super::scope::ScalarEvaluation::untracked(Value::Error(ErrorKind::Unsupported));
    };
    if !call_shape_is_valid(descriptor, args) {
        return super::scope::ScalarEvaluation::untracked(Value::Error(ErrorKind::Value));
    }
    let prepared = CallArguments::prepare(descriptor, args);
    let args = prepared.as_slice();
    if let Some(kind) = direct_sheet_span_error(engine, context, descriptor, args) {
        return super::scope::ScalarEvaluation::untracked(Value::Error(kind));
    }
    match descriptor.evaluator() {
        Evaluator::Dynamic(kernel::DynamicFunction::Map) => {
            map_scalar_with_trace(engine, context, args)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::ByRow) => {
            helper_scalar_with_trace(engine, context, kernel::DynamicArrayFunction::ByRow, args)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::ByCol) => {
            helper_scalar_with_trace(engine, context, kernel::DynamicArrayFunction::ByCol, args)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::MakeArray) => helper_scalar_with_trace(
            engine,
            context,
            kernel::DynamicArrayFunction::MakeArray,
            args,
        ),
        Evaluator::Dynamic(kernel::DynamicFunction::Reduce) => {
            helper_scalar_with_trace(engine, context, kernel::DynamicArrayFunction::Reduce, args)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::Scan) => {
            helper_scalar_with_trace(engine, context, kernel::DynamicArrayFunction::Scan, args)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::Let) => {
            let value = let_scope_value(engine, context, args);
            engine.scalar_from_scope(context, &value)
        }
        Evaluator::Dynamic(kernel::DynamicFunction::Lambda) => {
            let value = lambda_scope_value(context, args, None);
            engine.scalar_from_scope(context, &value)
        }
        _ => super::scope::ScalarEvaluation::untracked(dispatch_scalar(
            descriptor, engine, context, args,
        )),
    }
}

fn dispatch_scalar(
    descriptor: FunctionDescriptor,
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    match descriptor.evaluator() {
        Evaluator::Legacy(function) => legacy::call_legacy(engine, context, function, args),
        Evaluator::Logical(function) => logical::call(engine, context, function, args),
        Evaluator::Aggregate(function) => aggregate::call(engine, context, function, args),
        Evaluator::Math(function) => math::call(engine, context, function, args),
        Evaluator::Trigonometry(function) => trigonometry::call(engine, context, function, args),
        Evaluator::Combinatorics(function) => combinatorics::call(engine, context, function, args),
        Evaluator::SumOfSquares(function) => sum_of_squares::call(engine, context, function, args),
        Evaluator::Engineering(function) => engineering::call(engine, context, function, args),
        Evaluator::Lookup(function) => lookup::call(engine, context, function, args),
        Evaluator::Information(function) => information::call(engine, context, function, args),
        Evaluator::Text(function) => text::call(engine, context, function, args),
        Evaluator::TextAdditional(function) => {
            text_additional::call(engine, context, function, args)
        }
        Evaluator::Date(function) => date::call(engine, context, function, args),
        Evaluator::DateAdditional(function) => {
            date_additional::call(engine, context, function, args)
        }
        Evaluator::Dynamic(function) => dynamic::call(engine, context, function, args),
        Evaluator::Array(function) => array::call_scalar(engine, context, function, args),
        Evaluator::Statistical(function) => statistical::call(engine, context, function, args),
        Evaluator::StatisticalAdditional(function) => {
            statistical_additional::call(engine, context, function, args)
        }
        Evaluator::Financial(function) => financial::call(engine, context, function, args),
        Evaluator::FinancialAdditional(function) => {
            financial_additional::call(engine, context, function, args)
        }
        Evaluator::Areas => areas(engine, context, args),
    }
}

fn areas(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [reference] = args else {
        return Value::Error(ErrorKind::Value);
    };
    match engine.resolve_reference_value_expr(context, reference) {
        Ok(super::runtime::ReferenceValue::Empty) => Value::Error(ErrorKind::Ref),
        Ok(reference) if reference.has_sheet_span() => Value::Error(ErrorKind::Value),
        Ok(reference) => Value::Number(reference.area_count() as f64),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn callable_call_scope(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<super::scope::ScopeValue> {
    if let Some(value) = context.binding(name) {
        return Some(invoke_scope_value(engine, context, value.clone(), args));
    }
    let (id, named) = engine.resolve_name_expr_with_id_in_context(context, name)?;
    Some(match super::lambda::definition(named) {
        Some(_) => {
            let closure = lambda_scope_value(context, &named_lambda_args(named), Some(id));
            invoke_scope_value(engine, context, closure, args)
        }
        None => super::scope::ScopeValue::Scalar(super::scope::ScalarEvaluation::untracked(
            Value::Error(ErrorKind::Value),
        )),
    })
}

pub(in crate::calculation) fn intrinsic_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<super::scope::ScopeValue> {
    let descriptor = descriptor::resolve(name)?;
    if !call_shape_is_valid(descriptor, args) {
        return Some(super::scope::ScopeValue::Scalar(
            super::scope::ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)),
        ));
    }
    match descriptor.evaluator() {
        Evaluator::Dynamic(kernel::DynamicFunction::Let) => {
            Some(let_scope_value(engine, context, args))
        }
        Evaluator::Dynamic(kernel::DynamicFunction::Lambda) => {
            Some(lambda_scope_value(context, args, None))
        }
        _ => None,
    }
}

pub(super) fn uses_reference_metadata_only(normalized_name: &str) -> bool {
    descriptor::descriptor(normalized_name).is_some_and(|descriptor| {
        matches!(
            descriptor.dependency_kind(),
            DependencyKind::ReferenceMetadataOnly(_)
        )
    })
}

fn named_lambda_args(expr: &Expr) -> Vec<Expr> {
    let Expr::Call { args, .. } = expr else {
        return Vec::new();
    };
    args.clone()
}

fn invoke_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    value: super::scope::ScopeValue,
    args: &[Expr],
) -> super::scope::ScopeValue {
    match value {
        super::scope::ScopeValue::Callable(closure) => {
            invoke_lambda(engine, context, &closure, args)
        }
        _ => super::scope::ScopeValue::Scalar(super::scope::ScalarEvaluation::untracked(
            Value::Error(ErrorKind::Value),
        )),
    }
}

fn direct_sheet_span_error(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    descriptor: FunctionDescriptor,
    args: &[Expr],
) -> Option<ErrorKind> {
    if matches!(
        descriptor.dependency_kind(),
        DependencyKind::ReferenceMetadataOnly(descriptor::ReferenceMetadataKind::Predicate)
    ) {
        return None;
    }
    let policy = descriptor.sheet_span_policy();
    if matches!(policy, SheetSpanPolicy::CollectAcrossSheets) {
        return None;
    }
    let layout = descriptor.call_contract().layout();
    let has_multi_sheet_argument = args
        .iter()
        .enumerate()
        .filter(|(index, arg)| {
            !is_let_expression(arg)
                && !matches!(
                    layout.mode_at(*index, args.len()),
                    Some(
                        contract::ArgumentMode::Callable
                            | contract::ArgumentMode::Deferred
                            | contract::ArgumentMode::BindingName
                    )
                )
        })
        .any(|(_, arg)| {
            engine
                .resolve_reference_value_expr(context.without_reference_work_charge(), arg)
                .is_ok_and(|reference| reference.has_sheet_span())
        });
    if !has_multi_sheet_argument {
        return None;
    }
    Some(match policy {
        SheetSpanPolicy::ReturnExcelError(kind) => kind,
        SheetSpanPolicy::Unsupported => ErrorKind::Unsupported,
        SheetSpanPolicy::CollectAcrossSheets => {
            unreachable!("collecting policies returned before argument inspection")
        }
    })
}

fn is_let_expression(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => is_let_expression(inner),
        Expr::Call { name, .. } => {
            function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Let))
        }
        _ => false,
    }
}

pub(super) fn call_function_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<ArrayEvaluation, ErrorKind>> {
    let descriptor = descriptor::resolve(name)?;
    if !call_shape_is_valid(descriptor, args) {
        return descriptor.array_evaluator().map(|_| Err(ErrorKind::Value));
    }
    let prepared = CallArguments::prepare(descriptor, args);
    let args = prepared.as_slice();
    match descriptor.array_evaluator()? {
        ArrayEvaluator::Legacy(function) => {
            legacy::call_legacy_array(engine, context, function, args)
                .map(|result| result.map(ArrayEvaluation::untracked))
        }
        ArrayEvaluator::Information(function) => {
            information::call_array(engine, context, function, args)
                .map(|result| result.map(ArrayEvaluation::untracked))
        }
        ArrayEvaluator::Elementwise(function) => Some(
            call_elementwise_array(engine, context, function.scalar_function(), args)
                .map(ArrayEvaluation::untracked),
        ),
        ArrayEvaluator::Dynamic(function) => {
            Some(helper_array_with_trace(engine, context, function, args))
        }
        ArrayEvaluator::Map => Some(dynamic::map_array_with_trace(engine, context, args)),
        ArrayEvaluator::Array(function) => {
            Some(array::call_array(engine, context, function, args).map(ArrayEvaluation::untracked))
        }
    }
}

fn call_shape_is_valid(descriptor: FunctionDescriptor, args: &[Expr]) -> bool {
    let contract = descriptor.call_contract();
    if !contract.arity().accepts(args.len()) {
        return false;
    }
    args.iter().enumerate().all(|(index, argument)| {
        let Some(mode) = contract.layout().mode_at(index, args.len()) else {
            return false;
        };
        if matches!(mode, contract::ArgumentMode::BindingName) && !matches!(argument, Expr::Name(_))
        {
            return false;
        }
        if matches!(argument, Expr::Missing) {
            let behavior = contract.missing_behavior_at(index);
            if matches!(
                mode,
                contract::ArgumentMode::Callable | contract::ArgumentMode::BindingName
            ) && matches!(behavior, contract::MissingArgumentBehavior::CoerceToBlank)
            {
                return false;
            }
        }
        true
    })
}

pub(in crate::calculation) fn function_call_shape_is_valid(name: &str, args: &[Expr]) -> bool {
    descriptor::resolve(name).is_some_and(|descriptor| call_shape_is_valid(descriptor, args))
}

pub(in crate::calculation) fn function_arguments_are_reachable(
    name: &str,
    args: &[Expr],
    max_let_bindings: u64,
) -> bool {
    let Some(descriptor) = descriptor::resolve(name) else {
        return false;
    };
    if !call_shape_is_valid(descriptor, args) {
        return false;
    }
    match descriptor.evaluator() {
        Evaluator::Dynamic(DynamicFunction::Let) => {
            let binding_count = (args.len() - 1) / 2;
            if u64::try_from(binding_count).map_or(true, |count| count > max_let_bindings) {
                return false;
            }
            let mut names = BTreeSet::new();
            args[..args.len() - 1].chunks_exact(2).all(|pair| {
                let Expr::Name(name) = &pair[0] else {
                    return false;
                };
                validate_local_name(name, LocalNamePolicy::Let)
                    .is_some_and(|name| names.insert(name.into_string()))
            })
        }
        Evaluator::Dynamic(DynamicFunction::Lambda) => definition_from_args(args).is_some(),
        _ => true,
    }
}

pub(in crate::calculation) fn prepare_evaluator_arguments<'formula>(
    evaluator: Evaluator,
    args: &'formula [Expr],
) -> Option<Cow<'formula, [Expr]>> {
    let descriptor = descriptor::descriptors()
        .iter()
        .copied()
        .find(|descriptor| descriptor.evaluator() == evaluator)?;
    call_shape_is_valid(descriptor, args)
        .then(|| CallArguments::prepare(descriptor, args).expressions)
}

pub(super) fn is_supported_function(name: &str) -> bool {
    descriptor::resolve(name).is_some()
}

pub(in crate::calculation) fn function_evaluator(name: &str) -> Option<Evaluator> {
    descriptor::resolve(name).map(FunctionDescriptor::evaluator)
}

pub(in crate::calculation) fn function_result_kind(name: &str) -> Option<FunctionResultKind> {
    descriptor::resolve(name).map(FunctionDescriptor::result_kind)
}

pub(in crate::calculation) fn canonical_name_for_evaluator(
    evaluator: Evaluator,
) -> Option<&'static str> {
    descriptor::descriptors()
        .iter()
        .find(|descriptor| descriptor.evaluator() == evaluator)
        .map(|descriptor| descriptor.canonical_name())
}

pub(super) fn is_reference_returning_function(name: &str) -> bool {
    descriptor::resolve(name).is_some_and(|descriptor| descriptor.result_kind().returns_reference())
}

pub(super) fn descriptor_sheet_span_policy(name: &str) -> Option<SheetSpanPolicy> {
    let normalized = normalize_name(name);
    descriptor::descriptor(&normalized).map(descriptor::FunctionDescriptor::sheet_span_policy)
}

pub(super) fn function_catalog() -> Vec<super::FunctionCatalogEntry> {
    function_catalog_for_version(CompatibilityVersion::V0_1_10)
}

fn function_catalog_for_version(version: CompatibilityVersion) -> Vec<super::FunctionCatalogEntry> {
    let mut entries = descriptor::descriptors()
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.is_in_public_catalog() && descriptor.minimum_version() <= version
        })
        .flat_map(|descriptor| {
            let canonical = std::iter::once(super::FunctionCatalogEntry::new(
                descriptor.canonical_name().to_owned(),
                descriptor.canonical_name().to_owned(),
                false,
                descriptor.catalog_returns_array(),
                descriptor.is_official(),
            ));
            let aliases = descriptor.aliases().iter().map(move |alias| {
                debug_assert_eq!(alias.adapter(), descriptor::AliasAdapter::Canonical);
                super::FunctionCatalogEntry::new(
                    alias.name().to_owned(),
                    descriptor.canonical_name().to_owned(),
                    true,
                    descriptor.catalog_returns_array(),
                    alias.is_official(),
                )
            });
            canonical.chain(aliases)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name().cmp(right.name()));
    entries
}

pub(super) fn function_volatility(name: &str) -> Option<descriptor::Volatility> {
    descriptor::resolve(name).map(FunctionDescriptor::volatility)
}

pub(super) fn function_dependency_kind(name: &str) -> Option<DependencyKind> {
    descriptor::resolve(name).map(FunctionDescriptor::dependency_kind)
}

fn call_elementwise_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: kernel::MathFunction,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let arrays = args
        .iter()
        .map(|argument| engine.eval_array(context, argument))
        .collect::<Result<Vec<_>, _>>()?;
    let rows = arrays.iter().map(|array| array.rows).max().unwrap_or(1);
    let cols = arrays.iter().map(|array| array.cols).max().unwrap_or(1);
    let cells = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(
        context,
        cells
            .checked_mul(args.len().max(1) as u64)
            .ok_or(ErrorKind::Num)?,
    )?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for column in 0..cols {
            let scalar_args = arrays
                .iter()
                .map(|array| value_as_expr(element_at(array, row, column)))
                .collect::<Vec<_>>();
            data.push(math::call(engine, context, function, &scalar_args));
        }
    }
    Ok(Array { rows, cols, data })
}

fn value_as_expr(value: &Value) -> Expr {
    match value {
        Value::Blank => Expr::Missing,
        Value::Number(number) => Expr::number(*number),
        Value::Text(text) => Expr::Text(text.clone()),
        Value::Logical(value) => Expr::Logical(*value),
        Value::Error(kind) => Expr::ErrorLit(*kind),
    }
}

pub(super) fn normalize_name(name: &str) -> String {
    descriptor::normalize_name(name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use sha2::{Digest, Sha256};

    use super::{Expr, descriptor, normalize_name};

    #[test]
    fn descriptor_contract_prepares_defaults_and_enforces_boundaries() {
        let if_descriptor = descriptor::descriptor("IF").expect("IF descriptor");
        let if_args = vec![Expr::Logical(true), Expr::number(7.0)];
        let prepared = super::CallArguments::prepare(if_descriptor, &if_args);
        assert_eq!(prepared.as_slice().len(), 3);
        assert_eq!(prepared.as_slice()[2], Expr::Logical(false));

        let sumif_descriptor = descriptor::descriptor("SUMIF").expect("SUMIF descriptor");
        let criteria_range = Expr::Name("criteria_range".to_owned());
        let sumif_args = vec![criteria_range.clone(), Expr::Text(">0".to_owned())];
        let prepared = super::CallArguments::prepare(sumif_descriptor, &sumif_args);
        assert_eq!(prepared.as_slice()[2], criteria_range);

        let address_descriptor = descriptor::descriptor("ADDRESS").expect("ADDRESS descriptor");
        let address_args = vec![Expr::number(1.0), Expr::number(1.0)];
        let prepared = super::CallArguments::prepare(address_descriptor, &address_args);
        assert_eq!(prepared.as_slice().len(), 4);
        assert_eq!(prepared.as_slice()[2], Expr::number(1.0));
        assert_eq!(prepared.as_slice()[3], Expr::Logical(true));

        let indirect_descriptor = descriptor::descriptor("INDIRECT").expect("INDIRECT descriptor");
        let absent_style = vec![Expr::Text("A1".to_owned())];
        let prepared = super::CallArguments::prepare(indirect_descriptor, &absent_style);
        assert_eq!(prepared.as_slice()[1], Expr::Logical(true));
        let explicit_missing = vec![Expr::Text("A1".to_owned()), Expr::Missing];
        let prepared = super::CallArguments::prepare(indirect_descriptor, &explicit_missing);
        assert_eq!(prepared.as_slice()[1], Expr::Missing);

        let hyperlink = descriptor::descriptor("HYPERLINK").expect("HYPERLINK descriptor");
        let hyperlink_args = vec![Expr::number(123.0)];
        let prepared = super::CallArguments::prepare(hyperlink, &hyperlink_args);
        assert_eq!(prepared.as_slice(), hyperlink_args);

        let lookup = descriptor::descriptor("LOOKUP").expect("LOOKUP descriptor");
        let lookup_args = vec![Expr::number(2.0), Expr::Name("lookup_array".to_owned())];
        let prepared = super::CallArguments::prepare(lookup, &lookup_args);
        assert_eq!(prepared.as_slice(), lookup_args);

        let index = descriptor::descriptor("INDEX").expect("INDEX descriptor");
        assert!(super::call_shape_is_valid(
            index,
            &[
                Expr::Name("areas".to_owned()),
                Expr::number(1.0),
                Expr::number(1.0),
                Expr::number(2.0),
            ],
        ));
        let count_blank = descriptor::descriptor("COUNTBLANK").expect("COUNTBLANK descriptor");
        assert!(!super::call_shape_is_valid(
            count_blank,
            &[
                Expr::Name("left".to_owned()),
                Expr::Name("right".to_owned())
            ],
        ));
        let and_function = descriptor::descriptor("AND").expect("AND descriptor");
        assert!(!super::call_shape_is_valid(
            and_function,
            &vec![Expr::Logical(true); 256],
        ));

        for (name, maximum) in [
            ("SWITCH", 254_usize),
            ("CONCAT", 253),
            ("TEXTJOIN", 254),
            ("MAXIFS", 253),
            ("MINIFS", 253),
        ] {
            let descriptor = descriptor::descriptor(name).expect("registered descriptor");
            assert!(
                super::call_shape_is_valid(descriptor, &vec![Expr::number(1.0); maximum]),
                "{name} must accept its documented maximum",
            );
            assert!(
                !super::call_shape_is_valid(descriptor, &vec![Expr::number(1.0); maximum + 1]),
                "{name} must reject arguments above its documented maximum",
            );
        }
    }

    #[test]
    fn normalization_removes_composed_excel_storage_prefixes() {
        assert_eq!(normalize_name("_xlfn._xlws.FILTER"), "FILTER");
        assert_eq!(normalize_name("_xludf._xlfn.COVAR"), "COVARIANCE.P");
        assert_eq!(normalize_name("_XLWS._XLUDF._XLFN.SUM"), "SUM");
    }

    #[test]
    fn coverage_registry_has_287_unique_excel_facing_names() {
        let kernels: BTreeSet<_> = descriptor::descriptors()
            .iter()
            .map(|descriptor| descriptor.canonical_name())
            .collect();
        assert_eq!(kernels.len(), descriptor::descriptors().len());
        assert!(kernels.contains("__XLUDF.DUMMYFUNCTION"));
        assert_eq!(kernels.len(), 275);

        let aliases = descriptor::descriptors()
            .iter()
            .flat_map(|descriptor| descriptor.aliases())
            .map(|alias| alias.name())
            .collect::<BTreeSet<_>>();
        let alias_count = descriptor::descriptors()
            .iter()
            .map(|descriptor| descriptor.aliases().len())
            .sum::<usize>();
        assert_eq!(aliases.len(), alias_count);
        assert_eq!(aliases.len(), 13);
        assert!(aliases.is_disjoint(&kernels));
        assert!(
            descriptor::descriptors()
                .iter()
                .flat_map(|descriptor| descriptor.aliases())
                .all(|alias| descriptor::resolve(alias.name()).is_some())
        );

        let catalog = super::function_catalog();
        assert_eq!(catalog.len(), kernels.len() + aliases.len());
        assert_eq!(catalog.len(), 288);
        assert_eq!(
            catalog.iter().filter(|entry| entry.is_official()).count(),
            287
        );
        assert!(
            catalog
                .windows(2)
                .all(|pair| pair[0].name() < pair[1].name())
        );
        assert_eq!(
            catalog
                .iter()
                .filter(|entry| entry.name() == "AREAS")
                .count(),
            1
        );
        assert_eq!(
            catalog
                .iter()
                .find(|entry| entry.name() == "COVAR")
                .map(|entry| entry.canonical_name()),
            Some("COVARIANCE.P")
        );
    }

    #[test]
    fn migrated_catalog_is_byte_exact_with_the_v0_1_9_snapshot() {
        let mut digest = Sha256::new();
        for entry in
            super::function_catalog_for_version(super::descriptor::CompatibilityVersion::V0_1_9)
        {
            digest.update(entry.name().as_bytes());
            digest.update([0]);
            digest.update(entry.canonical_name().as_bytes());
            digest.update([0]);
            digest.update(if entry.is_alias() { b"1" } else { b"0" });
            digest.update([0]);
            digest.update(if entry.returns_array() { b"1" } else { b"0" });
            digest.update([0]);
            digest.update(if entry.is_official() { b"1" } else { b"0" });
            digest.update(b"\n");
        }
        let actual: [u8; 32] = digest.finalize().into();
        assert_eq!(
            actual,
            [
                0xd0, 0xa5, 0x38, 0x20, 0x7e, 0x53, 0x6d, 0x3c, 0x5b, 0x52, 0xe2, 0xae, 0x1c, 0x33,
                0x53, 0xcf, 0xef, 0x3e, 0xe9, 0x65, 0xb8, 0xea, 0x84, 0x1c, 0x14, 0x1b, 0xf2, 0x0a,
                0x6c, 0x12, 0xd9, 0xae,
            ]
        );
    }
}
