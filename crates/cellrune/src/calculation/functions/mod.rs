use super::ast::Expr;
use super::eval::{Engine, EvalContext};
use super::operators::element_at;
use super::runtime::Array;
use super::value::{ErrorKind, Value};

mod aggregate;
mod array;
mod calendar;
mod combinatorics;
mod date;
mod date_additional;
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

pub(super) fn call_function(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    let normalized = normalize_name(name);
    match function_group(&normalized) {
        Some(FunctionGroup::Legacy) => legacy::call_legacy(engine, context, &normalized, args),
        Some(FunctionGroup::Logical) => logical::call(engine, context, &normalized, args),
        Some(FunctionGroup::Aggregate) => aggregate::call(engine, context, &normalized, args),
        Some(FunctionGroup::Math) => math::call(engine, context, &normalized, args),
        Some(FunctionGroup::Trigonometry) => trigonometry::call(engine, context, &normalized, args),
        Some(FunctionGroup::Combinatorics) => {
            combinatorics::call(engine, context, &normalized, args)
        }
        Some(FunctionGroup::SumOfSquares) => {
            sum_of_squares::call(engine, context, &normalized, args)
        }
        Some(FunctionGroup::Engineering) => engineering::call(engine, context, &normalized, args),
        Some(FunctionGroup::Lookup) => lookup::call(engine, context, &normalized, args),
        Some(FunctionGroup::Information) => information::call(engine, context, &normalized, args),
        Some(FunctionGroup::Text) => text::call(engine, context, &normalized, args),
        Some(FunctionGroup::TextAdditional) => {
            text_additional::call(engine, context, &normalized, args)
        }
        Some(FunctionGroup::Date) => date::call(engine, context, &normalized, args),
        Some(FunctionGroup::DateAdditional) => {
            date_additional::call(engine, context, &normalized, args)
        }
        Some(FunctionGroup::Dynamic) => dynamic::call(engine, context, &normalized, args),
        Some(FunctionGroup::Array) => array::call_scalar(engine, context, &normalized, args),
        Some(FunctionGroup::Statistical) => statistical::call(engine, context, &normalized, args),
        Some(FunctionGroup::StatisticalAdditional) => {
            statistical_additional::call(engine, context, &normalized, args)
        }
        Some(FunctionGroup::Financial) => financial::call(engine, context, &normalized, args),
        Some(FunctionGroup::FinancialAdditional) => {
            financial_additional::call(engine, context, &normalized, args)
        }
        None => Value::Error(ErrorKind::Unsupported),
    }
}

pub(super) fn call_function_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<Array, ErrorKind>> {
    let normalized = normalize_name(name);
    let specialized = match normalized.as_str() {
        "MAP" => Some(dynamic::map_array(engine, context, args)),
        "CHOOSECOLS" | "CHOOSEROWS" | "DROP" | "FILTER" | "HSTACK" | "MMULT" | "SEQUENCE"
        | "SORT" | "TAKE" | "TRANSPOSE" | "UNIQUE" | "VSTACK" => {
            Some(array::call_array(engine, context, &normalized, args))
        }
        "ERROR.TYPE" | "ISBLANK" | "ISERR" | "ISERROR" | "ISEVEN" | "ISLOGICAL" | "ISNA"
        | "ISNONTEXT" | "ISNUMBER" | "ISODD" | "ISTEXT" | "N" | "T" | "TYPE" => {
            information::call_array(engine, context, &normalized, args)
        }
        _ => legacy::call_legacy_array(engine, context, &normalized, args),
    };
    if specialized.is_some() {
        return specialized;
    }
    if ELEMENTWISE_ARRAY_FUNCTIONS.contains(&normalized.as_str()) {
        return Some(call_elementwise_array(engine, context, &normalized, args));
    }
    None
}

pub(super) fn is_supported_function(name: &str) -> bool {
    let normalized = normalize_name(name);
    function_group(&normalized).is_some()
}

pub(super) fn function_catalog() -> Vec<super::FunctionCatalogEntry> {
    let mut entries = FUNCTION_GROUPS
        .iter()
        .flat_map(|(_, names)| names.iter().copied())
        .map(|name| {
            super::FunctionCatalogEntry::new(
                name.to_owned(),
                name.to_owned(),
                false,
                is_array_result_function(name),
                name != "__XLUDF.DUMMYFUNCTION",
            )
        })
        .chain(LEGACY_ALIASES.iter().map(|(alias, canonical)| {
            super::FunctionCatalogEntry::new(
                (*alias).to_owned(),
                (*canonical).to_owned(),
                true,
                is_array_result_function(canonical),
                true,
            )
        }))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name().cmp(right.name()));
    entries
}

