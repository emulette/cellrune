use std::collections::HashMap;
use std::sync::OnceLock;

use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::ErrorKind;
use super::contract::CallContract;
use super::kernel::{
    AggregateFunction, ArrayEvaluator, ArrayFunction, CombinatoricsFunction, DatabaseFunction,
    DateAdditionalFunction, DateFunction, DistributionFunction, DynamicArrayFunction,
    DynamicFunction, ElementwiseArrayFunction, EngineeringFunction, Evaluator,
    FinancialAdditionalFunction, FinancialFunction, GroupedArrayFunction, GroupedFunction,
    InformationArrayFunction, InformationFunction, LegacyArrayFunction, LegacyFunction,
    LogicalFunction, LookupFunction, MathFunction, ModernTextArrayFunction, ModernTextFunction,
    RegressionFunction, RomanFunction, StatisticalAdditionalFunction, StatisticalFunction,
    SumOfSquaresFunction, TextAdditionalFunction, TextFunction, TrigonometryFunction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FunctionId(&'static str);

impl FunctionId {
    const fn new(canonical_name: &'static str) -> Self {
        Self(canonical_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::calculation) struct BuiltinCallable(FunctionId);

impl BuiltinCallable {
    pub(in crate::calculation) const fn canonical_name(self) -> &'static str {
        self.0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum AggregateCallableCapability {
    Unary,
    Relative,
}

impl AggregateCallableCapability {
    pub(in crate::calculation) const fn argument_count(self) -> usize {
        match self {
            Self::Unary => 1,
            Self::Relative => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) enum FunctionResultKind {
    Scalar,
    Array,
    Reference,
    ReferenceOrArray,
    Callable,
    Contextual,
}

impl FunctionResultKind {
    pub(in crate::calculation) const fn returns_reference(self) -> bool {
        matches!(self, Self::Reference | Self::ReferenceOrArray)
    }
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
    FormulaPredicate,
    FormulaText,
    SheetIndex,
    SheetCount,
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
    V0_1_11,
    V0_1_12,
    V0_1_13,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoragePrefixPolicy {
    ExcelNamespaces,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FunctionDescriptor {
    id: FunctionId,
    canonical_name: &'static str,
    aliases: &'static [FunctionAlias],
    evaluator: Evaluator,
    array_evaluator: Option<ArrayEvaluator>,
    call_contract: CallContract,
    result_kind: FunctionResultKind,
    sheet_span_policy: SheetSpanPolicy,
    volatility: Volatility,
    dependency_kind: DependencyKind,
    storage_prefix_policy: StoragePrefixPolicy,
    minimum_version: CompatibilityVersion,
    catalog_array_result: bool,
    public_catalog: bool,
    official: bool,
    builtin_callable: Option<BuiltinCallable>,
    aggregate_callable: Option<AggregateCallableCapability>,
}

impl FunctionDescriptor {
    const fn new(canonical_name: &'static str, evaluator: Evaluator) -> Self {
        Self {
            id: FunctionId::new(canonical_name),
            canonical_name,
            aliases: &[],
            evaluator,
            array_evaluator: None,
            call_contract: evaluator.call_contract(),
            result_kind: FunctionResultKind::Scalar,
            sheet_span_policy: SheetSpanPolicy::Unsupported,
            volatility: Volatility::None,
            dependency_kind: DependencyKind::Standard,
            storage_prefix_policy: StoragePrefixPolicy::ExcelNamespaces,
            minimum_version: CompatibilityVersion::Baseline,
            catalog_array_result: false,
            public_catalog: true,
            official: true,
            builtin_callable: None,
            aggregate_callable: None,
        }
    }

    const fn with_aliases(mut self, aliases: &'static [FunctionAlias]) -> Self {
        self.aliases = aliases;
        self
    }

    const fn with_array_evaluator(mut self, evaluator: ArrayEvaluator) -> Self {
        self.array_evaluator = Some(evaluator);
        self.result_kind = FunctionResultKind::Array;
        self.catalog_array_result = true;
        self
    }

    const fn with_contextual_result(mut self, evaluator: Option<ArrayEvaluator>) -> Self {
        self.array_evaluator = evaluator;
        self.result_kind = FunctionResultKind::Contextual;
        self.catalog_array_result = true;
        self
    }

    const fn with_callable_result(mut self) -> Self {
        self.result_kind = FunctionResultKind::Callable;
        self.catalog_array_result = true;
        self
    }

    const fn with_catalog_array_result(mut self) -> Self {
        self.catalog_array_result = true;
        self
    }

    const fn with_reference_result(mut self) -> Self {
        self.result_kind = FunctionResultKind::Reference;
        self
    }

    const fn with_reference_array_result(mut self, evaluator: ArrayEvaluator) -> Self {
        self.array_evaluator = Some(evaluator);
        self.result_kind = FunctionResultKind::ReferenceOrArray;
        self.catalog_array_result = true;
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

    const fn with_builtin_aggregate(mut self, capability: AggregateCallableCapability) -> Self {
        self.builtin_callable = Some(BuiltinCallable(self.id));
        self.aggregate_callable = Some(capability);
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

    pub(super) const fn call_contract(self) -> CallContract {
        self.call_contract
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

    pub(super) const fn catalog_returns_array(self) -> bool {
        self.catalog_array_result
    }

    pub(super) const fn is_in_public_catalog(self) -> bool {
        self.public_catalog
    }

    pub(super) const fn is_official(self) -> bool {
        self.official
    }

    pub(super) const fn builtin_callable(self) -> Option<BuiltinCallable> {
        self.builtin_callable
    }

    pub(super) const fn aggregate_callable(self) -> Option<AggregateCallableCapability> {
        self.aggregate_callable
    }
}

macro_rules! function {
    ($variant:ident, $name:literal, Legacy) => {
        FunctionDescriptor::new($name, Evaluator::Legacy(LegacyFunction::$variant))
    };
    ($variant:ident, $name:literal, Logical) => {
        FunctionDescriptor::new($name, Evaluator::Logical(LogicalFunction::$variant))
    };
    ($variant:ident, $name:literal, Aggregate) => {
        FunctionDescriptor::new($name, Evaluator::Aggregate(AggregateFunction::$variant))
    };
    ($variant:ident, $name:literal, Grouped) => {
        FunctionDescriptor::new($name, Evaluator::Grouped(GroupedFunction::$variant))
    };
    ($variant:ident, $name:literal, Database) => {
        FunctionDescriptor::new($name, Evaluator::Database(DatabaseFunction::$variant))
    };
    ($variant:ident, $name:literal, Math) => {
        FunctionDescriptor::new($name, Evaluator::Math(MathFunction::$variant))
    };
    ($variant:ident, $name:literal, Roman) => {
        FunctionDescriptor::new($name, Evaluator::Roman(RomanFunction::$variant))
    };
    ($variant:ident, $name:literal, Trigonometry) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::Trigonometry(TrigonometryFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, Combinatorics) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::Combinatorics(CombinatoricsFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, SumOfSquares) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::SumOfSquares(SumOfSquaresFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, Engineering) => {
        FunctionDescriptor::new($name, Evaluator::Engineering(EngineeringFunction::$variant))
    };
    ($variant:ident, $name:literal, Lookup) => {
        FunctionDescriptor::new($name, Evaluator::Lookup(LookupFunction::$variant))
    };
    ($variant:ident, $name:literal, Information) => {
        FunctionDescriptor::new($name, Evaluator::Information(InformationFunction::$variant))
    };
    ($variant:ident, $name:literal, Text) => {
        FunctionDescriptor::new($name, Evaluator::Text(TextFunction::$variant))
    };
    ($variant:ident, $name:literal, TextAdditional) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::TextAdditional(TextAdditionalFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, ModernText) => {
        FunctionDescriptor::new($name, Evaluator::ModernText(ModernTextFunction::$variant))
    };
    ($variant:ident, $name:literal, Date) => {
        FunctionDescriptor::new($name, Evaluator::Date(DateFunction::$variant))
    };
    ($variant:ident, $name:literal, DateAdditional) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::DateAdditional(DateAdditionalFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, Dynamic) => {
        FunctionDescriptor::new($name, Evaluator::Dynamic(DynamicFunction::$variant))
    };
    ($variant:ident, $name:literal, Array) => {
        FunctionDescriptor::new($name, Evaluator::Array(ArrayFunction::$variant))
    };
    ($variant:ident, $name:literal, Regression) => {
        FunctionDescriptor::new($name, Evaluator::Regression(RegressionFunction::$variant))
    };
    ($variant:ident, $name:literal, Statistical) => {
        FunctionDescriptor::new($name, Evaluator::Statistical(StatisticalFunction::$variant))
    };
    ($variant:ident, $name:literal, StatisticalAdditional) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::StatisticalAdditional(StatisticalAdditionalFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, Distribution) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::Distribution(DistributionFunction::$variant),
        )
    };
    ($variant:ident, $name:literal, Financial) => {
        FunctionDescriptor::new($name, Evaluator::Financial(FinancialFunction::$variant))
    };
    ($variant:ident, $name:literal, FinancialAdditional) => {
        FunctionDescriptor::new(
            $name,
            Evaluator::FinancialAdditional(FinancialAdditionalFunction::$variant),
        )
    };
    (Areas, "AREAS", Areas) => {
        FunctionDescriptor::new("AREAS", Evaluator::Areas)
    };
}

const COLLECT_ACROSS_SHEETS: SheetSpanPolicy = SheetSpanPolicy::CollectAcrossSheets;
const VALUE_ON_SHEET_SPAN: SheetSpanPolicy = SheetSpanPolicy::ReturnExcelError(ErrorKind::Value);
const REF_ON_SHEET_SPAN: SheetSpanPolicy = SheetSpanPolicy::ReturnExcelError(ErrorKind::Ref);

// This is the sole inventory of supported functions and accepted public aliases. Keep behavior
// flags beside each canonical registration so evaluation, capability analysis and catalog
// serialization cannot drift into independent name lists.
const DESCRIPTORS: &[FunctionDescriptor] = &[
    function!(If, "IF", Legacy)
        .with_array_evaluator(ArrayEvaluator::Legacy(LegacyArrayFunction::If)),
    function!(And, "AND", Legacy),
    function!(IfError, "IFERROR", Legacy),
    function!(Lower, "LOWER", Legacy),
    function!(Text, "TEXT", Legacy),
    function!(CountIf, "COUNTIF", Legacy)
        .with_array_evaluator(ArrayEvaluator::Legacy(LegacyArrayFunction::CountIf)),
    function!(CountIfs, "COUNTIFS", Legacy)
        .with_array_evaluator(ArrayEvaluator::Legacy(LegacyArrayFunction::CountIfs)),
    function!(SumProduct, "SUMPRODUCT", Legacy),
    function!(Index, "INDEX", Legacy)
        .with_reference_array_result(ArrayEvaluator::Legacy(LegacyArrayFunction::Index))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!(Match, "MATCH", Legacy),
    function!(DummyFunction, "__XLUDF.DUMMYFUNCTION", Legacy).unofficial(),
    function!(True, "TRUE", Logical),
    function!(False, "FALSE", Logical),
    function!(Not, "NOT", Logical),
    function!(Or, "OR", Logical),
    function!(Xor, "XOR", Logical),
    function!(IfNa, "IFNA", Logical),
    function!(Ifs, "IFS", Logical),
    function!(Switch, "SWITCH", Logical),
    function!(Sum, "SUM", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(Average, "AVERAGE", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(Min, "MIN", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(Max, "MAX", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(Count, "COUNT", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(CountA, "COUNTA", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(CountBlank, "COUNTBLANK", Aggregate),
    function!(Product, "PRODUCT", Aggregate)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_builtin_aggregate(AggregateCallableCapability::Unary),
    function!(Subtotal, "SUBTOTAL", Aggregate),
    function!(SumIf, "SUMIF", Aggregate)
        .with_dependency_kind(DependencyKind::ResizedCriteriaValueRange),
    function!(SumIfs, "SUMIFS", Aggregate),
    function!(AverageIf, "AVERAGEIF", Aggregate)
        .with_dependency_kind(DependencyKind::ResizedCriteriaValueRange),
    function!(AverageIfs, "AVERAGEIFS", Aggregate),
    function!(Average, "DAVERAGE", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Count, "DCOUNT", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(CountA, "DCOUNTA", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Get, "DGET", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Max, "DMAX", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Min, "DMIN", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Product, "DPRODUCT", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(StDev, "DSTDEV", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(StDevP, "DSTDEVP", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Sum, "DSUM", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Var, "DVAR", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(VarP, "DVARP", Database).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Abs, "ABS", Math)
        .with_array_evaluator(ArrayEvaluator::Elementwise(ElementwiseArrayFunction::Abs)),
    function!(Base, "BASE", Math),
    function!(Ceiling, "CEILING", Math),
    function!(CeilingMath, "CEILING.MATH", Math),
    function!(CeilingPrecise, "CEILING.PRECISE", Math),
    function!(Decimal, "DECIMAL", Math),
    function!(Even, "EVEN", Math),
    function!(Exp, "EXP", Math),
    function!(Floor, "FLOOR", Math),
    function!(FloorMath, "FLOOR.MATH", Math),
    function!(FloorPrecise, "FLOOR.PRECISE", Math),
    function!(Int, "INT", Math),
    function!(IsoCeiling, "ISO.CEILING", Math),
    function!(Ln, "LN", Math),
    function!(Log, "LOG", Math),
    function!(Log10, "LOG10", Math),
    function!(Mod, "MOD", Math),
    function!(MRound, "MROUND", Math),
    function!(Odd, "ODD", Math),
    function!(Pi, "PI", Math),
    function!(Power, "POWER", Math),
    function!(Quotient, "QUOTIENT", Math),
    function!(Round, "ROUND", Math),
    function!(RoundDown, "ROUNDDOWN", Math),
    function!(RoundUp, "ROUNDUP", Math),
    function!(SeriesSum, "SERIESSUM", Math),
    function!(Sign, "SIGN", Math),
    function!(Sqrt, "SQRT", Math),
    function!(SqrtPi, "SQRTPI", Math),
    function!(Trunc, "TRUNC", Math),
    function!(Arabic, "ARABIC", Roman).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Roman, "ROMAN", Roman).with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Acos, "ACOS", Trigonometry),
    function!(Acosh, "ACOSH", Trigonometry),
    function!(Acot, "ACOT", Trigonometry),
    function!(Acoth, "ACOTH", Trigonometry),
    function!(Asin, "ASIN", Trigonometry),
    function!(Asinh, "ASINH", Trigonometry),
    function!(Atan, "ATAN", Trigonometry),
    function!(Atan2, "ATAN2", Trigonometry),
    function!(Atanh, "ATANH", Trigonometry),
    function!(Cos, "COS", Trigonometry),
    function!(Cosh, "COSH", Trigonometry),
    function!(Cot, "COT", Trigonometry),
    function!(Coth, "COTH", Trigonometry),
    function!(Csc, "CSC", Trigonometry),
    function!(Csch, "CSCH", Trigonometry),
    function!(Degrees, "DEGREES", Trigonometry),
    function!(Radians, "RADIANS", Trigonometry),
    function!(Sec, "SEC", Trigonometry),
    function!(Sech, "SECH", Trigonometry),
    function!(Sin, "SIN", Trigonometry),
    function!(Sinh, "SINH", Trigonometry),
    function!(Tan, "TAN", Trigonometry),
    function!(Tanh, "TANH", Trigonometry),
    function!(Combin, "COMBIN", Combinatorics),
    function!(Combina, "COMBINA", Combinatorics),
    function!(Fact, "FACT", Combinatorics),
    function!(FactDouble, "FACTDOUBLE", Combinatorics),
    function!(Gcd, "GCD", Combinatorics),
    function!(Lcm, "LCM", Combinatorics),
    function!(Multinomial, "MULTINOMIAL", Combinatorics),
    function!(Permut, "PERMUT", Combinatorics),
    function!(PermutationA, "PERMUTATIONA", Combinatorics),
    function!(SumSq, "SUMSQ", SumOfSquares),
    function!(SumX2My2, "SUMX2MY2", SumOfSquares),
    function!(SumX2Py2, "SUMX2PY2", SumOfSquares),
    function!(SumXMy2, "SUMXMY2", SumOfSquares),
    function!(Bin2Dec, "BIN2DEC", Engineering),
    function!(Bin2Hex, "BIN2HEX", Engineering),
    function!(Bin2Oct, "BIN2OCT", Engineering),
    function!(BitAnd, "BITAND", Engineering),
    function!(BitLShift, "BITLSHIFT", Engineering),
    function!(BitOr, "BITOR", Engineering),
    function!(BitRShift, "BITRSHIFT", Engineering),
    function!(BitXor, "BITXOR", Engineering),
    function!(Dec2Bin, "DEC2BIN", Engineering),
    function!(Dec2Hex, "DEC2HEX", Engineering),
    function!(Dec2Oct, "DEC2OCT", Engineering),
    function!(Delta, "DELTA", Engineering),
    function!(Erf, "ERF", Engineering),
    function!(ErfPrecise, "ERF.PRECISE", Engineering),
    function!(Erfc, "ERFC", Engineering),
    function!(ErfcPrecise, "ERFC.PRECISE", Engineering),
    function!(GeStep, "GESTEP", Engineering),
    function!(Hex2Bin, "HEX2BIN", Engineering),
    function!(Hex2Dec, "HEX2DEC", Engineering),
    function!(Hex2Oct, "HEX2OCT", Engineering),
    function!(Oct2Bin, "OCT2BIN", Engineering),
    function!(Oct2Dec, "OCT2DEC", Engineering),
    function!(Oct2Hex, "OCT2HEX", Engineering),
    function!(Address, "ADDRESS", Lookup),
    function!(Choose, "CHOOSE", Lookup),
    function!(Column, "COLUMN", Lookup),
    function!(Columns, "COLUMNS", Lookup),
    function!(HLookup, "HLOOKUP", Lookup),
    function!(Hyperlink, "HYPERLINK", Lookup),
    function!(Indirect, "INDIRECT", Lookup)
        .with_reference_result()
        .with_dependency_kind(DependencyKind::DynamicReference(
            DynamicReferenceKind::Indirect,
        )),
    function!(Lookup, "LOOKUP", Lookup),
    function!(Offset, "OFFSET", Lookup)
        .with_reference_result()
        .with_sheet_span_policy(REF_ON_SHEET_SPAN)
        .with_dependency_kind(DependencyKind::DynamicReference(
            DynamicReferenceKind::Offset,
        )),
    function!(Rows, "ROWS", Lookup),
    function!(Row, "ROW", Lookup),
    function!(Sheet, "SHEET", Lookup)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::SheetIndex,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Sheets, "SHEETS", Lookup)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::SheetCount,
        ))
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(VLookup, "VLOOKUP", Lookup).with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!(XMatch, "XMATCH", Lookup).with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(XLookup, "XLOOKUP", Lookup),
    function!(GroupBy, "GROUPBY", Grouped)
        .with_array_evaluator(ArrayEvaluator::Grouped(GroupedArrayFunction::GroupBy))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(PercentOf, "PERCENTOF", Grouped)
        .with_builtin_aggregate(AggregateCallableCapability::Relative)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(PivotBy, "PIVOTBY", Grouped)
        .with_array_evaluator(ArrayEvaluator::Grouped(GroupedArrayFunction::PivotBy))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(ErrorType, "ERROR.TYPE", Information).with_array_evaluator(
        ArrayEvaluator::Information(InformationArrayFunction::ErrorType),
    ),
    function!(FormulaText, "FORMULATEXT", Information)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::FormulaText,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(IsBlank, "ISBLANK", Information).with_array_evaluator(ArrayEvaluator::Information(
        InformationArrayFunction::IsBlank,
    )),
    function!(IsErr, "ISERR", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::IsErr)),
    function!(IsError, "ISERROR", Information).with_array_evaluator(ArrayEvaluator::Information(
        InformationArrayFunction::IsError,
    )),
    function!(IsEven, "ISEVEN", Information).with_array_evaluator(ArrayEvaluator::Information(
        InformationArrayFunction::IsEven,
    )),
    function!(IsFormula, "ISFORMULA", Information)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::FormulaPredicate,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(IsLogical, "ISLOGICAL", Information).with_array_evaluator(
        ArrayEvaluator::Information(InformationArrayFunction::IsLogical),
    ),
    function!(IsNa, "ISNA", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::IsNa)),
    function!(IsNonText, "ISNONTEXT", Information).with_array_evaluator(
        ArrayEvaluator::Information(InformationArrayFunction::IsNonText),
    ),
    function!(IsNumber, "ISNUMBER", Information).with_array_evaluator(ArrayEvaluator::Information(
        InformationArrayFunction::IsNumber,
    )),
    function!(IsOdd, "ISODD", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::IsOdd)),
    function!(IsRef, "ISREF", Information)
        .with_dependency_kind(DependencyKind::ReferenceMetadataOnly(
            ReferenceMetadataKind::Predicate,
        ))
        .with_sheet_span_policy(VALUE_ON_SHEET_SPAN),
    function!(IsText, "ISTEXT", Information).with_array_evaluator(ArrayEvaluator::Information(
        InformationArrayFunction::IsText,
    )),
    function!(N, "N", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::N)),
    function!(Na, "NA", Information),
    function!(T, "T", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::T)),
    function!(Type, "TYPE", Information)
        .with_array_evaluator(ArrayEvaluator::Information(InformationArrayFunction::Type)),
    function!(Concat, "CONCAT", Text),
    function!(Exact, "EXACT", Text),
    function!(Find, "FIND", Text),
    function!(Left, "LEFT", Text),
    function!(Len, "LEN", Text),
    function!(Mid, "MID", Text),
    function!(Proper, "PROPER", Text),
    function!(Replace, "REPLACE", Text),
    function!(Rept, "REPT", Text),
    function!(Right, "RIGHT", Text),
    function!(Search, "SEARCH", Text),
    function!(Substitute, "SUBSTITUTE", Text),
    function!(TextJoin, "TEXTJOIN", Text),
    function!(Trim, "TRIM", Text),
    function!(Upper, "UPPER", Text),
    function!(Char, "CHAR", TextAdditional),
    function!(Clean, "CLEAN", TextAdditional),
    function!(Concatenate, "CONCATENATE", TextAdditional),
    function!(Dollar, "DOLLAR", TextAdditional),
    function!(TextAfter, "TEXTAFTER", TextAdditional),
    function!(TextBefore, "TEXTBEFORE", TextAdditional),
    function!(UniChar, "UNICHAR", TextAdditional),
    function!(Unicode, "UNICODE", TextAdditional),
    function!(Value, "VALUE", TextAdditional),
    function!(ValueToText, "VALUETOTEXT", TextAdditional),
    function!(ArrayToText, "ARRAYTOTEXT", ModernText)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(RegexExtract, "REGEXEXTRACT", ModernText)
        .with_array_evaluator(ArrayEvaluator::ModernText(
            ModernTextArrayFunction::RegexExtract,
        ))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(RegexReplace, "REGEXREPLACE", ModernText)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(RegexTest, "REGEXTEST", ModernText)
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(TextSplit, "TEXTSPLIT", ModernText)
        .with_array_evaluator(ArrayEvaluator::ModernText(
            ModernTextArrayFunction::TextSplit,
        ))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Date, "DATE", Date),
    function!(DateDif, "DATEDIF", Date),
    function!(Day, "DAY", Date),
    function!(EDate, "EDATE", Date),
    function!(Eomonth, "EOMONTH", Date),
    function!(Month, "MONTH", Date),
    function!(NetworkDays, "NETWORKDAYS", Date),
    function!(Now, "NOW", Date).with_volatility(Volatility::Now),
    function!(Today, "TODAY", Date).with_volatility(Volatility::Today),
    function!(Weekday, "WEEKDAY", Date),
    function!(Workday, "WORKDAY", Date),
    function!(Year, "YEAR", Date),
    function!(YearFrac, "YEARFRAC", Date),
    function!(Days, "DAYS", DateAdditional),
    function!(Days360, "DAYS360", DateAdditional),
    function!(Hour, "HOUR", DateAdditional),
    function!(IsoWeekNum, "ISOWEEKNUM", DateAdditional),
    function!(Minute, "MINUTE", DateAdditional),
    function!(Second, "SECOND", DateAdditional),
    function!(Time, "TIME", DateAdditional),
    function!(WeekNum, "WEEKNUM", DateAdditional),
    function!(ByCol, "BYCOL", Dynamic)
        .with_array_evaluator(ArrayEvaluator::Dynamic(DynamicArrayFunction::ByCol)),
    function!(ByRow, "BYROW", Dynamic)
        .with_array_evaluator(ArrayEvaluator::Dynamic(DynamicArrayFunction::ByRow)),
    function!(IsOmitted, "ISOMITTED", Dynamic).with_catalog_array_result(),
    function!(Lambda, "LAMBDA", Dynamic).with_callable_result(),
    function!(Let, "LET", Dynamic).with_contextual_result(None),
    function!(MakeArray, "MAKEARRAY", Dynamic)
        .with_array_evaluator(ArrayEvaluator::Dynamic(DynamicArrayFunction::MakeArray)),
    function!(Map, "MAP", Dynamic).with_array_evaluator(ArrayEvaluator::Map),
    function!(Reduce, "REDUCE", Dynamic)
        .with_contextual_result(Some(ArrayEvaluator::Dynamic(DynamicArrayFunction::Reduce))),
    function!(Scan, "SCAN", Dynamic)
        .with_array_evaluator(ArrayEvaluator::Dynamic(DynamicArrayFunction::Scan)),
    function!(ChooseCols, "CHOOSECOLS", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::ChooseCols)),
    function!(ChooseRows, "CHOOSEROWS", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::ChooseRows)),
    function!(Drop, "DROP", Array).with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Drop)),
    function!(Expand, "EXPAND", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Expand))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Filter, "FILTER", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Filter)),
    function!(HStack, "HSTACK", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::HStack)),
    function!(MInverse, "MINVERSE", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::MInverse))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(MMult, "MMULT", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::MMult)),
    function!(MUnit, "MUNIT", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::MUnit))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Sequence, "SEQUENCE", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Sequence)),
    function!(Sort, "SORT", Array).with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Sort)),
    function!(SortBy, "SORTBY", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::SortBy))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Take, "TAKE", Array).with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Take)),
    function!(ToCol, "TOCOL", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::ToCol))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(ToRow, "TOROW", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::ToRow))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Transpose, "TRANSPOSE", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Transpose)),
    function!(TrimRange, "TRIMRANGE", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::TrimRange))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Unique, "UNIQUE", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::Unique)),
    function!(VStack, "VSTACK", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::VStack)),
    function!(WrapCols, "WRAPCOLS", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::WrapCols))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(WrapRows, "WRAPROWS", Array)
        .with_array_evaluator(ArrayEvaluator::Array(ArrayFunction::WrapRows))
        .with_minimum_version(CompatibilityVersion::V0_1_10),
    function!(Growth, "GROWTH", Regression)
        .with_array_evaluator(ArrayEvaluator::Regression(RegressionFunction::Growth))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(LinEst, "LINEST", Regression)
        .with_array_evaluator(ArrayEvaluator::Regression(RegressionFunction::LinEst))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(LogEst, "LOGEST", Regression)
        .with_array_evaluator(ArrayEvaluator::Regression(RegressionFunction::LogEst))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Trend, "TREND", Regression)
        .with_array_evaluator(ArrayEvaluator::Regression(RegressionFunction::Trend))
        .with_minimum_version(CompatibilityVersion::V0_1_11),
    function!(Correl, "CORREL", Statistical),
    function!(CovarianceP, "COVARIANCE.P", Statistical)
        .with_aliases(&[FunctionAlias::official("COVAR")]),
    function!(CovarianceS, "COVARIANCE.S", Statistical)
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(FTest, "F.TEST", Statistical)
        .with_aliases(&[FunctionAlias::official("FTEST")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TTest, "T.TEST", Statistical)
        .with_aliases(&[FunctionAlias::official("TTEST")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(ZTest, "Z.TEST", Statistical)
        .with_aliases(&[FunctionAlias::official("ZTEST")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(Intercept, "INTERCEPT", Statistical),
    function!(Large, "LARGE", Statistical),
    function!(MaxIfs, "MAXIFS", Statistical),
    function!(Median, "MEDIAN", Statistical),
    function!(MinIfs, "MINIFS", Statistical),
    function!(ModeSingle, "MODE.SNGL", Statistical)
        .with_aliases(&[FunctionAlias::official("MODE")]),
    function!(NormSDistLegacy, "NORMSDIST", Statistical),
    function!(NormSDist, "NORM.S.DIST", Statistical),
    function!(Pearson, "PEARSON", Statistical),
    function!(PercentileInc, "PERCENTILE.INC", Statistical)
        .with_aliases(&[FunctionAlias::official("PERCENTILE")]),
    function!(PercentRankInc, "PERCENTRANK.INC", Statistical)
        .with_aliases(&[FunctionAlias::official("PERCENTRANK")]),
    function!(QuartileInc, "QUARTILE.INC", Statistical)
        .with_aliases(&[FunctionAlias::official("QUARTILE")]),
    function!(RankEq, "RANK.EQ", Statistical).with_aliases(&[FunctionAlias::official("RANK")]),
    function!(Rsq, "RSQ", Statistical),
    function!(Slope, "SLOPE", Statistical),
    function!(Small, "SMALL", Statistical),
    function!(StDevS, "STDEV.S", Statistical)
        .with_aliases(&[FunctionAlias::official("STDEV")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(VarS, "VAR.S", Statistical)
        .with_aliases(&[FunctionAlias::official("VAR")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(AveDev, "AVEDEV", StatisticalAdditional),
    function!(AverageA, "AVERAGEA", StatisticalAdditional)
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(DevSq, "DEVSQ", StatisticalAdditional),
    function!(ExponDist, "EXPON.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("EXPONDIST")]),
    function!(Gauss, "GAUSS", StatisticalAdditional),
    function!(GeoMean, "GEOMEAN", StatisticalAdditional),
    function!(HarMean, "HARMEAN", StatisticalAdditional),
    function!(MaxA, "MAXA", StatisticalAdditional).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(MinA, "MINA", StatisticalAdditional).with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(NormDist, "NORM.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("NORMDIST")]),
    function!(Phi, "PHI", StatisticalAdditional),
    function!(PoissonDist, "POISSON.DIST", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("POISSON")]),
    function!(Standardize, "STANDARDIZE", StatisticalAdditional),
    function!(StDevP, "STDEV.P", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("STDEVP")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(VarP, "VAR.P", StatisticalAdditional)
        .with_aliases(&[FunctionAlias::official("VARP")])
        .with_sheet_span_policy(COLLECT_ACROSS_SHEETS),
    function!(BetaDist, "BETA.DIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(FDist, "F.DIST", Distribution).with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(FDistRt, "F.DIST.RT", Distribution)
        .with_aliases(&[FunctionAlias::official("FDIST")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(FInv, "F.INV", Distribution).with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(FInvRt, "F.INV.RT", Distribution)
        .with_aliases(&[FunctionAlias::official("FINV")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TDist, "T.DIST", Distribution).with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TDistRt, "T.DIST.RT", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TDist2T, "T.DIST.2T", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TInv, "T.INV", Distribution).with_minimum_version(CompatibilityVersion::V0_1_13),
    function!(TInv2T, "T.INV.2T", Distribution)
        .with_aliases(&[FunctionAlias::official("TINV")])
        .with_minimum_version(CompatibilityVersion::V0_1_13),
    // TDIST keeps its own kernel entry: the legacy signature carries a tails
    // argument the modern names lack, so it cannot be a canonical-adapter
    // alias (mirroring BETADIST's separation).
    function!(TDists, "TDIST", Distribution).with_minimum_version(CompatibilityVersion::V0_1_13),
    // BETADIST keeps its own kernel entry: the legacy signature drops the
    // cumulative flag, so it cannot be a canonical-adapter alias.
    function!(BetaDistLegacy, "BETADIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(BetaInv, "BETA.INV", Distribution)
        .with_aliases(&[FunctionAlias::official("BETAINV")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(BinomDist, "BINOM.DIST", Distribution)
        .with_aliases(&[FunctionAlias::official("BINOMDIST")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(BinomDistRange, "BINOM.DIST.RANGE", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(BinomInv, "BINOM.INV", Distribution)
        .with_aliases(&[FunctionAlias::official("CRITBINOM")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(Gamma, "GAMMA", Distribution).with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(GammaDist, "GAMMA.DIST", Distribution)
        .with_aliases(&[FunctionAlias::official("GAMMADIST")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(GammaInv, "GAMMA.INV", Distribution)
        .with_aliases(&[FunctionAlias::official("GAMMAINV")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(GammaLnPrecise, "GAMMALN.PRECISE", Distribution)
        .with_aliases(&[FunctionAlias::official("GAMMALN")])
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(HypgeomDist, "HYPGEOM.DIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    // HYPGEOMDIST takes four arguments to HYPGEOM.DIST's five, so it is a
    // distinct function rather than an alias.
    function!(HypgeomDistLegacy, "HYPGEOMDIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(NegBinomDist, "NEGBINOM.DIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    // Legacy arity (3, mass only) makes NEGBINOMDIST a distinct adapter
    // kernel rather than an alias of NEGBINOM.DIST.
    function!(NegBinomDistLegacy, "NEGBINOMDIST", Distribution)
        .with_minimum_version(CompatibilityVersion::V0_1_12),
    function!(Db, "DB", Financial),
    function!(Fv, "FV", Financial),
    function!(Ipmt, "IPMT", Financial),
    function!(Irr, "IRR", Financial),
    function!(Nper, "NPER", Financial),
    function!(Npv, "NPV", Financial),
    function!(Pmt, "PMT", Financial),
    function!(Ppmt, "PPMT", Financial),
    function!(Pv, "PV", Financial),
    function!(Rate, "RATE", Financial),
    function!(Sln, "SLN", Financial),
    function!(Syd, "SYD", Financial),
    function!(Xirr, "XIRR", Financial),
    function!(DollarDe, "DOLLARDE", FinancialAdditional),
    function!(DollarFr, "DOLLARFR", FinancialAdditional),
    function!(Effect, "EFFECT", FinancialAdditional),
    function!(FvSchedule, "FVSCHEDULE", FinancialAdditional),
    function!(IsPmt, "ISPMT", FinancialAdditional),
    function!(Mirr, "MIRR", FinancialAdditional),
    function!(Nominal, "NOMINAL", FinancialAdditional),
    function!(PDuration, "PDURATION", FinancialAdditional),
    function!(Rri, "RRI", FinancialAdditional),
    function!(Areas, "AREAS", Areas)
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

pub(super) fn callable_descriptor(callable: BuiltinCallable) -> FunctionDescriptor {
    let index = registry_index();
    let descriptor = *index
        .descriptors
        .get(&callable.0)
        .expect("BuiltinCallable descriptor ID must resolve");
    assert_eq!(descriptor.builtin_callable(), Some(callable));
    descriptor
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
mod snapshot;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use sha2::{Digest, Sha256};

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
    fn every_typed_evaluator_is_registered_exactly_once() {
        let registered = DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.evaluator())
            .collect::<BTreeSet<_>>();
        assert_eq!(registered.len(), DESCRIPTORS.len());
        assert_eq!(
            registered,
            Evaluator::all().into_iter().collect::<BTreeSet<_>>()
        );

        let registered_arrays = DESCRIPTORS
            .iter()
            .filter_map(|descriptor| descriptor.array_evaluator())
            .collect::<BTreeSet<_>>();
        let registered_array_count = DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.array_evaluator().is_some())
            .count();
        assert_eq!(registered_arrays.len(), registered_array_count);
        assert_eq!(
            registered_arrays,
            ArrayEvaluator::all().into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn builtin_callable_descriptors_are_total_unique_and_canonical() {
        let registered = DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.builtin_callable().is_some())
            .copied()
            .collect::<Vec<_>>();
        let callables = registered
            .iter()
            .filter_map(|descriptor| descriptor.builtin_callable())
            .collect::<BTreeSet<_>>();
        assert_eq!(registered.len(), 8);
        assert_eq!(callables.len(), registered.len());

        for descriptor in registered {
            let callable = descriptor.builtin_callable().expect("callable descriptor");
            assert_eq!(callable_descriptor(callable), descriptor);
            assert_eq!(descriptor.canonical_name(), callable.canonical_name());
            assert!(matches!(
                descriptor.evaluator(),
                Evaluator::Aggregate(_) | Evaluator::Grouped(GroupedFunction::PercentOf)
            ));
            assert!(descriptor.aggregate_callable().is_some());
        }

        for name in ["SUM", "AVERAGE", "MIN", "MAX", "COUNT", "COUNTA", "PRODUCT"] {
            assert_eq!(
                descriptor(name).and_then(FunctionDescriptor::aggregate_callable),
                Some(AggregateCallableCapability::Unary),
                "{name}",
            );
        }
        assert_eq!(
            descriptor("PERCENTOF").and_then(FunctionDescriptor::aggregate_callable),
            Some(AggregateCallableCapability::Relative),
        );
        assert_eq!(
            descriptor("COUNTBLANK").and_then(FunctionDescriptor::aggregate_callable),
            None,
        );
    }

    #[test]
    fn v0_1_10_semantic_registry_is_byte_exact() {
        let snapshot = super::snapshot::stable_semantic_snapshot(CompatibilityVersion::V0_1_10);
        let mut digest = Sha256::new();
        digest.update(snapshot.as_bytes());
        let actual = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, "2aad3e6874cde2c1de382dd5ef321d4b4c29684daf1981d7a6a1c5590a0cc045",
            "stable v0.1.10 semantic snapshot changed:\n{snapshot}",
        );
    }

    #[test]
    fn v0_1_11_semantic_registry_is_byte_exact() {
        let snapshot = super::snapshot::stable_semantic_snapshot(CompatibilityVersion::V0_1_11);
        let mut digest = Sha256::new();
        digest.update(snapshot.as_bytes());
        let actual = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, "3cd26d201cee732fd556d6d8cdfc742b7de99e887ac1f1c72a4d0d67a0aee93d",
            "stable v0.1.11 semantic snapshot changed:\n{snapshot}",
        );
    }

    #[test]
    fn v0_1_11_descriptors_freeze_function_families_arity_and_array_results() {
        for name in [
            "DAVERAGE", "DCOUNT", "DCOUNTA", "DGET", "DMAX", "DMIN", "DPRODUCT", "DSTDEV",
            "DSTDEVP", "DSUM", "DVAR", "DVARP",
        ] {
            let descriptor = descriptor(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(
                descriptor.minimum_version(),
                CompatibilityVersion::V0_1_11,
                "{name}",
            );
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Database(_)),
                "{name}"
            );
            assert!(descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(4), "{name}");
            assert!(!descriptor.catalog_returns_array(), "{name}");
        }

        for name in ["GROWTH", "LINEST", "LOGEST", "TREND"] {
            let descriptor = descriptor(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(descriptor.minimum_version(), CompatibilityVersion::V0_1_11);
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Regression(_)),
                "{name}"
            );
            assert!((1..=4).all(|arity| descriptor.call_contract().arity().accepts(arity)));
            assert!(!descriptor.call_contract().arity().accepts(0), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(5), "{name}");
            assert!(descriptor.catalog_returns_array(), "{name}");
        }

        for name in ["MINVERSE", "MUNIT"] {
            let descriptor = descriptor(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(descriptor.minimum_version(), CompatibilityVersion::V0_1_11);
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Array(_)),
                "{name}"
            );
            assert!(descriptor.call_contract().arity().accepts(1), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(0), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(descriptor.catalog_returns_array(), "{name}");
        }

        let arabic = descriptor("ARABIC").expect("ARABIC descriptor");
        let roman = descriptor("ROMAN").expect("ROMAN descriptor");
        for descriptor in [arabic, roman] {
            assert_eq!(descriptor.minimum_version(), CompatibilityVersion::V0_1_11);
            assert!(matches!(descriptor.evaluator(), Evaluator::Roman(_)));
            assert!(!descriptor.catalog_returns_array());
        }
        assert!(arabic.call_contract().arity().accepts(1));
        assert!(!arabic.call_contract().arity().accepts(0));
        assert!(!arabic.call_contract().arity().accepts(2));
        assert!(roman.call_contract().arity().accepts(1));
        assert!(roman.call_contract().arity().accepts(2));
        assert!(!roman.call_contract().arity().accepts(0));
        assert!(!roman.call_contract().arity().accepts(3));
    }

    #[test]
    fn v0_1_12_semantic_registry_is_byte_exact() {
        let snapshot = super::snapshot::stable_semantic_snapshot(CompatibilityVersion::V0_1_12);
        let mut digest = Sha256::new();
        digest.update(snapshot.as_bytes());
        let actual = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, "8f9e63dcadbd7b7839f1fcb22471519cb9dafa696925f3fc6e1ee68d9377c4d0",
            "stable v0.1.12 semantic snapshot changed:\n{snapshot}",
        );
    }

    #[test]
    fn v0_1_12_descriptors_freeze_function_families_arity_and_array_results() {
        // Six of the twenty names are aliases, so the lookup resolves accepted spellings rather
        // than canonical ones only.
        for name in [
            "BETA.DIST",
            "BETA.INV",
            "BETADIST",
            "BETAINV",
            "BINOM.DIST",
            "BINOM.DIST.RANGE",
            "BINOM.INV",
            "BINOMDIST",
            "CRITBINOM",
            "GAMMA",
            "GAMMA.DIST",
            "GAMMA.INV",
            "GAMMADIST",
            "GAMMAINV",
            "GAMMALN",
            "GAMMALN.PRECISE",
            "HYPGEOM.DIST",
            "HYPGEOMDIST",
            "NEGBINOM.DIST",
            "NEGBINOMDIST",
        ] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(
                descriptor.minimum_version(),
                CompatibilityVersion::V0_1_12,
                "{name}",
            );
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Distribution(_)),
                "{name}"
            );
            assert!(!descriptor.catalog_returns_array(), "{name}");
        }

        for name in ["GAMMA", "GAMMALN", "GAMMALN.PRECISE"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(1), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(0), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
        }

        for name in [
            "GAMMA.INV",
            "GAMMAINV",
            "BINOM.INV",
            "CRITBINOM",
            "NEGBINOMDIST",
        ] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(4), "{name}");
        }

        for name in [
            "GAMMA.DIST",
            "GAMMADIST",
            "BINOM.DIST",
            "BINOMDIST",
            "NEGBINOM.DIST",
            "HYPGEOMDIST",
        ] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(4), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(5), "{name}");
        }

        let binom_dist_range = resolve("BINOM.DIST.RANGE").expect("BINOM.DIST.RANGE descriptor");
        assert!((3..=4).all(|arity| binom_dist_range.call_contract().arity().accepts(arity)));
        assert!(!binom_dist_range.call_contract().arity().accepts(2));
        assert!(!binom_dist_range.call_contract().arity().accepts(5));

        let beta_dist = resolve("BETA.DIST").expect("BETA.DIST descriptor");
        assert!((4..=6).all(|arity| beta_dist.call_contract().arity().accepts(arity)));
        assert!(!beta_dist.call_contract().arity().accepts(3));
        assert!(!beta_dist.call_contract().arity().accepts(7));

        for name in ["BETA.INV", "BETAINV", "BETADIST"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(
                (3..=5).all(|arity| descriptor.call_contract().arity().accepts(arity)),
                "{name}",
            );
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(6), "{name}");
        }

        let hypgeom_dist = resolve("HYPGEOM.DIST").expect("HYPGEOM.DIST descriptor");
        assert!(hypgeom_dist.call_contract().arity().accepts(5));
        assert!(!hypgeom_dist.call_contract().arity().accepts(4));
        assert!(!hypgeom_dist.call_contract().arity().accepts(6));
    }

    #[test]
    fn v0_1_13_semantic_registry_is_byte_exact() {
        let snapshot = super::snapshot::stable_semantic_snapshot(CompatibilityVersion::V0_1_13);
        let mut digest = Sha256::new();
        digest.update(snapshot.as_bytes());
        let actual = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, "881849f24d58c69772f41a2507159277dae6f0d9d42589d93720428c6742de95",
            "stable v0.1.13 semantic snapshot changed:\n{snapshot}",
        );
    }

    #[test]
    fn v0_1_13_descriptors_freeze_function_families_arity_and_array_results() {
        // Six of the twenty names are aliases (FDIST, FINV, FTEST, TINV, TTEST,
        // ZTEST), so the lookup resolves accepted spellings rather than
        // canonical ones only. TDIST is a kernel with its own tails argument,
        // not an alias (mirroring BETADIST's separation).
        for name in [
            "F.DIST",
            "F.DIST.RT",
            "FDIST",
            "F.INV",
            "F.INV.RT",
            "FINV",
            "F.TEST",
            "FTEST",
            "T.DIST",
            "T.DIST.RT",
            "T.DIST.2T",
            "T.INV",
            "T.INV.2T",
            "TINV",
            "T.TEST",
            "TTEST",
            "TDIST",
            "Z.TEST",
            "ZTEST",
            "COVARIANCE.S",
        ] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(
                descriptor.minimum_version(),
                CompatibilityVersion::V0_1_13,
                "{name}",
            );
            assert!(
                matches!(
                    descriptor.evaluator(),
                    Evaluator::Distribution(_) | Evaluator::Statistical(_)
                ),
                "{name}"
            );
            assert!(!descriptor.catalog_returns_array(), "{name}");
        }

        for name in ["COVARIANCE.S", "F.TEST", "FTEST"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Statistical(_)),
                "{name}"
            );
            assert!(descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(1), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(3), "{name}");
        }

        // T.TEST takes its tails and type selectors after the two arrays.
        for name in ["T.TEST", "TTEST"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Statistical(_)),
                "{name}"
            );
            assert!(descriptor.call_contract().arity().accepts(4), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(5), "{name}");
        }

        for name in ["Z.TEST", "ZTEST"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(
                matches!(descriptor.evaluator(), Evaluator::Statistical(_)),
                "{name}"
            );
            assert!((2..=3).all(|arity| descriptor.call_contract().arity().accepts(arity)));
            assert!(!descriptor.call_contract().arity().accepts(1), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(4), "{name}");
        }

        let f_dist = resolve("F.DIST").expect("F.DIST descriptor");
        assert!(matches!(f_dist.evaluator(), Evaluator::Distribution(_)));
        assert!(f_dist.call_contract().arity().accepts(4));
        assert!(!f_dist.call_contract().arity().accepts(3));
        assert!(!f_dist.call_contract().arity().accepts(5));

        for name in ["F.DIST.RT", "FDIST", "F.INV", "FINV"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(4), "{name}");
        }

        for name in ["T.DIST", "TDIST"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(3), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(4), "{name}");
        }

        for name in ["T.DIST.RT", "T.DIST.2T", "T.INV", "T.INV.2T", "TINV"] {
            let descriptor = resolve(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert!(descriptor.call_contract().arity().accepts(2), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(1), "{name}");
            assert!(!descriptor.call_contract().arity().accepts(3), "{name}");
        }
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
        assert!(areas.call_contract().arity().accepts(1));
        assert!(!areas.call_contract().arity().accepts(0));
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

        let is_omitted = descriptor("ISOMITTED").expect("ISOMITTED descriptor");
        assert_eq!(is_omitted.result_kind(), FunctionResultKind::Scalar);
        assert!(is_omitted.catalog_returns_array());
        let lambda = descriptor("LAMBDA").expect("LAMBDA descriptor");
        assert_eq!(lambda.result_kind(), FunctionResultKind::Callable);
        assert!(lambda.catalog_returns_array());
        let let_function = descriptor("LET").expect("LET descriptor");
        assert_eq!(let_function.result_kind(), FunctionResultKind::Contextual);
        assert!(let_function.catalog_returns_array());

        for name in [
            "EXPAND",
            "SORTBY",
            "TOCOL",
            "TOROW",
            "TRIMRANGE",
            "WRAPCOLS",
            "WRAPROWS",
        ] {
            let descriptor = descriptor(name).unwrap_or_else(|| panic!("{name} descriptor"));
            assert_eq!(
                descriptor.minimum_version(),
                CompatibilityVersion::V0_1_10,
                "{name}",
            );
            assert_eq!(
                descriptor.result_kind(),
                FunctionResultKind::Array,
                "{name}"
            );
            assert!(descriptor.catalog_returns_array(), "{name}");
        }
    }
}
