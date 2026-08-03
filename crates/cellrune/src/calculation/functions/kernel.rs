macro_rules! function_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(in crate::calculation) enum $name {
            $($variant),+
        }

        impl $name {
            #[cfg(test)]
            pub(super) const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[cfg(test)]
            pub(super) const fn stable_name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant)),+
                }
            }
        }
    };
}

function_enum!(LegacyFunction {
    If,
    And,
    IfError,
    Lower,
    Text,
    CountIf,
    CountIfs,
    SumProduct,
    Index,
    Match,
    DummyFunction,
});

function_enum!(LogicalFunction {
    True,
    False,
    Not,
    Or,
    Xor,
    IfNa,
    Ifs,
    Switch,
});

function_enum!(AggregateFunction {
    Sum,
    Average,
    Min,
    Max,
    Count,
    CountA,
    CountBlank,
    Product,
    Subtotal,
    SumIf,
    SumIfs,
    AverageIf,
    AverageIfs,
});

function_enum!(GroupedFunction {
    GroupBy,
    PercentOf,
    PivotBy,
});

function_enum!(DatabaseFunction {
    Average,
    Count,
    CountA,
    Get,
    Max,
    Min,
    Product,
    StDev,
    StDevP,
    Sum,
    Var,
    VarP,
});

function_enum!(MathFunction {
    Abs,
    Base,
    Ceiling,
    CeilingMath,
    CeilingPrecise,
    Decimal,
    Even,
    Exp,
    Floor,
    FloorMath,
    FloorPrecise,
    Int,
    IsoCeiling,
    Ln,
    Log,
    Log10,
    Mod,
    MRound,
    Odd,
    Pi,
    Power,
    Quotient,
    Round,
    RoundDown,
    RoundUp,
    SeriesSum,
    Sign,
    Sqrt,
    SqrtPi,
    Trunc,
});

function_enum!(RomanFunction { Arabic, Roman });

function_enum!(TrigonometryFunction {
    Acos,
    Acosh,
    Acot,
    Acoth,
    Asin,
    Asinh,
    Atan,
    Atan2,
    Atanh,
    Cos,
    Cosh,
    Cot,
    Coth,
    Csc,
    Csch,
    Degrees,
    Radians,
    Sec,
    Sech,
    Sin,
    Sinh,
    Tan,
    Tanh,
});

function_enum!(CombinatoricsFunction {
    Combin,
    Combina,
    Fact,
    FactDouble,
    Gcd,
    Lcm,
    Multinomial,
    Permut,
    PermutationA,
});

function_enum!(SumOfSquaresFunction {
    SumSq,
    SumX2My2,
    SumX2Py2,
    SumXMy2,
});

function_enum!(EngineeringFunction {
    Bin2Dec,
    Bin2Hex,
    Bin2Oct,
    BitAnd,
    BitLShift,
    BitOr,
    BitRShift,
    BitXor,
    Dec2Bin,
    Dec2Hex,
    Dec2Oct,
    Delta,
    Erf,
    ErfPrecise,
    Erfc,
    ErfcPrecise,
    GeStep,
    Hex2Bin,
    Hex2Dec,
    Hex2Oct,
    Oct2Bin,
    Oct2Dec,
    Oct2Hex,
});

function_enum!(LookupFunction {
    Address,
    Choose,
    Column,
    Columns,
    HLookup,
    Hyperlink,
    Indirect,
    Lookup,
    Offset,
    Rows,
    Row,
    Sheet,
    Sheets,
    VLookup,
    XMatch,
    XLookup,
});

function_enum!(InformationFunction {
    ErrorType,
    FormulaText,
    IsBlank,
    IsErr,
    IsError,
    IsEven,
    IsLogical,
    IsNa,
    IsNonText,
    IsNumber,
    IsOdd,
    IsFormula,
    IsRef,
    IsText,
    N,
    Na,
    T,
    Type,
});

function_enum!(TextFunction {
    Concat,
    Exact,
    Find,
    Left,
    Len,
    Mid,
    Proper,
    Replace,
    Rept,
    Right,
    Search,
    Substitute,
    TextJoin,
    Trim,
    Upper,
});

function_enum!(TextAdditionalFunction {
    Char,
    Clean,
    Concatenate,
    Dollar,
    TextAfter,
    TextBefore,
    UniChar,
    Unicode,
    Value,
    ValueToText,
});