fn is_array_result_function(name: &str) -> bool {
    DYNAMIC_FUNCTIONS.contains(&name)
        || ARRAY_FUNCTIONS.contains(&name)
        || LEGACY_ARRAY_FUNCTIONS.contains(&name)
        || INFORMATION_ARRAY_FUNCTIONS.contains(&name)
        || ELEMENTWISE_ARRAY_FUNCTIONS.contains(&name)
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
    engine.ensure_function_iterations(
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

const LEGACY_ARRAY_FUNCTIONS: &[&str] = &["IF", "COUNTIF", "COUNTIFS", "INDEX"];
const INFORMATION_ARRAY_FUNCTIONS: &[&str] = &[
    "ERROR.TYPE",
    "ISBLANK",
    "ISERR",
    "ISERROR",
    "ISEVEN",
    "ISLOGICAL",
    "ISNA",
    "ISNONTEXT",
    "ISNUMBER",
    "ISODD",
    "ISTEXT",
    "N",
    "T",
    "TYPE",
];
const ELEMENTWISE_ARRAY_FUNCTIONS: &[&str] = &["ABS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionGroup {
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
}

fn function_group(name: &str) -> Option<FunctionGroup> {
    FUNCTION_GROUPS
        .iter()
        .find_map(|(group, names)| names.contains(&name).then_some(*group))
}

const LEGACY_FUNCTIONS: &[&str] = &[
    "IF",
    "AND",
    "IFERROR",
    "LOWER",
    "TEXT",
    "COUNTIF",
    "COUNTIFS",
    "SUMPRODUCT",
    "INDEX",
    "MATCH",
    "__XLUDF.DUMMYFUNCTION",
];
const LOGICAL_FUNCTIONS: &[&str] = &["TRUE", "FALSE", "NOT", "OR", "XOR", "IFNA", "IFS", "SWITCH"];
const AGGREGATE_FUNCTIONS: &[&str] = &[
    "SUM",
    "AVERAGE",
    "MIN",
    "MAX",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "PRODUCT",
    "SUBTOTAL",
    "SUMIF",
    "SUMIFS",
    "AVERAGEIF",
    "AVERAGEIFS",
];
const MATH_FUNCTIONS: &[&str] = &[
    "ABS",
    "BASE",
    "CEILING",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "DECIMAL",
    "EVEN",
    "EXP",
    "FLOOR",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "INT",
    "ISO.CEILING",
    "LN",
    "LOG",
    "LOG10",
    "MOD",
    "MROUND",
    "ODD",
    "PI",
    "POWER",
    "QUOTIENT",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "SERIESSUM",
    "SIGN",
    "SQRT",
    "SQRTPI",
    "TRUNC",
];
const TRIGONOMETRY_FUNCTIONS: &[&str] = &[
    "ACOS", "ACOSH", "ACOT", "ACOTH", "ASIN", "ASINH", "ATAN", "ATAN2", "ATANH", "COS", "COSH",
    "COT", "COTH", "CSC", "CSCH", "DEGREES", "RADIANS", "SEC", "SECH", "SIN", "SINH", "TAN",
    "TANH",
];
const COMBINATORICS_FUNCTIONS: &[&str] = &[
    "COMBIN",
    "COMBINA",
    "FACT",
    "FACTDOUBLE",
    "GCD",
    "LCM",
    "MULTINOMIAL",
    "PERMUT",
    "PERMUTATIONA",
];
const SUM_OF_SQUARES_FUNCTIONS: &[&str] = &["SUMSQ", "SUMX2MY2", "SUMX2PY2", "SUMXMY2"];
const ENGINEERING_FUNCTIONS: &[&str] = &[
    "BIN2DEC",
    "BIN2HEX",
    "BIN2OCT",
    "BITAND",
    "BITLSHIFT",
    "BITOR",
    "BITRSHIFT",
    "BITXOR",
    "DEC2BIN",
    "DEC2HEX",
    "DEC2OCT",
    "DELTA",
    "ERF",
    "ERF.PRECISE",
    "ERFC",
    "ERFC.PRECISE",
    "GESTEP",
    "HEX2BIN",
    "HEX2DEC",
    "HEX2OCT",
    "OCT2BIN",
    "OCT2DEC",
    "OCT2HEX",
];
const LOOKUP_FUNCTIONS: &[&str] = &[
    "ADDRESS",
    "CHOOSE",
    "COLUMN",
    "COLUMNS",
    "HLOOKUP",
    "HYPERLINK",
    "INDIRECT",
    "LOOKUP",
    "OFFSET",
    "ROWS",
    "ROW",
    "VLOOKUP",
    "XLOOKUP",
];
const INFORMATION_FUNCTIONS: &[&str] = &[
    "ERROR.TYPE",
    "ISBLANK",
    "ISERR",
    "ISERROR",
    "ISEVEN",
    "ISLOGICAL",
    "ISNA",
    "ISNONTEXT",
    "ISNUMBER",
    "ISODD",
    "ISREF",
    "ISTEXT",
    "N",
    "NA",
    "T",
    "TYPE",
];
const TEXT_FUNCTIONS: &[&str] = &[
    "CONCAT",
    "EXACT",
    "FIND",
    "LEFT",
    "LEN",
    "MID",
    "PROPER",
    "REPLACE",
    "REPT",
    "RIGHT",
    "SEARCH",
    "SUBSTITUTE",
    "TEXTJOIN",
    "TRIM",
    "UPPER",
];
const TEXT_ADDITIONAL_FUNCTIONS: &[&str] = &[
    "CHAR",
    "CLEAN",
    "CONCATENATE",
    "DOLLAR",
    "TEXTAFTER",
    "TEXTBEFORE",
    "UNICHAR",
    "UNICODE",
    "VALUE",
    "VALUETOTEXT",
];
const DATE_FUNCTIONS: &[&str] = &[
    "DATE",
    "DATEDIF",
    "DAY",
    "EDATE",
    "EOMONTH",
    "MONTH",
    "NETWORKDAYS",
    "NOW",
    "TODAY",
    "WEEKDAY",
    "WORKDAY",
    "YEAR",
    "YEARFRAC",
];
const DATE_ADDITIONAL_FUNCTIONS: &[&str] = &[
    "DAYS",
    "DAYS360",
    "HOUR",
    "ISOWEEKNUM",
    "MINUTE",
    "SECOND",
    "TIME",
    "WEEKNUM",
];
const DYNAMIC_FUNCTIONS: &[&str] = &["MAP"];
const ARRAY_FUNCTIONS: &[&str] = &[
    "CHOOSECOLS",
    "CHOOSEROWS",
    "DROP",
    "FILTER",
    "HSTACK",
    "MMULT",
    "SEQUENCE",
    "SORT",
    "TAKE",
    "TRANSPOSE",
    "UNIQUE",
    "VSTACK",
];
const STATISTICAL_FUNCTIONS: &[&str] = &[
    "CORREL",
    "COVARIANCE.P",
    "INTERCEPT",
    "LARGE",
    "MAXIFS",
    "MEDIAN",
    "MINIFS",
    "MODE.SNGL",
    "NORMSDIST",
    "NORM.S.DIST",
    "PEARSON",
    "PERCENTILE.INC",
    "PERCENTRANK.INC",
    "QUARTILE.INC",
    "RANK.EQ",
    "RSQ",
    "SLOPE",
    "SMALL",
    "STDEV.S",
    "VAR.S",
];
const STATISTICAL_ADDITIONAL_FUNCTIONS: &[&str] = &[
    "AVEDEV",
    "AVERAGEA",
    "DEVSQ",
    "EXPON.DIST",
    "GAUSS",
    "GEOMEAN",
    "HARMEAN",
    "MAXA",
    "MINA",
    "NORM.DIST",
    "PHI",
    "POISSON.DIST",
    "STANDARDIZE",
    "STDEV.P",
    "VAR.P",
];
const FINANCIAL_FUNCTIONS: &[&str] = &[
    "DB", "FV", "IPMT", "IRR", "NPER", "NPV", "PMT", "PPMT", "PV", "RATE", "SLN", "SYD", "XIRR",
];
const FINANCIAL_ADDITIONAL_FUNCTIONS: &[&str] = &[
    "DOLLARDE",
    "DOLLARFR",
    "EFFECT",
    "FVSCHEDULE",
    "ISPMT",
    "MIRR",
    "NOMINAL",
    "PDURATION",
    "RRI",
];

const FUNCTION_GROUPS: &[(FunctionGroup, &[&str])] = &[
    (FunctionGroup::Legacy, LEGACY_FUNCTIONS),
    (FunctionGroup::Logical, LOGICAL_FUNCTIONS),
    (FunctionGroup::Aggregate, AGGREGATE_FUNCTIONS),
    (FunctionGroup::Math, MATH_FUNCTIONS),
    (FunctionGroup::Trigonometry, TRIGONOMETRY_FUNCTIONS),
    (FunctionGroup::Combinatorics, COMBINATORICS_FUNCTIONS),
    (FunctionGroup::SumOfSquares, SUM_OF_SQUARES_FUNCTIONS),
    (FunctionGroup::Engineering, ENGINEERING_FUNCTIONS),
    (FunctionGroup::Lookup, LOOKUP_FUNCTIONS),
    (FunctionGroup::Information, INFORMATION_FUNCTIONS),
    (FunctionGroup::Text, TEXT_FUNCTIONS),
    (FunctionGroup::TextAdditional, TEXT_ADDITIONAL_FUNCTIONS),
    (FunctionGroup::Date, DATE_FUNCTIONS),
    (FunctionGroup::DateAdditional, DATE_ADDITIONAL_FUNCTIONS),
    (FunctionGroup::Dynamic, DYNAMIC_FUNCTIONS),
    (FunctionGroup::Array, ARRAY_FUNCTIONS),
    (FunctionGroup::Statistical, STATISTICAL_FUNCTIONS),
    (
        FunctionGroup::StatisticalAdditional,
        STATISTICAL_ADDITIONAL_FUNCTIONS,
    ),
    (FunctionGroup::Financial, FINANCIAL_FUNCTIONS),
    (
        FunctionGroup::FinancialAdditional,
        FINANCIAL_ADDITIONAL_FUNCTIONS,
    ),
];

const LEGACY_ALIASES: &[(&str, &str)] = &[
    ("COVAR", "COVARIANCE.P"),
    ("EXPONDIST", "EXPON.DIST"),
    ("MODE", "MODE.SNGL"),
    ("NORMDIST", "NORM.DIST"),
    ("PERCENTILE", "PERCENTILE.INC"),
    ("PERCENTRANK", "PERCENTRANK.INC"),
    ("POISSON", "POISSON.DIST"),
    ("QUARTILE", "QUARTILE.INC"),
    ("RANK", "RANK.EQ"),
    ("STDEV", "STDEV.S"),
    ("STDEVP", "STDEV.P"),
    ("VAR", "VAR.S"),
    ("VARP", "VAR.P"),
];

pub(super) fn normalize_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let mut base = upper.as_str();
    while let Some(stripped) = base
        .strip_prefix("_XLFN.")
        .or_else(|| base.strip_prefix("_XLUDF."))
        .or_else(|| base.strip_prefix("_XLWS."))
    {
        base = stripped;
    }
    LEGACY_ALIASES
        .iter()
        .find_map(|(alias, target)| (*alias == base).then_some(*target))
        .unwrap_or(base)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FUNCTION_GROUPS, LEGACY_ALIASES, function_group, normalize_name};

    #[test]
    fn normalization_removes_composed_excel_storage_prefixes() {
        assert_eq!(normalize_name("_xlfn._xlws.FILTER"), "FILTER");
        assert_eq!(normalize_name("_xludf._xlfn.COVAR"), "COVARIANCE.P");
        assert_eq!(normalize_name("_XLWS._XLUDF._XLFN.SUM"), "SUM");
    }

    #[test]
    fn coverage_registry_has_278_unique_excel_facing_names() {
        let kernels: BTreeSet<&str> = FUNCTION_GROUPS
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        let registered_kernel_count: usize =
            FUNCTION_GROUPS.iter().map(|(_, names)| names.len()).sum();
        assert_eq!(kernels.len(), registered_kernel_count);
        assert!(kernels.contains("__XLUDF.DUMMYFUNCTION"));
        assert_eq!(kernels.len(), 266);

        let aliases: BTreeSet<&str> = LEGACY_ALIASES.iter().map(|(alias, _)| *alias).collect();
        assert_eq!(aliases.len(), LEGACY_ALIASES.len());
        assert!(aliases.is_disjoint(&kernels));
        assert!(
            LEGACY_ALIASES
                .iter()
                .all(|(_, target)| function_group(target).is_some())
        );

        let official_kernels = kernels.len() - 1;
        assert_eq!(official_kernels + aliases.len(), 278);

        let catalog = super::function_catalog();
        assert_eq!(catalog.len(), kernels.len() + aliases.len());
        assert!(
            catalog
                .windows(2)
                .all(|pair| pair[0].name() < pair[1].name())
        );
    }
}
