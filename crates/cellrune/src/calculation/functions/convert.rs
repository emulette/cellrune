//! Typed unit registry and worksheet adapter for `CONVERT`.
//!
//! The registry is deliberately compiled into the evaluator.  The prep JSON is an input
//! artifact, not a runtime resource, so token lookup remains deterministic and cannot drift
//! with a workbook or process-local file.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::{required_number, required_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum UnitDimension {
    Area,
    Distance,
    Energy,
    Force,
    Information,
    Magnetism,
    Mass,
    Power,
    Pressure,
    Speed,
    Temperature,
    Time,
    Volume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum PrefixPolicy {
    None,
    Decimal,
    DecimalBinary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UnitEntry {
    id: &'static str,
    dimension: UnitDimension,
    tokens: &'static [&'static str],
    scale_to_base: f64,
    offset_to_base: f64,
    prefix_policy: PrefixPolicy,
    prefix_power: u8,
}

impl UnitEntry {
    const fn new(
        dimension: UnitDimension,
        id: &'static str,
        tokens: &'static [&'static str],
        scale_to_base: f64,
        offset_to_base: f64,
        prefix_policy: PrefixPolicy,
        prefix_power: u8,
    ) -> Self {
        Self {
            id,
            dimension,
            tokens,
            scale_to_base,
            offset_to_base,
            prefix_policy,
            prefix_power,
        }
    }

    #[cfg(test)]
    pub(super) const fn id(self) -> &'static str {
        self.id
    }

    #[cfg(test)]
    pub(super) const fn dimension(self) -> UnitDimension {
        self.dimension
    }

    #[cfg(test)]
    pub(super) const fn canonical_token(self) -> &'static str {
        self.tokens[0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixKind {
    Decimal,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Prefix {
    spelling: &'static str,
    multiplier: f64,
    kind: PrefixKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ParsedUnit {
    entry: &'static UnitEntry,
    prefix_multiplier: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnitParseError {
    UnknownToken,
    ForbiddenPrefix,
}

const DECIMAL_PREFIXES: &[Prefix] = &[
    Prefix {
        spelling: "da",
        multiplier: 1e1,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "Y",
        multiplier: 1e24,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "Z",
        multiplier: 1e21,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "E",
        multiplier: 1e18,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "P",
        multiplier: 1e15,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "T",
        multiplier: 1e12,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "G",
        multiplier: 1e9,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "M",
        multiplier: 1e6,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "k",
        multiplier: 1e3,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "h",
        multiplier: 1e2,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "e",
        multiplier: 1e1,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "d",
        multiplier: 1e-1,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "c",
        multiplier: 1e-2,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "m",
        multiplier: 1e-3,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "u",
        multiplier: 1e-6,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "n",
        multiplier: 1e-9,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "p",
        multiplier: 1e-12,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "f",
        multiplier: 1e-15,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "a",
        multiplier: 1e-18,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "z",
        multiplier: 1e-21,
        kind: PrefixKind::Decimal,
    },
    Prefix {
        spelling: "y",
        multiplier: 1e-24,
        kind: PrefixKind::Decimal,
    },
];

// Excel's CONVERT spelling is lower-case `ki`; `Ki` is intentionally not accepted.
const BINARY_PREFIXES: &[Prefix] = &[
    Prefix {
        spelling: "Yi",
        multiplier: 1208925819614629174706176.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Zi",
        multiplier: 1180591620717411303424.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Ei",
        multiplier: 1152921504606846976.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Pi",
        multiplier: 1125899906842624.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Ti",
        multiplier: 1099511627776.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Gi",
        multiplier: 1073741824.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "Mi",
        multiplier: 1048576.0,
        kind: PrefixKind::Binary,
    },
    Prefix {
        spelling: "ki",
        multiplier: 1024.0,
        kind: PrefixKind::Binary,
    },
];

// This is the typed translation of inputs/convert_registry.json.  The first token in every
// entry is canonical; the remaining tokens are exact, case-sensitive aliases.
const ENTRIES: &[UnitEntry] = &[
    UnitEntry::new(
        UnitDimension::Mass,
        "gram",
        &["g"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "slug",
        &["sg"],
        14593.9029372064,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "pound_mass",
        &["lbm"],
        453.59237,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "atomic_mass_unit",
        &["u"],
        1.660538782e-24,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "ounce_mass",
        &["ozm"],
        28.349523125,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "grain",
        &["grain"],
        0.06479891,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "short_hundredweight",
        &["cwt", "shweight"],
        45359.237,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "imperial_hundredweight",
        &["uk_cwt", "lcwt", "hweight"],
        50802.34544,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "stone",
        &["stone"],
        6350.29318,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "short_ton",
        &["ton"],
        907184.74,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Mass,
        "imperial_ton",
        &["uk_ton", "LTON", "brton"],
        1016046.9088,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "meter",
        &["m"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "international_mile",
        &["mi"],
        1609.344,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "nautical_mile",
        &["Nmi"],
        1852.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "inch",
        &["in"],
        0.0254,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "foot",
        &["ft"],
        0.3048,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "yard",
        &["yd"],
        0.9144,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "angstrom",
        &["ang"],
        1e-10,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "ell",
        &["ell"],
        1.143,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "light_year",
        &["ly"],
        9460730472580800.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "parsec",
        &["parsec", "pc"],
        30856775812815500.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "pica_point",
        &["Picapt", "Pica"],
        0.000_352_777_777_777_777_76,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "pica",
        &["pica"],
        0.004_233_333_333_333_334,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Distance,
        "survey_mile",
        &["survey_mi"],
        1_609.347_218_694_437_3,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Time,
        "year",
        &["yr"],
        31557600.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Time,
        "day",
        &["day", "d"],
        86400.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Time,
        "hour",
        &["hr"],
        3600.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Time,
        "minute",
        &["mn", "min"],
        60.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Time,
        "second",
        &["sec", "s"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Pressure,
        "pascal",
        &["Pa", "p"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Pressure,
        "atmosphere",
        &["atm", "at"],
        101325.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Pressure,
        "millimeter_mercury",
        &["mmHg"],
        133.322,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Pressure,
        "psi",
        &["psi"],
        6894.75729316836,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Pressure,
        "torr",
        &["Torr"],
        133.322_368_421_052_63,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Force,
        "newton",
        &["N"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Force,
        "dyne",
        &["dyn", "dy"],
        1e-5,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Force,
        "pound_force",
        &["lbf"],
        4.4482216152605,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "joule",
        &["J"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "erg",
        &["e"],
        1e-7,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "thermodynamic_calorie",
        &["c"],
        4.184,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "it_calorie",
        &["cal"],
        4.1868,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "electron_volt",
        &["eV", "ev"],
        1.602176487e-19,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "horsepower_hour",
        &["HPh", "hh"],
        2684519.53769617,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "watt_hour",
        &["Wh", "wh"],
        3600.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "foot_pound",
        &["flb"],
        1.3558179483314,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Energy,
        "it_btu",
        &["BTU", "btu"],
        1055.05585262,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Power,
        "mechanical_horsepower",
        &["HP", "h"],
        745.69987158227,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Power,
        "metric_horsepower",
        &["PS"],
        735.49875,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Power,
        "watt",
        &["W", "w"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Magnetism,
        "tesla",
        &["T"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Magnetism,
        "gauss",
        &["ga"],
        1e-4,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Temperature,
        "celsius",
        &["C", "cel"],
        1.0,
        273.15,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Temperature,
        "fahrenheit",
        &["F", "fah"],
        0.555_555_555_555_555_6,
        255.372_222_222_222_23,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Temperature,
        "kelvin",
        &["K", "kel"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Temperature,
        "rankine",
        &["Rank"],
        0.555_555_555_555_555_6,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Temperature,
        "reaumur",
        &["Reau"],
        1.25,
        273.15,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "teaspoon",
        &["tsp"],
        0.00000492892159375,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "modern_teaspoon",
        &["tspm"],
        0.000005,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "tablespoon",
        &["tbs"],
        0.00001478676478125,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "fluid_ounce",
        &["oz"],
        0.0000295735295625,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cup",
        &["cup"],
        0.0002365882365,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "us_pint",
        &["pt", "us_pt"],
        0.000473176473,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "uk_pint",
        &["uk_pt"],
        0.00056826125,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "us_quart",
        &["qt"],
        0.000946352946,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "uk_quart",
        &["uk_qt"],
        0.0011365225,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "us_gallon",
        &["gal"],
        0.003785411784,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "uk_gallon",
        &["uk_gal"],
        0.00454609,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "liter",
        &["l", "L", "lt"],
        0.001,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_angstrom",
        &["ang3", "ang^3"],
        1e-30,
        0.0,
        PrefixPolicy::Decimal,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "oil_barrel",
        &["barrel"],
        0.158987294928,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "us_bushel",
        &["bushel"],
        0.03523907016688,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_foot",
        &["ft3", "ft^3"],
        0.028316846592,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_inch",
        &["in3", "in^3"],
        0.000016387064,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_light_year",
        &["ly3", "ly^3"],
        846786664623715165955512486945616474714112000000.0,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_meter",
        &["m3", "m^3"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_mile",
        &["mi3", "mi^3"],
        4_168_181_825.440_579_4,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_yard",
        &["yd3", "yd^3"],
        0.764554857984,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_nautical_mile",
        &["Nmi3", "Nmi^3"],
        6352182208.0,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "cubic_pica_point",
        &["Picapt3", "Picapt^3", "Pica3", "Pica^3"],
        4.390_395_661_865_569e-11,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "gross_registered_ton",
        &["GRT", "regton"],
        2.8316846592,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Volume,
        "measurement_ton",
        &["MTON"],
        1.13267386368,
        0.0,
        PrefixPolicy::None,
        3,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "international_acre",
        &["uk_acre"],
        4046.8564224,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "us_survey_acre",
        &["us_acre"],
        4_046.872_609_874_252,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_angstrom",
        &["ang2", "ang^2"],
        1e-20,
        0.0,
        PrefixPolicy::Decimal,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "are",
        &["ar"],
        100.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_foot",
        &["ft2", "ft^2"],
        0.09290304,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "hectare",
        &["ha"],
        10000.0,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_inch",
        &["in2", "in^2"],
        0.00064516,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_light_year",
        &["ly2", "ly^2"],
        89505421074818927300612528640000.0,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_meter",
        &["m2", "m^2"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "morgen",
        &["Morgen"],
        2500.0,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_mile",
        &["mi2", "mi^2"],
        2589988.110336,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_nautical_mile",
        &["Nmi2", "Nmi^2"],
        3429904.0,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_pica_point",
        &["Picapt2", "Picapt^2", "Pica2", "Pica^2"],
        1.244_521_604_938_271_5e-7,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Area,
        "square_yard",
        &["yd2", "yd^2"],
        0.83612736,
        0.0,
        PrefixPolicy::None,
        2,
    ),
    UnitEntry::new(
        UnitDimension::Information,
        "bit",
        &["bit"],
        1.0,
        0.0,
        PrefixPolicy::DecimalBinary,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Information,
        "byte",
        &["byte"],
        8.0,
        0.0,
        PrefixPolicy::DecimalBinary,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Speed,
        "admiralty_knot",
        &["admkn"],
        0.514_773_333_333_333_3,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Speed,
        "knot",
        &["kn"],
        0.514_444_444_444_444_5,
        0.0,
        PrefixPolicy::None,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Speed,
        "meter_per_hour",
        &["m/h", "m/hr"],
        0.000_277_777_777_777_777_8,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Speed,
        "meter_per_second",
        &["m/s", "m/sec"],
        1.0,
        0.0,
        PrefixPolicy::Decimal,
        1,
    ),
    UnitEntry::new(
        UnitDimension::Speed,
        "mile_per_hour",
        &["mph"],
        0.44704,
        0.0,
        PrefixPolicy::None,
        1,
    ),
];

#[cfg(test)]
pub(super) const fn entries() -> &'static [UnitEntry] {
    ENTRIES
}

fn exact_entry(token: &str) -> Option<&'static UnitEntry> {
    ENTRIES.iter().find(|entry| entry.tokens.contains(&token))
}

fn prefix_allowed(entry: &UnitEntry, kind: PrefixKind) -> bool {
    matches!(
        (entry.prefix_policy, kind),
        (PrefixPolicy::Decimal, PrefixKind::Decimal)
            | (
                PrefixPolicy::DecimalBinary,
                PrefixKind::Decimal | PrefixKind::Binary
            )
    )
}

fn prefixes() -> impl Iterator<Item = &'static Prefix> {
    // Binary and the two-character decimal prefix are visited before one-character decimal
    // prefixes.  The parser still checks every candidate's exact suffix, so this ordering is a
    // correctness rule rather than an optimization.
    BINARY_PREFIXES
        .iter()
        .chain(DECIMAL_PREFIXES.iter().take(1))
        .chain(DECIMAL_PREFIXES.iter().skip(1))
}

pub(super) fn parse_unit(token: &str) -> Result<ParsedUnit, UnitParseError> {
    // Exact canonical/alias lookup must precede prefix splitting.  In particular `p`, `Pa`, `pc`,
    // `Pica`, `e` and `T` are units in their own right despite also being prefix spellings.
    if let Some(entry) = exact_entry(token) {
        return Ok(ParsedUnit {
            entry,
            prefix_multiplier: 1.0,
        });
    }

    let mut forbidden_prefix = false;
    for prefix in prefixes() {
        let Some(suffix) = token.strip_prefix(prefix.spelling) else {
            continue;
        };
        if suffix.is_empty() {
            continue;
        }
        let Some(entry) = exact_entry(suffix) else {
            continue;
        };
        if !prefix_allowed(entry, prefix.kind) {
            forbidden_prefix = true;
            continue;
        }
        let multiplier = prefix.multiplier.powi(i32::from(entry.prefix_power));
        if !multiplier.is_finite() {
            return Err(UnitParseError::ForbiddenPrefix);
        }
        return Ok(ParsedUnit {
            entry,
            prefix_multiplier: multiplier,
        });
    }
    if forbidden_prefix {
        Err(UnitParseError::ForbiddenPrefix)
    } else {
        Err(UnitParseError::UnknownToken)
    }
}

pub(super) fn convert_units(value: f64, from: &str, to: &str) -> Result<f64, ErrorKind> {
    let source = parse_unit(from).map_err(|_| ErrorKind::NA)?;
    let target = parse_unit(to).map_err(|_| ErrorKind::NA)?;
    if source.entry.dimension != target.entry.dimension {
        return Err(ErrorKind::NA);
    }
    let source_scale = source.entry.scale_to_base * source.prefix_multiplier;
    let target_scale = target.entry.scale_to_base * target.prefix_multiplier;
    let result = if source.entry.offset_to_base == 0.0 && target.entry.offset_to_base == 0.0 {
        // A source and target without affine offsets can be reduced before multiplying the input.
        // This keeps identity and near-identity conversions finite when the intermediate base
        // value exceeds f64 even though the requested result is representable.
        value * (source_scale / target_scale)
    } else {
        let source_base = value * source_scale + source.entry.offset_to_base;
        (source_base - target.entry.offset_to_base) / target_scale
    };
    if result.is_finite() {
        Ok(result)
    } else {
        Err(ErrorKind::Num)
    }
}

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [value, from, to] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let value = match required_number(engine, context, value) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let from = match required_text(engine, context, from) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let to = match required_text(engine, context, to) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    match convert_units(value, &from, &to) {
        Ok(value) => Value::Number(value),
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryValidationError {
    EmptyEntry,
    InvalidScale,
    InvalidPrefixPower,
    AffinePrefix,
    DuplicateToken,
    PrefixAmbiguity,
    InvalidPrefixTable,
}

#[cfg(test)]
pub(super) fn validate_registry() -> Result<(), RegistryValidationError> {
    validate_entries(ENTRIES)?;
    let mut seen_prefixes = Vec::new();
    let mut previous_length = usize::MAX;
    for prefix in prefixes() {
        if prefix.spelling.is_empty()
            || !prefix.multiplier.is_finite()
            || prefix.multiplier <= 0.0
            || prefix.spelling.len() > previous_length
        {
            return Err(RegistryValidationError::InvalidPrefixTable);
        }
        if seen_prefixes.contains(&prefix.spelling) {
            return Err(RegistryValidationError::PrefixAmbiguity);
        }
        seen_prefixes.push(prefix.spelling);
        previous_length = prefix.spelling.len();
    }
    Ok(())
}

#[cfg(test)]
fn validate_entries(entries: &[UnitEntry]) -> Result<(), RegistryValidationError> {
    let mut index = Vec::new();
    for entry in entries {
        if entry.id.is_empty() || entry.tokens.is_empty() || entry.canonical_token().is_empty() {
            return Err(RegistryValidationError::EmptyEntry);
        }
        if !entry.scale_to_base.is_finite()
            || entry.scale_to_base <= 0.0
            || !entry.offset_to_base.is_finite()
        {
            return Err(RegistryValidationError::InvalidScale);
        }
        if !(1..=3).contains(&entry.prefix_power) {
            return Err(RegistryValidationError::InvalidPrefixPower);
        }
        if entry.offset_to_base != 0.0 && entry.prefix_policy != PrefixPolicy::None {
            return Err(RegistryValidationError::AffinePrefix);
        }
        for token in entry.tokens {
            if index.contains(token) {
                return Err(RegistryValidationError::DuplicateToken);
            }
            index.push(*token);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        PrefixKind, RegistryValidationError, UnitDimension, UnitParseError, convert_units, entries,
        parse_unit, validate_registry,
    };
    use crate::calculation::value::ErrorKind;

    fn approx(actual: f64, expected: f64) {
        assert!(actual.is_finite());
        assert!((actual - expected).abs() <= 1e-12 * expected.abs().max(1.0));
    }

    #[test]
    fn static_registry_has_the_frozen_inventory() {
        validate_registry().expect("typed CONVERT registry validates");
        assert_eq!(entries().len(), 102);
        assert_eq!(
            entries()
                .iter()
                .map(|entry| entry.tokens.len())
                .sum::<usize>(),
            152
        );
    }

    #[test]
    fn exact_tokens_win_over_prefixes_and_aliases_remain_case_sensitive() {
        assert_eq!(parse_unit("pc").expect("parsec alias").entry.id(), "parsec");
        assert_eq!(parse_unit("Pa").expect("pascal").entry.id(), "pascal");
        assert_eq!(parse_unit("p").expect("pascal alias").entry.id(), "pascal");
        assert_eq!(parse_unit("e").expect("erg").entry.id(), "erg");
        assert_eq!(parse_unit("T").expect("tesla").entry.id(), "tesla");
        assert_eq!(
            parse_unit("Pica").expect("pica point").entry.id(),
            "pica_point"
        );
        assert_eq!(parse_unit("KM"), Err(UnitParseError::UnknownToken));
    }

    #[test]
    fn prefix_policies_and_dimension_powers_are_typed() {
        let kilometer = parse_unit("km").expect("kilometer");
        assert_eq!(kilometer.entry.dimension(), UnitDimension::Distance);
        approx(kilometer.prefix_multiplier, 1e3);
        approx(
            parse_unit("km2")
                .expect("square kilometer")
                .prefix_multiplier,
            1e6,
        );
        approx(
            parse_unit("cm3")
                .expect("cubic centimeter")
                .prefix_multiplier,
            1e-6,
        );
        approx(
            parse_unit("kibyte").expect("kibibyte").prefix_multiplier,
            1024.0,
        );
        assert_eq!(parse_unit("kC"), Err(UnitParseError::ForbiddenPrefix));
        assert_eq!(parse_unit("kim"), Err(UnitParseError::ForbiddenPrefix));
        assert_eq!(parse_unit("Kibyte"), Err(UnitParseError::UnknownToken));
        // Keep the prefix kind in the model rather than accepting a decimal spelling as binary.
        assert_eq!(super::BINARY_PREFIXES[7].kind, PrefixKind::Binary);
    }

    #[test]
    fn conversion_matches_independent_reference_points() {
        approx(
            convert_units(10.0, "km", "mi").expect("km to mi"),
            6.2137119223733395,
        );
        approx(convert_units(68.0, "F", "C").expect("F to C"), 20.0);
        approx(
            convert_units(1.0, "tsp", "tbs").expect("tsp to tbs"),
            1.0 / 3.0,
        );
        approx(convert_units(1.0, "km2", "m2").expect("area prefix"), 1e6);
        approx(
            convert_units(1.0, "cm3", "m3").expect("volume prefix"),
            1e-6,
        );
        approx(
            convert_units(1.0, "kibyte", "bit").expect("binary prefix"),
            8192.0,
        );
        approx(convert_units(80.0, "Reau", "C").expect("reaumur"), 100.0);
        assert_eq!(convert_units(1.0, "ft", "sec"), Err(ErrorKind::NA));
        assert_eq!(convert_units(1.0, "kC", "C"), Err(ErrorKind::NA));
        assert_eq!(convert_units(1e308, "Ym", "m"), Err(ErrorKind::Num));
        assert_eq!(
            convert_units(1e308, "Ym", "Ym"),
            Ok(1e308),
            "a representable identity result must not inherit intermediate base overflow"
        );
    }

    #[test]
    fn frozen_reference_matrix_covers_every_generated_convert_case() {
        for (value, from, to, expected) in [
            (10.0, "km", "mi", 6.2137119223733395),      // oracle_km_mi
            (1.0, "lbm", "kg", 0.45359237),              // docs_lbm_kg
            (68.0, "F", "C", 20.0),                      // docs_f_c
            (6.0, "tsp", "tbs", 2.0),                    // docs_tsp_tbs
            (6.0, "gal", "l", 22.712470704),             // docs_gal_l
            (6.0, "mi", "km", 9.656064),                 // docs_mi_km
            (6.0, "in", "ft", 0.5),                      // docs_in_ft
            (6.0, "cm", "in", 2.3622047244094486),       // docs_cm_in
            (7.25, "pc", "pc", 7.25),                    // identity_pc
            (1.0, "pc", "parsec", 1.0),                  // parsec_alias
            (1.0, "Pa", "p", 1.0),                       // pascal_exact_precedence
            (1.0, "Pica", "pica", 0.08333333333333333),  // pica_case_collision
            (1.0, "ee", "J", 1e-6),                      // deka_erg_collision
            (1.0, "km", "m", 1e3),                       // kilometer
            (1.0, "ms", "sec", 1e-3),                    // millisecond
            (1.0, "kPa", "Pa", 1e3),                     // kilopascal
            (1.0, "ug", "g", 1e-6),                      // microgram
            (1.0, "km2", "m2", 1e6),                     // square_kilometer
            (1.0, "km^2", "m^2", 1e6),                   // square_kilometer_caret
            (1.0, "cm3", "m3", 1e-6),                    // cubic_centimeter
            (1.0, "mm^3", "m^3", 1e-9),                  // cubic_millimeter_caret
            (1.0, "kl", "m3", 1.0),                      // kiloliter_scalar_prefix
            (1.0, "kibyte", "bit", 8192.0),              // kibibyte_bits
            (1.0, "Mibit", "byte", 131072.0),            // mebibit_bytes
            (1.0, "kbyte", "byte", 1e3),                 // decimal_kilobyte
            (0.0, "C", "K", 273.15),                     // freezing_c_k
            (32.0, "F", "K", 273.15),                    // freezing_f_k
            (0.0, "K", "F", -459.67),                    // absolute_zero_k_f
            (80.0, "Reau", "C", 100.0),                  // reaumur_c
            (491.67, "Rank", "K", 273.15),               // rankine_k
            (1.0, "survey_mi", "m", 1609.3472186944373), // survey_mile_m
            (1.0, "us_acre", "uk_acre", 1.000004000012), // acre_alias_independent
            (1.0, "GRT", "ft3", 100.0),                  // gross_ton_ft3
            (1.0, "MTON", "ft3", 40.0),                  // measurement_ton_ft3
            (1.0, "admkn", "kn", 1.0006393088552916),    // admiralty_vs_knot
        ] {
            approx(
                convert_units(value, from, to)
                    .unwrap_or_else(|error| panic!("{from} to {to} failed: {error:?}")),
                expected,
            );
        }

        for (value, from, to, expected) in [
            (1.0, "bogus", "m", ErrorKind::NA),     // unknown_unit
            (1.0, "KM", "m", ErrorKind::NA),        // case_mismatch
            (1.0, "ft", "sec", ErrorKind::NA),      // dimension_mismatch
            (1.0, "kC", "C", ErrorKind::NA),        // forbidden_prefix_temperature
            (1.0, "kim", "m", ErrorKind::NA),       // forbidden_binary_meter
            (1.0, "Kibyte", "byte", ErrorKind::NA), // uppercase_binary_prefix
            (1e308, "Ym", "m", ErrorKind::Num),     // overflow_prefix
        ] {
            assert_eq!(
                convert_units(value, from, to),
                Err(expected),
                "{from} to {to}"
            );
        }
    }

    #[test]
    fn worksheet_adapter_coercion_and_excel_errors_are_end_to_end() {
        use crate::{
            CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellValue,
            ExcelError, FormulaText, WorkbookDraft, calculate_workbook,
        };

        let mut draft = WorkbookDraft::new();
        let sheet = draft.workbook().sheets()[0].id();
        for (address, formula) in [
            ("A1", "=CONVERT(\"2\",\"km\",\"mi\")"),
            ("A2", "=CONVERT(\"label\",\"km\",\"mi\")"),
            ("A3", "=CONVERT(1,\"ft\",\"sec\")"),
            ("A4", "=CONVERT(1E308,\"Ym\",\"m\")"),
        ] {
            draft
                .set_cell_formula(
                    sheet,
                    CellAddress::from_a1(address).expect("valid test address"),
                    FormulaText::from_user_input(formula).expect("valid test formula"),
                )
                .expect("formula edit");
        }
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let result = |address: &str| {
            calculation.cell(CalculationCellId::new(
                sheet,
                CellAddress::from_a1(address).expect("valid test address"),
            ))
        };
        let Some(CalculationCellResult::Value(CellValue::Number(value))) = result("A1") else {
            panic!(
                "text numeric coercion did not produce a number: {:?}",
                result("A1")
            );
        };
        approx(value.get(), 1.242_742_384_474_668);
        assert_eq!(
            result("A2"),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            )))
        );
        assert_eq!(
            result("A3"),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::NotAvailable
            )))
        );
        assert_eq!(
            result("A4"),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            )))
        );
    }

    #[test]
    fn validation_rejects_the_registry_invariants_without_runtime_json() {
        assert_eq!(validate_registry(), Ok(()));
        let first = entries()[0];
        let duplicate = super::UnitEntry::new(
            UnitDimension::Mass,
            "duplicate",
            &["g"],
            1.0,
            0.0,
            super::PrefixPolicy::None,
            1,
        );
        assert_eq!(
            super::validate_entries(&[first, duplicate]),
            Err(RegistryValidationError::DuplicateToken)
        );
        let affine = super::UnitEntry::new(
            UnitDimension::Temperature,
            "invalid_affine",
            &["bad_affine"],
            1.0,
            273.15,
            super::PrefixPolicy::Decimal,
            1,
        );
        assert_eq!(
            super::validate_entries(&[affine]),
            Err(RegistryValidationError::AffinePrefix)
        );
    }
}