function_enum!(ModernTextFunction {
    ArrayToText,
    RegexExtract,
    RegexReplace,
    RegexTest,
    TextSplit,
});

function_enum!(DateFunction {
    Date,
    DateDif,
    Day,
    EDate,
    Eomonth,
    Month,
    NetworkDays,
    Now,
    Today,
    Weekday,
    Workday,
    Year,
    YearFrac,
});

function_enum!(DateAdditionalFunction {
    Days,
    Days360,
    Hour,
    IsoWeekNum,
    Minute,
    Second,
    Time,
    WeekNum,
});

function_enum!(DynamicFunction {
    ByCol,
    ByRow,
    IsOmitted,
    Lambda,
    Let,
    MakeArray,
    Map,
    Reduce,
    Scan,
});

function_enum!(ArrayFunction {
    ChooseCols,
    ChooseRows,
    Drop,
    Expand,
    Filter,
    HStack,
    MInverse,
    MMult,
    MUnit,
    Sequence,
    Sort,
    SortBy,
    Take,
    ToCol,
    ToRow,
    Transpose,
    TrimRange,
    Unique,
    VStack,
    WrapCols,
    WrapRows,
});

function_enum!(RegressionFunction {
    Growth,
    LinEst,
    LogEst,
    Trend,
});

function_enum!(StatisticalFunction {
    Correl,
    CovarianceP,
    Intercept,
    Large,
    MaxIfs,
    Median,
    MinIfs,
    ModeSingle,
    NormSDistLegacy,
    NormSDist,
    Pearson,
    PercentileInc,
    PercentRankInc,
    QuartileInc,
    RankEq,
    Rsq,
    Slope,
    Small,
    StDevS,
    VarS,
});

function_enum!(StatisticalAdditionalFunction {
    AveDev,
    AverageA,
    DevSq,
    ExponDist,
    Gauss,
    GeoMean,
    HarMean,
    MaxA,
    MinA,
    NormDist,
    Phi,
    PoissonDist,
    Standardize,
    StDevP,
    VarP,
});

function_enum!(DistributionFunction {
    BinomDist,
    BinomDistRange,
    BinomInv,
    Gamma,
    GammaDist,
    GammaInv,
    GammaLnPrecise,
    NegBinomDist,
    NegBinomDistLegacy,
});

function_enum!(FinancialFunction {
    Db,
    Fv,
    Ipmt,
    Irr,
    Nper,
    Npv,
    Pmt,
    Ppmt,
    Pv,
    Rate,
    Sln,
    Syd,
    Xirr,
});

