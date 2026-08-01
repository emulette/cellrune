use super::ast::Expr;
use super::eval::{Engine, EvalContext};
use super::operators::element_at;
use super::runtime::Array;
use super::scope::ArrayEvaluation;
use super::sheet_span::SheetSpanPolicy;
use super::value::{ErrorKind, Value};
use descriptor::{
    ArgumentPolicy, ArrayEvaluator, CompatibilityVersion, DependencyKind, Evaluator,
    FunctionDescriptor,
};

mod aggregate;
mod array;
mod calendar;
mod combinatorics;
mod date;
mod date_additional;
pub(super) mod descriptor;
mod dynamic;
mod engineering;
mod financial;
mod financial_additional;
mod information;
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

pub(super) fn call_function(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    let normalized = normalize_name(name);
    if let Some(value) = callable_call_scope(engine, context, name, args) {
        return scope_value_to_scalar(engine, context, value);
    }
    let Some(descriptor) = descriptor::descriptor(&normalized) else {
        return Value::Error(ErrorKind::Unsupported);
    };
    if let Some(kind) = direct_sheet_span_error(engine, context, descriptor, args) {
        return Value::Error(kind);
    }
    match descriptor.argument_policy() {
        ArgumentPolicy::EvaluatorManaged => {
            dispatch_scalar(descriptor, engine, context, &normalized, args)
        }
    }
}

fn dispatch_scalar(
    descriptor: FunctionDescriptor,
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    canonical_name: &str,
    args: &[Expr],
) -> Value {
    match descriptor.evaluator() {
        Evaluator::Legacy => legacy::call_legacy(engine, context, canonical_name, args),
        Evaluator::Logical => logical::call(engine, context, canonical_name, args),
        Evaluator::Aggregate => aggregate::call(engine, context, canonical_name, args),
        Evaluator::Math => math::call(engine, context, canonical_name, args),
        Evaluator::Trigonometry => trigonometry::call(engine, context, canonical_name, args),
        Evaluator::Combinatorics => combinatorics::call(engine, context, canonical_name, args),
        Evaluator::SumOfSquares => sum_of_squares::call(engine, context, canonical_name, args),
        Evaluator::Engineering => engineering::call(engine, context, canonical_name, args),
        Evaluator::Lookup => lookup::call(engine, context, canonical_name, args),
        Evaluator::Information => information::call(engine, context, canonical_name, args),
        Evaluator::Text => text::call(engine, context, canonical_name, args),
        Evaluator::TextAdditional => text_additional::call(engine, context, canonical_name, args),
        Evaluator::Date => date::call(engine, context, canonical_name, args),
        Evaluator::DateAdditional => date_additional::call(engine, context, canonical_name, args),
        Evaluator::Dynamic => dynamic::call(engine, context, canonical_name, args),
        Evaluator::Array => array::call_scalar(engine, context, canonical_name, args),
        Evaluator::Statistical => statistical::call(engine, context, canonical_name, args),
        Evaluator::StatisticalAdditional => {
            statistical_additional::call(engine, context, canonical_name, args)
        }
        Evaluator::Financial => financial::call(engine, context, canonical_name, args),
        Evaluator::FinancialAdditional => {
            financial_additional::call(engine, context, canonical_name, args)
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

fn scope_value_to_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    value: super::scope::ScopeValue,
) -> Value {
    match value {
        super::scope::ScopeValue::Callable(_) => Value::Error(ErrorKind::Calc),
        value => engine.scalar_from_scope(context, &value).value,
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
    let has_multi_sheet_argument = args
        .iter()
        .filter(|arg| !is_let_expression(arg))
        .any(|arg| {
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
        Expr::Call { name, .. } => normalize_name(name) == "LET",
        _ => false,
    }
}

pub(super) fn call_function_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<ArrayEvaluation, ErrorKind>> {
    let normalized = normalize_name(name);
    let descriptor = descriptor::descriptor(&normalized)?;
    match descriptor.array_evaluator()? {
        ArrayEvaluator::Legacy => legacy::call_legacy_array(engine, context, &normalized, args)
            .map(|result| result.map(ArrayEvaluation::untracked)),
        ArrayEvaluator::Information => information::call_array(engine, context, &normalized, args)
            .map(|result| result.map(ArrayEvaluation::untracked)),
        ArrayEvaluator::Elementwise => Some(
            call_elementwise_array(engine, context, &normalized, args)
                .map(ArrayEvaluation::untracked),
        ),
        ArrayEvaluator::DynamicHelper => {
            helper_array_with_trace(engine, context, &normalized, args)
        }
        ArrayEvaluator::Map => Some(dynamic::map_array_with_trace(engine, context, args)),
        ArrayEvaluator::Array => Some(
            array::call_array(engine, context, &normalized, args).map(ArrayEvaluation::untracked),
        ),
    }
}

pub(super) fn is_supported_function(name: &str) -> bool {
    descriptor::resolve(name).is_some()
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
        .filter(|descriptor| descriptor.minimum_version() <= version)
        .flat_map(|descriptor| {
            let canonical = std::iter::once(super::FunctionCatalogEntry::new(
                descriptor.canonical_name().to_owned(),
                descriptor.canonical_name().to_owned(),
                false,
                descriptor.result_kind().returns_array(),
                descriptor.is_official(),
            ));
            let aliases = descriptor.aliases().iter().map(move |alias| {
                debug_assert_eq!(alias.adapter(), descriptor::AliasAdapter::Canonical);
                super::FunctionCatalogEntry::new(
                    alias.name().to_owned(),
                    descriptor.canonical_name().to_owned(),
                    true,
                    descriptor.result_kind().returns_array(),
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
    name: &str,
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
            data.push(call_function(engine, context, name, &scalar_args));
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

    use super::{descriptor, normalize_name};

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
