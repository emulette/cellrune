//! Functions Excel stores behind a future-function prefix.
//!
//! Excel saves functions introduced after the Excel 2007 file format as `_xlfn.NAME`, and FILTER
//! and SORT as `_xlfn._xlws.NAME`. A bare spelling loads as an unknown user-defined function and
//! becomes `#NAME?` once Excel recalculates. Every name below was saved with its prefix by Excel
//! 2021, Excel for the web, or Excel for Mac. ISO.CEILING, NETWORKDAYS.INTL, and WORKDAY.INTL are
//! absent because Excel saves them bare. The list includes functions the catalog cannot calculate,
//! since formulas that use them are still written.

const FUTURE_FUNCTIONS: [&str; 159] = [
    "ACOT",
    "ACOTH",
    "AGGREGATE",
    "ARABIC",
    "ARRAYTOTEXT",
    "BASE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.DIST.RANGE",
    "BINOM.INV",
    "BITAND",
    "BITLSHIFT",
    "BITOR",
    "BITRSHIFT",
    "BITXOR",
    "BYCOL",
    "BYROW",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "COMBINA",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COT",
    "COTH",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "CSC",
    "CSCH",
    "DAYS",
    "DECIMAL",
    "DETECTLANGUAGE",
    "DROP",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST",
    "F.DIST.RT",
    "F.INV",
    "F.INV.RT",
    "F.TEST",
    "FIELDVALUE",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY",
    "FORECAST.ETS.STAT",
    "FORECAST.LINEAR",
    "FORMULATEXT",
    "GAMMA",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "GAUSS",
    "GROUPBY",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFNA",
    "IFS",
    "IMAGE",
    "IMCOSH",
    "IMCOT",
    "IMCSC",
    "IMCSCH",
    "IMSEC",
    "IMSECH",
    "IMSINH",
    "IMTAN",
    "ISFORMULA",
    "ISOMITTED",
    "ISOWEEKNUM",
    "LAMBDA",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAKEARRAY",
    "MAP",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "MUNIT",
    "NEGBINOM.DIST",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "NUMBERVALUE",
    "PDURATION",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTOF",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "PERMUTATIONA",
    "PHI",
    "PIVOTBY",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "REDUCE",
    "REGEXEXTRACT",
    "REGEXREPLACE",
    "REGEXTEST",
    "RRI",
    "SCAN",
    "SEC",
    "SECH",
    "SEQUENCE",
    "SHEET",
    "SHEETS",
    "SKEW.P",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "STOCKHISTORY",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "TRANSLATE",
    "TRIMRANGE",
    "UNICHAR",
    "UNICODE",
    "UNIQUE",
    "VALUETOTEXT",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEIBULL.DIST",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "XOR",
    "Z.TEST",
];

const WORKSHEET_FUTURE_FUNCTIONS: [&str; 2] = ["FILTER", "SORT"];

/// Returns the prefix Excel stores in front of a bare function name, if it needs one.
pub(in crate::calculation) fn future_function_prefix(name: &str) -> Option<&'static str> {
    let upper = name.to_ascii_uppercase();
    if WORKSHEET_FUTURE_FUNCTIONS.contains(&upper.as_str()) {
        Some("_xlfn._xlws.")
    } else if FUTURE_FUNCTIONS.binary_search(&upper.as_str()).is_ok() {
        Some("_xlfn.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{FUTURE_FUNCTIONS, WORKSHEET_FUTURE_FUNCTIONS};
    use crate::calculation::functions::descriptor;

    // Future functions Excel saved with `_xlfn.` that the catalog does not calculate yet.
    const OUTSIDE_CATALOG: [&str; 11] = [
        "AGGREGATE",
        "DETECTLANGUAGE",
        "FIELDVALUE",
        "FORECAST.ETS",
        "FORECAST.ETS.CONFINT",
        "FORECAST.ETS.SEASONALITY",
        "FORECAST.ETS.STAT",
        "IMAGE",
        "IMCOSH",
        "STOCKHISTORY",
        "TRANSLATE",
    ];

    #[test]
    fn future_functions_are_unique_and_catalog_names_resolve() {
        assert!(FUTURE_FUNCTIONS.is_sorted_by(|left, right| left < right));
        for name in WORKSHEET_FUTURE_FUNCTIONS {
            assert!(FUTURE_FUNCTIONS.binary_search(&name).is_err());
        }
        for name in OUTSIDE_CATALOG {
            assert!(FUTURE_FUNCTIONS.binary_search(&name).is_ok(), "{name}");
            assert!(
                descriptor::resolve(name).is_none(),
                "{name} is in the catalog now"
            );
        }
        for name in FUTURE_FUNCTIONS
            .iter()
            .chain(&WORKSHEET_FUTURE_FUNCTIONS)
            .filter(|name| !OUTSIDE_CATALOG.contains(name))
        {
            assert!(
                descriptor::descriptor(name).is_some(),
                "{name} is not a canonical catalog function"
            );
        }
    }
}
