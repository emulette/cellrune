use std::fmt::Write as _;

use super::{
    AggregateCallableCapability, AliasAdapter, BuiltinCallable, CompatibilityVersion, DESCRIPTORS,
    DependencyKind, DynamicReferenceKind, FunctionResultKind, ReferenceMetadataKind,
    SheetSpanPolicy, StoragePrefixPolicy, Volatility,
};

pub(super) fn stable_semantic_snapshot(version: CompatibilityVersion) -> String {
    let mut snapshot = String::new();
    for descriptor in DESCRIPTORS
        .iter()
        .filter(|descriptor| descriptor.minimum_version() <= version)
    {
        write!(
            snapshot,
            "name={};evaluator={};array_evaluator={};contract={};result={};sheet_span={};volatility={};dependency={};storage={};version={};catalog_array={};public={};official={};builtin={};aggregate={};aliases=",
            descriptor.canonical_name(),
            descriptor.evaluator().stable_name(),
            descriptor
                .array_evaluator()
                .map_or_else(|| "none".to_owned(), |value| value.stable_name()),
            descriptor.call_contract().stable_snapshot(),
            result_kind_name(descriptor.result_kind()),
            sheet_span_name(descriptor.sheet_span_policy()),
            volatility_name(descriptor.volatility()),
            dependency_name(descriptor.dependency_kind()),
            storage_policy_name(descriptor.storage_prefix_policy()),
            compatibility_version_name(descriptor.minimum_version()),
            bool_name(descriptor.catalog_returns_array()),
            bool_name(descriptor.is_in_public_catalog()),
            bool_name(descriptor.is_official()),
            descriptor
                .builtin_callable()
                .map_or("none", BuiltinCallable::canonical_name),
            descriptor
                .aggregate_callable()
                .map_or("none", aggregate_capability_name),
        )
        .expect("writing to String cannot fail");
        for alias in descriptor.aliases() {
            write!(
                snapshot,
                "{}:{}:{},",
                alias.name(),
                alias_adapter_name(alias.adapter()),
                bool_name(alias.is_official()),
            )
            .expect("writing to String cannot fail");
        }
        snapshot.push('\n');
    }
    snapshot
}

const fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

const fn alias_adapter_name(adapter: AliasAdapter) -> &'static str {
    match adapter {
        AliasAdapter::Canonical => "canonical",
    }
}

const fn aggregate_capability_name(capability: AggregateCallableCapability) -> &'static str {
    match capability {
        AggregateCallableCapability::Unary => "unary",
        AggregateCallableCapability::Relative => "relative",
    }
}

const fn result_kind_name(kind: FunctionResultKind) -> &'static str {
    match kind {
        FunctionResultKind::Scalar => "scalar",
        FunctionResultKind::Array => "array",
        FunctionResultKind::Reference => "reference",
        FunctionResultKind::ReferenceOrArray => "reference_or_array",
        FunctionResultKind::Callable => "callable",
        FunctionResultKind::Contextual => "contextual",
    }
}

fn sheet_span_name(policy: SheetSpanPolicy) -> String {
    match policy {
        SheetSpanPolicy::CollectAcrossSheets => "collect_across_sheets".to_owned(),
        SheetSpanPolicy::ReturnExcelError(kind) => format!("error:{}", kind.as_str()),
        SheetSpanPolicy::Unsupported => "unsupported".to_owned(),
    }
}

const fn volatility_name(volatility: Volatility) -> &'static str {
    match volatility {
        Volatility::None => "none",
        Volatility::Today => "today",
        Volatility::Now => "now",
    }
}

const fn reference_metadata_name(kind: ReferenceMetadataKind) -> &'static str {
    match kind {
        ReferenceMetadataKind::Predicate => "predicate",
        ReferenceMetadataKind::FormulaPredicate => "formula_predicate",
        ReferenceMetadataKind::FormulaText => "formula_text",
        ReferenceMetadataKind::SheetIndex => "sheet_index",
        ReferenceMetadataKind::SheetCount => "sheet_count",
        ReferenceMetadataKind::AreaCount => "area_count",
    }
}

fn dependency_name(kind: DependencyKind) -> String {
    match kind {
        DependencyKind::Standard => "standard".to_owned(),
        DependencyKind::ReferenceMetadataOnly(metadata) => {
            format!("reference_metadata:{}", reference_metadata_name(metadata))
        }
        DependencyKind::DynamicReference(DynamicReferenceKind::Indirect) => {
            "dynamic:indirect".to_owned()
        }
        DependencyKind::DynamicReference(DynamicReferenceKind::Offset) => {
            "dynamic:offset".to_owned()
        }
        DependencyKind::ResizedCriteriaValueRange => "resized_criteria_value_range".to_owned(),
    }
}

const fn storage_policy_name(policy: StoragePrefixPolicy) -> &'static str {
    match policy {
        StoragePrefixPolicy::ExcelNamespaces => "excel_namespaces",
    }
}

const fn compatibility_version_name(version: CompatibilityVersion) -> &'static str {
    match version {
        CompatibilityVersion::Baseline => "baseline",
        CompatibilityVersion::V0_1_9 => "0.1.9",
        CompatibilityVersion::V0_1_10 => "0.1.10",
        CompatibilityVersion::V0_1_11 => "0.1.11",
        CompatibilityVersion::V0_1_12 => "0.1.12",
        CompatibilityVersion::V0_1_13 => "0.1.13",
        CompatibilityVersion::V0_1_14 => "0.1.14",
        CompatibilityVersion::V0_1_15 => "0.1.15",
    }
}
