use std::collections::HashMap;
use std::sync::OnceLock;

use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FunctionId(&'static str);

impl FunctionId {
    const fn new(canonical_name: &'static str) -> Self {
        Self(canonical_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Evaluator {
    Legacy,
    Logical,
    Aggregate,
    Math,
    Trigonometry,
    Combinatorics,
    SumOfSquares,
    Engineering,
    Lookup,
    Information,
    Text,
    TextAdditional,
    Date,
    DateAdditional,
    Dynamic,
    Array,
    Statistical,
    StatisticalAdditional,
    Financial,
    FinancialAdditional,
    Areas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArrayEvaluator {
    Legacy,
    Information,
    Elementwise,
    DynamicHelper,
    Map,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FunctionResultKind {
    Scalar,
    Array,
    Reference,
    ReferenceOrArray,
    Dynamic,
}

impl FunctionResultKind {
    pub(super) const fn returns_array(self) -> bool {
        matches!(self, Self::Array | Self::ReferenceOrArray | Self::Dynamic)
    }

    pub(super) const fn returns_reference(self) -> bool {
        matches!(self, Self::Reference | Self::ReferenceOrArray)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArgumentPolicy {
    EvaluatorManaged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AliasAdapter {
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FunctionAlias {
    name: &'static str,
    adapter: AliasAdapter,
    official: bool,
}

impl FunctionAlias {
    const fn official(name: &'static str) -> Self {
        Self {
            name,
            adapter: AliasAdapter::Canonical,
            official: true,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        self.name
    }

    pub(super) const fn adapter(self) -> AliasAdapter {
        self.adapter
    }

    pub(super) const fn is_official(self) -> bool {
        self.official
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum Volatility {
    None,
    Today,
    Now,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum DynamicReferenceKind {
    Indirect,
    Offset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum ReferenceMetadataKind {
    Predicate,
    AreaCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum DependencyKind {
    Standard,
    ReferenceMetadataOnly(ReferenceMetadataKind),
    DynamicReference(DynamicReferenceKind),
    ResizedCriteriaValueRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CompatibilityVersion {
    Baseline,
    V0_1_9,
    V0_1_10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoragePrefixPolicy {
    ExcelNamespaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FunctionDescriptor {
    id: FunctionId,
    canonical_name: &'static str,
    aliases: &'static [FunctionAlias],
    evaluator: Evaluator,
    array_evaluator: Option<ArrayEvaluator>,
    argument_policy: ArgumentPolicy,
    result_kind: FunctionResultKind,
    sheet_span_policy: SheetSpanPolicy,
    volatility: Volatility,
    dependency_kind: DependencyKind,
    storage_prefix_policy: StoragePrefixPolicy,
    minimum_version: CompatibilityVersion,
    official: bool,
}

impl FunctionDescriptor {
    const fn new(canonical_name: &'static str, evaluator: Evaluator) -> Self {
        Self {
            id: FunctionId::new(canonical_name),
            canonical_name,
            aliases: &[],
            evaluator,
            array_evaluator: None,
            argument_policy: ArgumentPolicy::EvaluatorManaged,
            result_kind: FunctionResultKind::Scalar,
            sheet_span_policy: SheetSpanPolicy::Unsupported,
            volatility: Volatility::None,
            dependency_kind: DependencyKind::Standard,
            storage_prefix_policy: StoragePrefixPolicy::ExcelNamespaces,
            minimum_version: CompatibilityVersion::Baseline,
            official: true,
        }
    }

    const fn with_aliases(mut self, aliases: &'static [FunctionAlias]) -> Self {
        self.aliases = aliases;
        self
    }

    const fn with_array_evaluator(mut self, evaluator: ArrayEvaluator) -> Self {
        self.array_evaluator = Some(evaluator);
        self.result_kind = FunctionResultKind::Array;
        self
    }

    const fn with_dynamic_result(mut self, evaluator: Option<ArrayEvaluator>) -> Self {
        self.array_evaluator = evaluator;
        self.result_kind = FunctionResultKind::Dynamic;
        self
    }

    const fn with_reference_result(mut self) -> Self {
        self.result_kind = FunctionResultKind::Reference;
        self
    }

    const fn with_reference_array_result(mut self, evaluator: ArrayEvaluator) -> Self {
        self.array_evaluator = Some(evaluator);
        self.result_kind = FunctionResultKind::ReferenceOrArray;
        self
    }

    const fn with_sheet_span_policy(mut self, policy: SheetSpanPolicy) -> Self {
        self.sheet_span_policy = policy;
        self
    }

    const fn with_volatility(mut self, volatility: Volatility) -> Self {
        self.volatility = volatility;
        self
    }

    const fn with_dependency_kind(mut self, kind: DependencyKind) -> Self {
        self.dependency_kind = kind;
        self
    }

    const fn with_minimum_version(mut self, version: CompatibilityVersion) -> Self {
        self.minimum_version = version;
        self
    }

    const fn unofficial(mut self) -> Self {
        self.official = false;
        self
    }

    pub(super) const fn id(self) -> FunctionId {
        self.id
    }

    pub(super) const fn canonical_name(self) -> &'static str {
        self.canonical_name
    }

    pub(super) const fn aliases(self) -> &'static [FunctionAlias] {
        self.aliases
    }

    pub(super) const fn evaluator(self) -> Evaluator {
        self.evaluator
    }

    pub(super) const fn array_evaluator(self) -> Option<ArrayEvaluator> {
        self.array_evaluator
    }

    pub(super) const fn argument_policy(self) -> ArgumentPolicy {
        self.argument_policy
    }

    pub(super) const fn result_kind(self) -> FunctionResultKind {
        self.result_kind
    }

    pub(super) const fn sheet_span_policy(self) -> SheetSpanPolicy {
        self.sheet_span_policy
    }

    pub(super) const fn volatility(self) -> Volatility {
        self.volatility
    }

    pub(super) const fn dependency_kind(self) -> DependencyKind {
        self.dependency_kind
    }

    pub(super) const fn storage_prefix_policy(self) -> StoragePrefixPolicy {
        self.storage_prefix_policy
    }

    pub(super) const fn minimum_version(self) -> CompatibilityVersion {
        self.minimum_version
    }

    pub(super) const fn is_official(self) -> bool {
        self.official
    }
}

macro_rules! function {
    ($name:literal, $evaluator:ident) => {
        FunctionDescriptor::new($name, Evaluator::$evaluator)
    };
}

const COLLECT_ACROSS_SHEETS: SheetSpanPolicy = SheetSpanPolicy::CollectAcrossSheets;
const VALUE_ON_SHEET_SPAN: SheetSpanPolicy = SheetSpanPolicy::ReturnExcelError(ErrorKind::Value);
const REF_ON_SHEET_SPAN: SheetSpanPolicy = SheetSpanPolicy::ReturnExcelError(ErrorKind::Ref);

// This is the sole inventory of supported functions and accepted public aliases. Keep behavior
// flags beside each canonical registration so evaluation, capability analysis and catalog
// serialization cannot drift into independent name lists.
const DESCRIPTORS: &[FunctionDescriptor] = &[
    function!("IF", Legacy).with_array_evaluator(ArrayEvaluator::Legacy),
    function!("AND", Legacy),
    function!("IFERROR", Legacy),
    function!("LOWER", Legacy),
    function!("TEXT", Legacy),
    function!("COUNTIF", Legacy).with_array_evaluator(ArrayEvaluator::Legacy),
    function!("COUNTIFS", Legacy).with_array_evaluator(ArrayEvaluator::Legacy),
    function!("SUMPRODUCT", Legacy),
    function!("INDEX", Legacy)
        .with_reference_array_result(ArrayEvaluator::Legacy)
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!("MATCH", Legacy),
    function!("__XLUDF.DUMMYFUNCTION", Legacy).unofficial(),
    function!("TRUE", Logical),
    function!("FALSE", Logical),
    function!("NOT", Logical),
    function!("OR", Logical),
    function!("XOR", Logical),
    function!("IFNA", Logical),
    function!("IFS", Logical),
    function!("SWITCH", Logical),
    function!("SUM", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("AVERAGE", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("MIN", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("MAX", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("COUNT", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("COUNTA", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("COUNTBLANK", Aggregate),
    function!("PRODUCT", Aggregate).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("SUBTOTAL", Aggregate),
    function!("SUMIF", Aggregate).with_dependency_kind(DependencyKind::ResizedCriteriaValueRange),
    function!("SUMIFS", Aggregate),
    function!("AVERAGEIF", Aggregate)
        .with_dependency_kind(DependencyKind::ResizedCriteriaValueRange),
    function!("AVERAGEIFS", Aggregate),
    function!("ABS", Math).with_array_evaluator(ArrayEvaluator::Elementwise),
    function!("BASE", Math),
    function!("CEILING", Math),
    function!("CEILING.MATH", Math),
    function!("CEILING.PRECISE", Math),
    function!("DECIMAL", Math),
    function!("EVEN", Math),
    function!("EXP", Math),
    function!("FLOOR", Math),
    function!("FLOOR.MATH", Math),
    function!("FLOOR.PRECISE", Math),
    function!("INT", Math),
    function!("ISO.CEILING", Math),
    function!("LN", Math),
    function!("LOG", Math),
    function!("LOG10", Math),
    function!("MOD", Math),
    function!("MROUND", Math),
    function!("ODD", Math),
    function!("PI", Math),
    function!("POWER", Math),
    function!("QUOTIENT", Math),
    function!("ROUND", Math),
    function!("ROUNDDOWN", Math),
    function!("ROUNDUP", Math),
    function!("SERIESSUM", Math),
    function!("SIGN", Math),
    function!("SQRT", Math),
    function!("SQRTPI", Math),
    function!("TRUNC", Math),
    function!("ACOS", Trigonometry),
    function!("ACOSH", Trigonometry),
    function!("ACOT", Trigonometry),
    function!("ACOTH", Trigonometry),
    function!("ASIN", Trigonometry),
    function!("ASINH", Trigonometry),
    function!("ATAN", Trigonometry),
    function!("ATAN2", Trigonometry),
    function!("ATANH", Trigonometry),
    function!("COS", Trigonometry),
    function!("COSH", Trigonometry),
    function!("COT", Trigonometry),
    function!("COTH", Trigonometry),
    function!("CSC", Trigonometry),
    function!("CSCH", Trigonometry),
    function!("DEGREES", Trigonometry),
    function!("RADIANS", Trigonometry),
    function!("SEC", Trigonometry),
    function!("SECH", Trigonometry),
    function!("SIN", Trigonometry),
    function!("SINH", Trigonometry),
    function!("TAN", Trigonometry),
    function!("TANH", Trigonometry),
    function!("COMBIN", Combinatorics),
    function!("COMBINA", Combinatorics),
    function!("FACT", Combinatorics),
    function!("FACTDOUBLE", Combinatorics),
    function!("GCD", Combinatorics),
    function!("LCM", Combinatorics),
    function!("MULTINOMIAL", Combinatorics),
    function!("PERMUT", Combinatorics),
    function!("PERMUTATIONA", Combinatorics),
    function!("SUMSQ", SumOfSquares),
    function!("SUMX2MY2", SumOfSquares),
    function!("SUMX2PY2", SumOfSquares),
    function!("SUMXMY2", SumOfSquares),
    function!("BIN2DEC", Engineering),
    function!("BIN2HEX", Engineering),
    function!("BIN2OCT", Engineering),
    function!("BITAND", Engineering),
    function!("BITLSHIFT", Engineering),
    function!("BITOR", Engineering),
    function!("BITRSHIFT", Engineering),
    function!("BITXOR", Engineering),
    function!("DEC2BIN", Engineering),
    function!("DEC2HEX", Engineering),
    function!("DEC2OCT", Engineering),
    function!("DELTA", Engineering),
    function!("ERF", Engineering),
    function!("ERF.PRECISE", Engineering),
    function!("ERFC", Engineering),
    function!("ERFC.PRECISE", Engineering),
    function!("GESTEP", Engineering),
    function!("HEX2BIN", Engineering),
    function!("HEX2DEC", Engineering),
    function!("HEX2OCT", Engineering),
    function!("OCT2BIN", Engineering),
    function!("OCT2DEC", Engineering),
    function!("OCT2HEX", Engineering),
    function!("ADDRESS", Lookup),
    function!("CHOOSE", Lookup),
    function!("COLUMN", Lookup),
    function!("COLUMNS", Lookup),
    function!("HLOOKUP", Lookup),
    function!("HYPERLINK", Lookup),
    function!("INDIRECT", Lookup)
        .with_reference_result()
        .with_dependency_kind(DependencyKind::DynamicReference(
            DynamicReferenceKind::Indirect,
        )),
    function!("LOOKUP", Lookup),
    function!("OFFSET", Lookup)
        .with_reference_result()
        .with_sheet_span_policy(REF_ON_SHEET_SPAN)
        .with_dependency_kind(DependencyKind::DynamicReference(
            DynamicReferenceKind::Offset,
        )),
    function!("ROWS", Lookup),
    function!("ROW", Lookup),
    function!("VLOOKUP", Lookup).with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!("XLOOKUP", Lookup),
    function!("ERROR.TYPE", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISBLANK", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISERR", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISERROR", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISEVEN", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISLOGICAL", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISNA", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISNONTEXT", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISNUMBER", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISODD", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("ISREF", Information)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::Predicate,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!("ISTEXT", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("N", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("NA", Information),
    function!("T", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("TYPE", Information).with_array_evaluator(ArrayEvaluator::Information),
    function!("CONCAT", Text),
    function!("EXACT", Text),
    function!("FIND", Text),
    function!("LEFT", Text),
    function!("LEN", Text),
    function!("MID", Text),
    function!("PROPER", Text),
    function!("REPLACE", Text),
    function!("REPT", Text),
    function!("RIGHT", Text),
    function!("SEARCH", Text),
    function!("SUBSTITUTE", Text),
    function!("TEXTJOIN", Text),
    function!("TRIM", Text),
    function!("UPPER", Text),
    function!("CHAR", TextAdditional),
    function!("CLEAN", TextAdditional),
    function!("CONCATENATE", TextAdditional),
    function!("DOLLAR", TextAdditional),
    function!("TEXTAFTER", TextAdditional),
    function!("TEXTBEFORE", TextAdditional),
    function!("UNICHAR", TextAdditional),
    function!("UNICODE", TextAdditional),
    function!("VALUE", TextAdditional),
    function!("VALUETOTEXT", TextAdditional),
    function!("DATE", Date),
    function!("DATEDIF", Date),
    function!("DAY", Date),
    function!("EDATE", Date),
    function!("EOMONTH", Date),
    function!("MONTH", Date),
    function!("NETWORKDAYS", Date),
    function!("NOW", Date).with_volatility(Volatility::Now),
    function!("TODAY", Date).with_volatility(Volatility::Today),
    function!("WEEKDAY", Date),
    function!("WORKDAY", Date),
    function!("YEAR", Date),
    function!("YEARFRAC", Date),
    function!("DAYS", DateAdditional),
    function!("DAYS360", DateAdditional),
    function!("HOUR", DateAdditional),
    function!("ISOWEEKNUM", DateAdditional),
    function!("MINUTE", DateAdditional),
    function!("SECOND", DateAdditional),
    function!("TIME", DateAdditional),
    function!("WEEKNUM", DateAdditional),
    function!("BYCOL", Dynamic).with_dynamic_result(Some(ArrayEvaluator::DynamicHelper)),
    function!("BYROW", Dynamic).with_dynamic_result(Some(ArrayEvaluator::DynamicHelper)),
    function!("ISOMITTED", Dynamic).with_dynamic_result(None),
    function!("LAMBDA", Dynamic).with_dynamic_result(None),
    function!("LET", Dynamic).with_dynamic_result(None),
    function!("MAKEARRAY", Dynamic).with_dynamic_result(Some(ArrayEvaluator::DynamicHelper)),
    function!("MAP", Dynamic).with_dynamic_result(Some(ArrayEvaluator::Map)),
    function!("REDUCE", Dynamic).with_dynamic_result(Some(ArrayEvaluator::DynamicHelper)),
    function!("SCAN", Dynamic).with_dynamic_result(Some(ArrayEvaluator::DynamicHelper)),
    function!("CHOOSECOLS", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("CHOOSEROWS", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("DROP", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("FILTER", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("HSTACK", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("MMULT", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("SEQUENCE", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("SORT", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("TAKE", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("TRANSPOSE", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("UNIQUE", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("VSTACK", Array).with_array_evaluator(ArrayEvaluator::Array),
    function!("CORREL", Statistical),
    function!("COVARIANCE.P", Statistical).with_aliases(&[FunctionAlias::official("COVAR")]),
    function!("INTERCEPT", Statistical),
    function!("LARGE", Statistical),
    function!("MAXIFS", Statistical),
    function!("MEDIAN", Statistical),
    function!("MINIFS", Statistical),
    function!("MODE.SNGL", Statistical).with_aliases(&[FunctionAlias::official("MODE")]),
    function!("NORMSDIST", Statistical),
    function!("NORM.S.DIST", Statistical),
    function!("PEARSON", Statistical),
    function!("PERCENTILE.INC", Statistical).with_aliases(&[FunctionAlias::official("PERCENTILE")]),
    function!("PERCENTRANK.INC", Statistical)
        .with_aliases(&[FunctionAlias::official("PERCENTRANK")]),
    function!("QUARTILE.INC", Statistical).with_aliases(&[FunctionAlias::official("QUARTILE")]),
    function!("RANK.EQ", Statistical).with_aliases(&[FunctionAlias::official("RANK")]),
    function!("RSQ", Statistical),
    function!("SLOPE", Statistical),
    function!("SMALL", Statistical),
    function!("STDEV.S", Statistical)
        .with_aliases(&[FunctionAlias::official("STDEV")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("VAR.S", Statistical)
        .with_aliases(&[FunctionAlias::official("VAR")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("AVEDEV", StatisticalAdditional),
    function!("AVERAGEA", StatisticalAdditional).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("DEVSQ", StatisticalAdditional),
    function!("EXPON.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("EXPONDIST")]),
    function!("GAUSS", StatisticalAdditional),
    function!("GEOMEAN", StatisticalAdditional),
    function!("HARMEAN", StatisticalAdditional),
    function!("MAXA", StatisticalAdditional).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("MINA", StatisticalAdditional).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("NORM.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("NORMDIST")]),
    function!("PHI", StatisticalAdditional),
    function!("POISSON.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("POISSON")]),
    function!("STANDARDIZE", StatisticalAdditional),
    function!("STDEV.P", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("STDEVP")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("VAR.P", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("VARP")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!("DB", Financial),
    function!("FV", Financial),
    function!("IPMT", Financial),
    function!("IRR", Financial),
    function!("NPER", Financial),
    function!("NPV", Financial),
    function!("PMT", Financial),
    function!("PPMT", Financial),
    function!("PV", Financial),
    function!("RATE", Financial),
    function!("SLN", Financial),
    function!("SYD", Financial),
    function!("XIRR", Financial),
    function!("DOLLARDE", FinancialAdditional),
    function!("DOLLARFR", FinancialAdditional),
    function!("EFFECT", FinancialAdditional),
    function!("FVSCHEDULE", FinancialAdditional),
    function!("ISPMT", FinancialAdditional),
    function!("MIRR", FinancialAdditional),
    function!("NOMINAL", FinancialAdditional),
    function!("PDURATION", FinancialAdditional),
    function!("RRI", FinancialAdditional),
    function!("AREAS", Areas)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::AreaCount,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN)
        .with_minimum_version(CompatibilityVersion::V0_1_9),
];

struct RegistryIndex {
    canonical: HashMap<&'static str, FunctionId>,
    accepted: HashMap<&'static str, FunctionId>,
    descriptors: HashMap<FunctionId, FunctionDescriptor>,
}

fn registry_index() -> &'static RegistryIndex {
    static INDEX: OnceLock<RegistryIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut canonical = HashMap::with_capacity(DESCRIPTORS.len());
        let alias_count = DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.aliases.len())
            .sum::<usize>();
        let mut accepted = HashMap::with_capacity(DESCRIPTORS.len() + alias_count);
        let mut descriptors = HashMap::with_capacity(DESCRIPTORS.len());
        for descriptor in DESCRIPTORS.iter().copied() {
            canonical.insert(descriptor.canonical_name, descriptor.id());
            accepted.insert(descriptor.canonical_name, descriptor.id());
            descriptors.insert(descriptor.id(), descriptor);
            for alias in descriptor.aliases {
                accepted.insert(alias.name, descriptor.id());
            }
        }
        RegistryIndex {
            canonical,
            accepted,
            descriptors,
        }
    })
}

pub(super) fn descriptors() -> &'static [FunctionDescriptor] {
    DESCRIPTORS
}

pub(super) fn descriptor(canonical_name: &str) -> Option<FunctionDescriptor> {
    let index = registry_index();
    index
        .canonical
        .get(canonical_name)
        .and_then(|id| index.descriptors.get(id))
        .copied()
}

pub(super) fn resolve(name: &str) -> Option<FunctionDescriptor> {
    let upper = name.to_ascii_uppercase();
    resolve_upper(&upper)
}

fn resolve_upper(upper: &str) -> Option<FunctionDescriptor> {
    let index = registry_index();
    let direct = index
        .accepted
        .get(upper)
        .and_then(|id| index.descriptors.get(id))
        .copied();
    if direct.is_some() {
        return direct;
    }
    let base = strip_storage_prefixes(upper);
    index
        .accepted
        .get(base)
        .and_then(|id| index.descriptors.get(id))
        .copied()
        .filter(|descriptor| {
            matches!(
                descriptor.storage_prefix_policy(),
                StoragePrefixPolicy::ExcelNamespaces
            )
        })
}

pub(super) fn normalize_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let base = strip_storage_prefixes(&upper);
    resolve_upper(&upper).map_or_else(
        || base.to_owned(),
        |descriptor| descriptor.canonical_name.to_owned(),
    )
}

fn strip_storage_prefixes(mut name: &str) -> &str {
    while let Some(stripped) = name
        .strip_prefix("_XLFN.")
        .or_else(|| name.strip_prefix("_XLUDF."))
        .or_else(|| name.strip_prefix("_XLWS."))
    {
        name = stripped;
    }
    name
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn registry_has_unique_canonical_ids_and_accepted_names() {
        let canonical = DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.canonical_name())
            .collect::<BTreeSet<_>>();
        assert_eq!(canonical.len(), DESCRIPTORS.len());
        let ids = DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.id())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), DESCRIPTORS.len());

        let accepted = DESCRIPTORS
            .iter()
            .flat_map(|descriptor| {
                std::iter::once(descriptor.canonical_name())
                    .chain(descriptor.aliases().iter().map(|alias| alias.name()))
            })
            .collect::<BTreeSet<_>>();
        let accepted_count = DESCRIPTORS
            .iter()
            .map(|descriptor| 1 + descriptor.aliases().len())
            .sum::<usize>();
        assert_eq!(accepted.len(), accepted_count);
    }

    #[test]
    fn storage_and_legacy_spellings_resolve_to_canonical_descriptors() {
        assert_eq!(normalize_name("_xlfn._xlws.FILTER"), "FILTER");
        assert_eq!(normalize_name("_xludf._xlfn.COVAR"), "COVARIANCE.P");
        assert_eq!(normalize_name("_XLWS._XLUDF._XLFN.SUM"), "SUM");
        assert_eq!(
            resolve("areas").map(FunctionDescriptor::id),
            descriptor("AREAS").map(FunctionDescriptor::id)
        );
        assert_eq!(normalize_name("_xlfn.MYSTERY"), "MYSTERY");
    }

    #[test]
    fn descriptors_expose_migrated_behavior_flags() {
        let areas = descriptor("AREAS").expect("AREAS descriptor");
        assert_eq!(areas.canonical_name(), "AREAS");
        assert_eq!(areas.evaluator(), Evaluator::Areas);
        assert_eq!(areas.argument_policy(), ArgumentPolicy::EvaluatorManaged);
        assert_eq!(areas.result_kind(), FunctionResultKind::Scalar);
        assert_eq!(areas.minimum_version(), CompatibilityVersion::V0_1_9);
        assert_eq!(
            areas.storage_prefix_policy(),
            StoragePrefixPolicy::ExcelNamespaces
        );
        assert_eq!(
            areas.dependency_kind(),
            DependencyKind::ReferenceMetadataOnly(ReferenceMetadataKind::AreaCount)
        );

        let covariance = descriptor("COVARIANCE.P").expect("COVARIANCE.P descriptor");
        let alias = covariance.aliases()[0];
        assert_eq!(alias.name(), "COVAR");
        assert_eq!(alias.adapter(), AliasAdapter::Canonical);
        assert!(alias.is_official());
        assert_eq!(
            resolve("COVAR").map(FunctionDescriptor::id),
            Some(covariance.id())
        );

        for name in [
            "SUM", "AVERAGE", "AVERAGEA", "COUNT", "COUNTA", "MAX", "MAXA", "MIN", "MINA",
            "PRODUCT", "STDEV.P", "STDEV.S", "VAR.P", "VAR.S",
        ] {
            assert_eq!(
                resolve(name).map(FunctionDescriptor::sheet_span_policy),
                Some(SheetSpanPolicy::CollectAcrossSheets),
                "{name}",
            );
        }
        for alias in ["STDEV", "STDEVP", "VAR", "VARP"] {
            assert_eq!(
                resolve(alias).map(FunctionDescriptor::sheet_span_policy),
                Some(SheetSpanPolicy::CollectAcrossSheets),
                "{alias}",
            );
        }
    }
}