function_enum!(FinancialAdditionalFunction {
    DollarDe,
    DollarFr,
    Effect,
    FvSchedule,
    IsPmt,
    Mirr,
    Nominal,
    PDuration,
    Rri,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::calculation) enum Evaluator {
    Legacy(LegacyFunction),
    Logical(LogicalFunction),
    Aggregate(AggregateFunction),
    Grouped(GroupedFunction),
    Database(DatabaseFunction),
    Math(MathFunction),
    Roman(RomanFunction),
    Trigonometry(TrigonometryFunction),
    Combinatorics(CombinatoricsFunction),
    SumOfSquares(SumOfSquaresFunction),
    Engineering(EngineeringFunction),
    Lookup(LookupFunction),
    Information(InformationFunction),
    Text(TextFunction),
    TextAdditional(TextAdditionalFunction),
    ModernText(ModernTextFunction),
    Date(DateFunction),
    DateAdditional(DateAdditionalFunction),
    Dynamic(DynamicFunction),
    Array(ArrayFunction),
    Regression(RegressionFunction),
    Statistical(StatisticalFunction),
    StatisticalAdditional(StatisticalAdditionalFunction),
    Distribution(DistributionFunction),
    Financial(FinancialFunction),
    FinancialAdditional(FinancialAdditionalFunction),
    Areas,
}

#[cfg(test)]
impl Evaluator {
    pub(super) fn stable_name(self) -> String {
        match self {
            Self::Legacy(value) => format!("legacy:{}", value.stable_name()),
            Self::Logical(value) => format!("logical:{}", value.stable_name()),
            Self::Aggregate(value) => format!("aggregate:{}", value.stable_name()),
            Self::Grouped(value) => format!("grouped:{}", value.stable_name()),
            Self::Database(value) => format!("database:{}", value.stable_name()),
            Self::Math(value) => format!("math:{}", value.stable_name()),
            Self::Roman(value) => format!("roman:{}", value.stable_name()),
            Self::Trigonometry(value) => format!("trigonometry:{}", value.stable_name()),
            Self::Combinatorics(value) => format!("combinatorics:{}", value.stable_name()),
            Self::SumOfSquares(value) => format!("sum_of_squares:{}", value.stable_name()),
            Self::Engineering(value) => format!("engineering:{}", value.stable_name()),
            Self::Lookup(value) => format!("lookup:{}", value.stable_name()),
            Self::Information(value) => format!("information:{}", value.stable_name()),
            Self::Text(value) => format!("text:{}", value.stable_name()),
            Self::TextAdditional(value) => format!("text_additional:{}", value.stable_name()),
            Self::ModernText(value) => format!("modern_text:{}", value.stable_name()),
            Self::Date(value) => format!("date:{}", value.stable_name()),
            Self::DateAdditional(value) => format!("date_additional:{}", value.stable_name()),
            Self::Dynamic(value) => format!("dynamic:{}", value.stable_name()),
            Self::Array(value) => format!("array:{}", value.stable_name()),
            Self::Regression(value) => format!("regression:{}", value.stable_name()),
            Self::Statistical(value) => format!("statistical:{}", value.stable_name()),
            Self::StatisticalAdditional(value) => {
                format!("statistical_additional:{}", value.stable_name())
            }
            Self::Distribution(value) => format!("distribution:{}", value.stable_name()),
            Self::Financial(value) => format!("financial:{}", value.stable_name()),
            Self::FinancialAdditional(value) => {
                format!("financial_additional:{}", value.stable_name())
            }
            Self::Areas => "areas".to_owned(),
        }
    }

    pub(super) fn all() -> Vec<Self> {
        let mut evaluators = Vec::new();
        evaluators.extend(LegacyFunction::ALL.iter().copied().map(Self::Legacy));
        evaluators.extend(LogicalFunction::ALL.iter().copied().map(Self::Logical));
        evaluators.extend(AggregateFunction::ALL.iter().copied().map(Self::Aggregate));
        evaluators.extend(GroupedFunction::ALL.iter().copied().map(Self::Grouped));
        evaluators.extend(DatabaseFunction::ALL.iter().copied().map(Self::Database));
        evaluators.extend(MathFunction::ALL.iter().copied().map(Self::Math));
        evaluators.extend(RomanFunction::ALL.iter().copied().map(Self::Roman));
        evaluators.extend(
            TrigonometryFunction::ALL
                .iter()
                .copied()
                .map(Self::Trigonometry),
        );
        evaluators.extend(
            CombinatoricsFunction::ALL
                .iter()
                .copied()
                .map(Self::Combinatorics),
        );
        evaluators.extend(
            SumOfSquaresFunction::ALL
                .iter()
                .copied()
                .map(Self::SumOfSquares),
        );
        evaluators.extend(
            EngineeringFunction::ALL
                .iter()
                .copied()
                .map(Self::Engineering),
        );
        evaluators.extend(LookupFunction::ALL.iter().copied().map(Self::Lookup));
        evaluators.extend(
            InformationFunction::ALL
                .iter()
                .copied()
                .map(Self::Information),
        );
        evaluators.extend(TextFunction::ALL.iter().copied().map(Self::Text));
        evaluators.extend(
            TextAdditionalFunction::ALL
                .iter()
                .copied()
                .map(Self::TextAdditional),
        );
        evaluators.extend(
            ModernTextFunction::ALL
                .iter()
                .copied()
                .map(Self::ModernText),
        );
        evaluators.extend(DateFunction::ALL.iter().copied().map(Self::Date));
        evaluators.extend(
            DateAdditionalFunction::ALL
                .iter()
                .copied()
                .map(Self::DateAdditional),
        );
        evaluators.extend(DynamicFunction::ALL.iter().copied().map(Self::Dynamic));
        evaluators.extend(ArrayFunction::ALL.iter().copied().map(Self::Array));
        evaluators.extend(
            RegressionFunction::ALL
                .iter()
                .copied()
                .map(Self::Regression),
        );
        evaluators.extend(
            StatisticalFunction::ALL
                .iter()
                .copied()
                .map(Self::Statistical),
        );
        evaluators.extend(
            StatisticalAdditionalFunction::ALL
                .iter()
                .copied()
                .map(Self::StatisticalAdditional),
        );
        evaluators.extend(
            DistributionFunction::ALL
                .iter()
                .copied()
                .map(Self::Distribution),
        );
        evaluators.extend(FinancialFunction::ALL.iter().copied().map(Self::Financial));
        evaluators.extend(
            FinancialAdditionalFunction::ALL
                .iter()
                .copied()
                .map(Self::FinancialAdditional),
        );
        evaluators.push(Self::Areas);
        evaluators
    }
}

function_enum!(LegacyArrayFunction {
    If,
    CountIf,
    CountIfs,
    Index,
});

function_enum!(InformationArrayFunction {
    ErrorType,
    IsBlank,
    IsErr,
    IsError,
    IsEven,
    IsLogical,
    IsNa,
    IsNonText,
    IsNumber,
    IsOdd,
    IsText,
    N,
    T,
    Type,
});

impl InformationArrayFunction {
    pub(super) const fn scalar_function(self) -> InformationFunction {
        match self {
            Self::ErrorType => InformationFunction::ErrorType,
            Self::IsBlank => InformationFunction::IsBlank,
            Self::IsErr => InformationFunction::IsErr,
            Self::IsError => InformationFunction::IsError,
            Self::IsEven => InformationFunction::IsEven,
            Self::IsLogical => InformationFunction::IsLogical,
            Self::IsNa => InformationFunction::IsNa,
            Self::IsNonText => InformationFunction::IsNonText,
            Self::IsNumber => InformationFunction::IsNumber,
            Self::IsOdd => InformationFunction::IsOdd,
            Self::IsText => InformationFunction::IsText,
            Self::N => InformationFunction::N,
            Self::T => InformationFunction::T,
            Self::Type => InformationFunction::Type,
        }
    }
}

function_enum!(DynamicArrayFunction {
    ByCol,
    ByRow,
    MakeArray,
    Reduce,
    Scan,
});

function_enum!(ElementwiseArrayFunction { Abs });

function_enum!(ModernTextArrayFunction {
    RegexExtract,
    TextSplit,
});

function_enum!(GroupedArrayFunction { GroupBy, PivotBy });

impl ElementwiseArrayFunction {
    pub(super) const fn scalar_function(self) -> MathFunction {
        match self {
            Self::Abs => MathFunction::Abs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::calculation) enum ArrayEvaluator {
    Legacy(LegacyArrayFunction),
    Information(InformationArrayFunction),
    Elementwise(ElementwiseArrayFunction),
    Dynamic(DynamicArrayFunction),
    Map,
    Array(ArrayFunction),
    Regression(RegressionFunction),
    ModernText(ModernTextArrayFunction),
    Grouped(GroupedArrayFunction),
}

#[cfg(test)]
impl ArrayEvaluator {
    pub(super) fn stable_name(self) -> String {
        match self {
            Self::Legacy(value) => format!("legacy:{}", value.stable_name()),
            Self::Information(value) => format!("information:{}", value.stable_name()),
            Self::Elementwise(value) => format!("elementwise:{}", value.stable_name()),
            Self::Dynamic(value) => format!("dynamic:{}", value.stable_name()),
            Self::Map => "map".to_owned(),
            Self::Array(value) => format!("array:{}", value.stable_name()),
            Self::Regression(value) => format!("regression:{}", value.stable_name()),
            Self::ModernText(value) => format!("modern_text:{}", value.stable_name()),
            Self::Grouped(value) => format!("grouped:{}", value.stable_name()),
        }
    }

    pub(super) fn all() -> Vec<Self> {
        let mut evaluators = Vec::new();
        evaluators.extend(LegacyArrayFunction::ALL.iter().copied().map(Self::Legacy));
        evaluators.extend(
            InformationArrayFunction::ALL
                .iter()
                .copied()
                .map(Self::Information),
        );
        evaluators.extend(
            ElementwiseArrayFunction::ALL
                .iter()
                .copied()
                .map(Self::Elementwise),
        );
        evaluators.extend(DynamicArrayFunction::ALL.iter().copied().map(Self::Dynamic));
        evaluators.push(Self::Map);
        evaluators.extend(ArrayFunction::ALL.iter().copied().map(Self::Array));
        evaluators.extend(
            RegressionFunction::ALL
                .iter()
                .copied()
                .map(Self::Regression),
        );
        evaluators.extend(
            ModernTextArrayFunction::ALL
                .iter()
                .copied()
                .map(Self::ModernText),
        );
        evaluators.extend(GroupedArrayFunction::ALL.iter().copied().map(Self::Grouped));
        evaluators
    }
}
