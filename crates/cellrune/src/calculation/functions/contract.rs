use super::kernel::{
    AggregateFunction, ArrayFunction, CombinatoricsFunction, DatabaseFunction,
    DateAdditionalFunction, DateFunction, DistributionFunction, DynamicFunction,
    EngineeringFunction, Evaluator, FinancialAdditionalFunction, FinancialFunction,
    GroupedFunction, InformationFunction, LegacyFunction, LogicalFunction, LookupFunction,
    MathFunction, ModernTextFunction, RegressionFunction, RomanFunction,
    StatisticalAdditionalFunction, StatisticalFunction, SumOfSquaresFunction,
    TextAdditionalFunction, TextFunction, TrigonometryFunction,
};

#[cfg(test)]
mod snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Arity {
    minimum: u16,
    maximum: Option<u16>,
    step: u16,
}

impl Arity {
    const fn exact(count: u16) -> Self {
        Self {
            minimum: count,
            maximum: Some(count),
            step: 1,
        }
    }

    const fn range(minimum: u16, maximum: u16) -> Self {
        Self {
            minimum,
            maximum: Some(maximum),
            step: 1,
        }
    }

    const fn stepped(minimum: u16, maximum: Option<u16>, step: u16) -> Self {
        Self {
            minimum,
            maximum,
            step,
        }
    }

    pub(super) fn accepts(self, count: usize) -> bool {
        let Ok(count) = u16::try_from(count) else {
            return false;
        };
        count >= self.minimum
            && match self.maximum {
                Some(maximum) => count <= maximum,
                None => true,
            }
            && (count - self.minimum).is_multiple_of(self.step)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArgumentMode {
    Scalar,
    Array,
    Reference,
    Callable,
    Deferred,
    BindingName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArgumentLayout {
    Uniform(ArgumentMode),
    Positional(&'static [ArgumentMode]),
    Repeating {
        leading: &'static [ArgumentMode],
        repeated: &'static [ArgumentMode],
        trailing: &'static [ArgumentMode],
    },
    LetBindings,
    LambdaDefinition,
    ArraysThenCallable,
}

impl ArgumentLayout {
    pub(super) fn mode_at(self, index: usize, argument_count: usize) -> Option<ArgumentMode> {
        match self {
            Self::Uniform(mode) => Some(mode),
            Self::Positional(modes) => modes.get(index).copied(),
            Self::Repeating {
                leading,
                repeated,
                trailing,
            } => {
                if let Some(mode) = leading.get(index) {
                    return Some(*mode);
                }
                let trailing_start = argument_count.saturating_sub(trailing.len());
                if index >= trailing_start && argument_count >= leading.len() + trailing.len() {
                    return trailing.get(index - trailing_start).copied();
                }
                (!repeated.is_empty()).then(|| repeated[(index - leading.len()) % repeated.len()])
            }
            Self::LetBindings => {
                if index + 1 == argument_count {
                    Some(ArgumentMode::Deferred)
                } else if index.is_multiple_of(2) {
                    Some(ArgumentMode::BindingName)
                } else {
                    Some(ArgumentMode::Deferred)
                }
            }
            Self::LambdaDefinition => {
                if index + 1 == argument_count {
                    Some(ArgumentMode::Deferred)
                } else {
                    Some(ArgumentMode::BindingName)
                }
            }
            Self::ArraysThenCallable => {
                if index + 1 == argument_count {
                    Some(ArgumentMode::Callable)
                } else {
                    Some(ArgumentMode::Array)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DefaultTrigger {
    Absent,
    Missing,
    AbsentOrMissing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ArgumentDefaultValue {
    Omitted,
    Number(f64),
    Logical(bool),
    NotAvailable,
    CalculationError,
    CallerReference,
    CriteriaRange,
    EmptyCollection,
    IndexColumn,
    LookupVector,
    NoPadding,
    NoSheetQualifier,
    NoUpperBound,
    AllOccurrences,
    SourceRows,
    SourceColumns,
    LinkLocation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ArgumentDefault {
    position: u16,
    trigger: DefaultTrigger,
    value: ArgumentDefaultValue,
}

impl ArgumentDefault {
    const fn new(position: u16, trigger: DefaultTrigger, value: ArgumentDefaultValue) -> Self {
        Self {
            position,
            trigger,
            value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RepeatingArgumentDefault {
    first_position: u16,
    step: u16,
    trigger: DefaultTrigger,
    value: ArgumentDefaultValue,
}

impl RepeatingArgumentDefault {
    const fn new(
        first_position: u16,
        step: u16,
        trigger: DefaultTrigger,
        value: ArgumentDefaultValue,
    ) -> Self {
        Self {
            first_position,
            step,
            trigger,
            value,
        }
    }

    fn applies_at(self, position: usize, trigger: DefaultTrigger) -> bool {
        let position = u16::try_from(position).ok();
        position.is_some_and(|position| {
            position >= self.first_position
                && (position - self.first_position).is_multiple_of(self.step)
                && (self.trigger == trigger || self.trigger == DefaultTrigger::AbsentOrMissing)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MissingArgumentPolicy {
    CoerceToBlank,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MissingArgumentBehavior {
    CoerceToBlank,
    Preserve,
    Default(ArgumentDefaultValue),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CallContract {
    arity: Arity,
    layout: ArgumentLayout,
    missing: MissingArgumentPolicy,
    defaults: &'static [ArgumentDefault],
    repeating_defaults: &'static [RepeatingArgumentDefault],
}

impl CallContract {
    const fn uniform(arity: Arity, mode: ArgumentMode) -> Self {
        Self {
            arity,
            layout: ArgumentLayout::Uniform(mode),
            missing: MissingArgumentPolicy::CoerceToBlank,
            defaults: &[],
            repeating_defaults: &[],
        }
    }

    const fn positional(arity: Arity, modes: &'static [ArgumentMode]) -> Self {
        Self {
            arity,
            layout: ArgumentLayout::Positional(modes),
            missing: MissingArgumentPolicy::CoerceToBlank,
            defaults: &[],
            repeating_defaults: &[],
        }
    }

    const fn repeating(
        arity: Arity,
        leading: &'static [ArgumentMode],
        repeated: &'static [ArgumentMode],
        trailing: &'static [ArgumentMode],
    ) -> Self {
        Self {
            arity,
            layout: ArgumentLayout::Repeating {
                leading,
                repeated,
                trailing,
            },
            missing: MissingArgumentPolicy::CoerceToBlank,
            defaults: &[],
            repeating_defaults: &[],
        }
    }

    const fn special(arity: Arity, layout: ArgumentLayout) -> Self {
        Self {
            arity,
            layout,
            missing: MissingArgumentPolicy::CoerceToBlank,
            defaults: &[],
            repeating_defaults: &[],
        }
    }

    const fn with_missing(mut self, missing: MissingArgumentPolicy) -> Self {
        self.missing = missing;
        self
    }

    const fn with_defaults(mut self, defaults: &'static [ArgumentDefault]) -> Self {
        self.defaults = defaults;
        self
    }

    const fn with_repeating_defaults(
        mut self,
        defaults: &'static [RepeatingArgumentDefault],
    ) -> Self {
        self.repeating_defaults = defaults;
        self
    }

    pub(super) const fn arity(self) -> Arity {
        self.arity
    }

    pub(super) const fn maximum_arity(self) -> Option<u16> {
        self.arity.maximum
    }

    pub(super) const fn layout(self) -> ArgumentLayout {
        self.layout
    }

    pub(super) fn default_at(
        self,
        position: usize,
        trigger: DefaultTrigger,
    ) -> Option<ArgumentDefaultValue> {
        self.defaults
            .iter()
            .find(|default| {
                usize::from(default.position) == position
                    && (default.trigger == trigger
                        || default.trigger == DefaultTrigger::AbsentOrMissing)
            })
            .map(|default| default.value)
            .or_else(|| {
                self.repeating_defaults
                    .iter()
                    .copied()
                    .find(|default| default.applies_at(position, trigger))
                    .map(|default| default.value)
            })
    }

    pub(super) fn missing_behavior_at(self, position: usize) -> MissingArgumentBehavior {
        self.default_at(position, DefaultTrigger::Missing).map_or(
            match self.missing {
                MissingArgumentPolicy::CoerceToBlank => MissingArgumentBehavior::CoerceToBlank,
                MissingArgumentPolicy::Preserve => MissingArgumentBehavior::Preserve,
            },
            MissingArgumentBehavior::Default,
        )
    }
}

const SCALAR: ArgumentMode = ArgumentMode::Scalar;
const ARRAY: ArgumentMode = ArgumentMode::Array;
const REFERENCE: ArgumentMode = ArgumentMode::Reference;
const CALLABLE: ArgumentMode = ArgumentMode::Callable;
const DEFERRED: ArgumentMode = ArgumentMode::Deferred;
const MAX_EXCEL_ARGUMENTS: u16 = 255;
const MAX_SWITCH_ARGUMENTS: u16 = 254;
const MAX_CONCAT_ARGUMENTS: u16 = 253;
const MAX_TEXT_JOIN_ARGUMENTS: u16 = 254;
const MAX_EXTREME_IFS_ARGUMENTS: u16 = 253;

const IF_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::Missing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::Missing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::Absent,
        ArgumentDefaultValue::Logical(false),
    ),
];
const INDEX_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::Missing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::IndexColumn,
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
];
const MATCH_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(1.0),
)];
const CRITERIA_RESULT_RANGE_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::CriteriaRange,
)];
const ROUND_DIGITS_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const LOG_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(10.0),
)];
const MODERN_MULTIPLE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(1, DefaultTrigger::Absent, ArgumentDefaultValue::Number(1.0)),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
];
const PRECISE_MULTIPLE_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(1.0),
)];
const BASE_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const RADIX_PLACES_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::NoPadding,
)];
const COMPARISON_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const ERF_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::NoUpperBound,
)];
const TABLE_LOOKUP_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    3,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Logical(true),
)];
const ADDRESS_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(true),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::NoSheetQualifier,
    ),
];
const HYPERLINK_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::LinkLocation,
)];
const INDIRECT_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Logical(true),
)];
const LOOKUP_VECTOR_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::LookupVector,
)];
const OFFSET_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::SourceRows,
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::SourceColumns,
    ),
];
const XLOOKUP_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::NotAvailable,
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        5,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
];
const SHEET_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    0,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::CallerReference,
)];
const SHEETS_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    0,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::EmptyCollection,
)];
const XMATCH_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
];
const DOLLAR_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(2.0),
)];
const LEFT_RIGHT_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(1.0),
)];
const FIND_SEARCH_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(1.0),
)];
const SUBSTITUTE_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    3,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::AllOccurrences,
)];
const TEXT_BOUNDARY_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(2, DefaultTrigger::Absent, ArgumentDefaultValue::Number(1.0)),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
    ArgumentDefault::new(
        5,
        DefaultTrigger::Absent,
        ArgumentDefaultValue::NotAvailable,
    ),
];
const VALUE_TO_TEXT_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Logical(false),
)];
const ARRAY_TO_TEXT_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const REGEX_EXTRACT_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
];
const REGEX_REPLACE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
];
const REGEX_TEST_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const TEXT_SPLIT_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::EmptyCollection,
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        5,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::NotAvailable,
    ),
];
const ROW_COLUMN_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    0,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::CallerReference,
)];
const ZERO_OPTION_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const ONE_OPTION_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(1.0),
)];
const EMPTY_COLLECTION_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::EmptyCollection,
)];
const TAKE_DROP_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(1, DefaultTrigger::Missing, ArgumentDefaultValue::SourceRows),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::SourceColumns,
    ),
];
const FILTER_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::CalculationError,
)];
const EXPAND_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(1, DefaultTrigger::Missing, ArgumentDefaultValue::SourceRows),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::SourceColumns,
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::NoPadding,
    ),
];
const SORTBY_DEFAULTS: &[RepeatingArgumentDefault] = &[RepeatingArgumentDefault::new(
    2,
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(1.0),
)];
const FLATTEN_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
];
const TRIM_RANGE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(3.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(3.0),
    ),
];
const WRAP_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::NoPadding,
)];
const SEQUENCE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        0,
        DefaultTrigger::Missing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        1,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
];
const REGRESSION_STATISTICS_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(1, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(
        2,
        DefaultTrigger::Absent,
        ArgumentDefaultValue::Logical(true),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::Absent,
        ArgumentDefaultValue::Logical(false),
    ),
];
const REGRESSION_PREDICTION_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(1, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(2, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(
        3,
        DefaultTrigger::Absent,
        ArgumentDefaultValue::Logical(true),
    ),
];
const ROMAN_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(0.0),
)];
const SORT_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(1.0),
    ),
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
];
const UNIQUE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        1,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
    ArgumentDefault::new(
        2,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Logical(false),
    ),
];
const PERCENT_RANK_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(3.0),
)];
const RANK_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::AbsentOrMissing,
    ArgumentDefaultValue::Number(0.0),
)];
const FINANCIAL_FIVE_ARGUMENT_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
];
const FINANCIAL_SIX_ARGUMENT_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        5,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
];
const SOLVER_GUESS_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    1,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(0.1),
)];
const XIRR_GUESS_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    2,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(0.1),
)];
const RATE_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(
        3,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(
        4,
        DefaultTrigger::AbsentOrMissing,
        ArgumentDefaultValue::Number(0.0),
    ),
    ArgumentDefault::new(5, DefaultTrigger::Absent, ArgumentDefaultValue::Number(0.1)),
];
const DB_DEFAULTS: &[ArgumentDefault] = &[ArgumentDefault::new(
    4,
    DefaultTrigger::Absent,
    ArgumentDefaultValue::Number(12.0),
)];
const GROUPBY_OPTION_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(3, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(4, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(5, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(6, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(7, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
];
const PIVOTBY_OPTION_DEFAULTS: &[ArgumentDefault] = &[
    ArgumentDefault::new(4, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(5, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(6, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(7, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(8, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(9, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
    ArgumentDefault::new(10, DefaultTrigger::Absent, ArgumentDefaultValue::Omitted),
];

impl Evaluator {
    pub(super) const fn call_contract(self) -> CallContract {
        match self {
            Self::Legacy(function) => function.call_contract(),
            Self::Logical(function) => function.call_contract(),
            Self::Aggregate(function) => function.call_contract(),
            Self::Grouped(function) => function.call_contract(),
            Self::Database(function) => function.call_contract(),
            Self::Math(function) => function.call_contract(),
            Self::Roman(function) => function.call_contract(),
            Self::Trigonometry(function) => function.call_contract(),
            Self::Combinatorics(function) => function.call_contract(),
            Self::SumOfSquares(function) => function.call_contract(),
            Self::Engineering(function) => function.call_contract(),
            Self::Lookup(function) => function.call_contract(),
            Self::Information(function) => function.call_contract(),
            Self::Text(function) => function.call_contract(),
            Self::TextAdditional(function) => function.call_contract(),
            Self::ModernText(function) => function.call_contract(),
            Self::Date(function) => function.call_contract(),
            Self::DateAdditional(function) => function.call_contract(),
            Self::Dynamic(function) => function.call_contract(),
            Self::Array(function) => function.call_contract(),
            Self::Regression(function) => function.call_contract(),
            Self::Statistical(function) => function.call_contract(),
            Self::StatisticalAdditional(function) => function.call_contract(),
            Self::Distribution(function) => function.call_contract(),
            Self::Financial(function) => function.call_contract(),
            Self::FinancialAdditional(function) => function.call_contract(),
            Self::Areas => CallContract::uniform(Arity::exact(1), REFERENCE),
        }
    }
}

impl LegacyFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::If => CallContract::positional(Arity::range(2, 3), &[SCALAR, DEFERRED, DEFERRED])
                .with_defaults(IF_DEFAULTS),
            Self::And | Self::SumProduct => {
                CallContract::uniform(Arity::range(1, MAX_EXCEL_ARGUMENTS), ARRAY)
            }
            Self::IfError => CallContract::positional(Arity::exact(2), &[DEFERRED, DEFERRED]),
            Self::Lower => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Text => CallContract::positional(Arity::exact(2), &[SCALAR, SCALAR]),
            Self::CountIf => CallContract::positional(Arity::exact(2), &[REFERENCE, SCALAR]),
            Self::CountIfs => CallContract::repeating(
                Arity::stepped(2, Some(MAX_EXCEL_ARGUMENTS), 2),
                &[],
                &[REFERENCE, SCALAR],
                &[],
            ),
            Self::Index => {
                CallContract::positional(Arity::range(2, 4), &[REFERENCE, SCALAR, SCALAR, SCALAR])
                    .with_defaults(INDEX_DEFAULTS)
            }
            Self::Match => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, REFERENCE, SCALAR])
                    .with_defaults(MATCH_DEFAULTS)
            }
            Self::DummyFunction => CallContract::uniform(Arity::exact(1), SCALAR),
        }
    }
}

impl LogicalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::True | Self::False => CallContract::uniform(Arity::exact(0), SCALAR),
            Self::Not => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Or => CallContract::uniform(Arity::range(1, 255), ARRAY),
            Self::Xor => CallContract::uniform(Arity::range(1, 254), ARRAY),
            Self::IfNa => CallContract::positional(Arity::exact(2), &[DEFERRED, DEFERRED]),
            Self::Ifs => CallContract::repeating(
                Arity::stepped(2, Some(MAX_EXCEL_ARGUMENTS), 2),
                &[],
                &[SCALAR, DEFERRED],
                &[],
            ),
            Self::Switch => CallContract::repeating(
                Arity::range(3, MAX_SWITCH_ARGUMENTS),
                &[SCALAR],
                &[SCALAR, DEFERRED],
                &[DEFERRED],
            ),
        }
    }
}

impl AggregateFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Sum
            | Self::Average
            | Self::Min
            | Self::Max
            | Self::Count
            | Self::CountA
            | Self::Product => CallContract::uniform(Arity::range(1, MAX_EXCEL_ARGUMENTS), ARRAY),
            Self::CountBlank => CallContract::uniform(Arity::exact(1), REFERENCE),
            Self::Subtotal => CallContract::repeating(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                &[SCALAR],
                &[ARRAY],
                &[],
            ),
            Self::SumIf | Self::AverageIf => {
                CallContract::positional(Arity::range(2, 3), &[REFERENCE, SCALAR, REFERENCE])
                    .with_defaults(CRITERIA_RESULT_RANGE_DEFAULTS)
            }
            Self::SumIfs | Self::AverageIfs => CallContract::repeating(
                Arity::stepped(3, Some(MAX_EXCEL_ARGUMENTS), 2),
                &[REFERENCE],
                &[REFERENCE, SCALAR],
                &[],
            ),
        }
    }
}

impl GroupedFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::GroupBy => CallContract::positional(
                Arity::range(3, 8),
                &[ARRAY, ARRAY, CALLABLE, SCALAR, SCALAR, ARRAY, ARRAY, SCALAR],
            )
            .with_missing(MissingArgumentPolicy::Preserve)
            .with_defaults(GROUPBY_OPTION_DEFAULTS),
            Self::PercentOf => CallContract::positional(Arity::exact(2), &[ARRAY, ARRAY]),
            Self::PivotBy => CallContract::positional(
                Arity::range(4, 11),
                &[
                    ARRAY, ARRAY, ARRAY, CALLABLE, SCALAR, SCALAR, ARRAY, SCALAR, ARRAY, ARRAY,
                    SCALAR,
                ],
            )
            .with_missing(MissingArgumentPolicy::Preserve)
            .with_defaults(PIVOTBY_OPTION_DEFAULTS),
        }
    }
}

impl DatabaseFunction {
    const fn call_contract(self) -> CallContract {
        CallContract::positional(Arity::exact(3), &[REFERENCE, SCALAR, REFERENCE])
            .with_missing(MissingArgumentPolicy::Preserve)
    }
}

impl MathFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Abs
            | Self::Even
            | Self::Exp
            | Self::Int
            | Self::Ln
            | Self::Log10
            | Self::Odd
            | Self::Sign
            | Self::Sqrt
            | Self::SqrtPi => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Ceiling
            | Self::Decimal
            | Self::Floor
            | Self::Mod
            | Self::MRound
            | Self::Power
            | Self::Quotient
            | Self::Round
            | Self::RoundUp => CallContract::uniform(Arity::exact(2), SCALAR),
            Self::RoundDown | Self::Trunc => {
                CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                    .with_defaults(ROUND_DIGITS_DEFAULTS)
            }
            Self::Log => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(LOG_DEFAULTS),
            Self::Pi => CallContract::uniform(Arity::exact(0), SCALAR),
            Self::CeilingMath | Self::FloorMath => {
                CallContract::positional(Arity::range(1, 3), &[SCALAR, SCALAR, SCALAR])
                    .with_defaults(MODERN_MULTIPLE_DEFAULTS)
            }
            Self::CeilingPrecise | Self::IsoCeiling | Self::FloorPrecise => {
                CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                    .with_defaults(PRECISE_MULTIPLE_DEFAULTS)
            }
            Self::Base => CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, SCALAR])
                .with_defaults(BASE_DEFAULTS),
            Self::SeriesSum => {
                CallContract::positional(Arity::exact(4), &[SCALAR, SCALAR, SCALAR, ARRAY])
            }
        }
    }
}

impl RomanFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Arabic => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Roman => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(ROMAN_DEFAULTS),
        }
    }
}

impl TrigonometryFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Atan2 => CallContract::uniform(Arity::exact(2), SCALAR),
            _ => CallContract::uniform(Arity::exact(1), SCALAR),
        }
    }
}

impl CombinatoricsFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Fact | Self::FactDouble => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Gcd | Self::Lcm | Self::Multinomial => {
                CallContract::uniform(Arity::range(1, 255), ARRAY)
            }
            Self::Combin | Self::Combina | Self::Permut | Self::PermutationA => {
                CallContract::uniform(Arity::exact(2), SCALAR)
            }
        }
    }
}

impl SumOfSquaresFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::SumSq => CallContract::uniform(Arity::range(1, 255), ARRAY),
            Self::SumX2My2 | Self::SumX2Py2 | Self::SumXMy2 => {
                CallContract::uniform(Arity::exact(2), ARRAY)
            }
        }
    }
}

impl EngineeringFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::BitAnd | Self::BitLShift | Self::BitOr | Self::BitRShift | Self::BitXor => {
                CallContract::uniform(Arity::exact(2), SCALAR)
            }
            Self::Bin2Dec
            | Self::Hex2Dec
            | Self::Oct2Dec
            | Self::ErfPrecise
            | Self::Erfc
            | Self::ErfcPrecise => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Bin2Hex
            | Self::Bin2Oct
            | Self::Dec2Bin
            | Self::Dec2Hex
            | Self::Dec2Oct
            | Self::Hex2Bin
            | Self::Hex2Oct
            | Self::Oct2Bin
            | Self::Oct2Hex => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(RADIX_PLACES_DEFAULTS),
            Self::Delta | Self::GeStep => {
                CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                    .with_defaults(COMPARISON_DEFAULTS)
            }
            Self::Erf => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(ERF_DEFAULTS),
        }
    }
}

impl LookupFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Address => CallContract::positional(
                Arity::range(2, 5),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(ADDRESS_DEFAULTS),
            Self::Choose => CallContract::repeating(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                &[SCALAR],
                &[DEFERRED],
                &[],
            ),
            Self::Column | Self::Row => CallContract::positional(Arity::range(0, 1), &[REFERENCE])
                .with_defaults(ROW_COLUMN_DEFAULTS),
            Self::Columns | Self::Rows => CallContract::uniform(Arity::exact(1), REFERENCE),
            Self::HLookup | Self::VLookup => {
                CallContract::positional(Arity::range(3, 4), &[SCALAR, REFERENCE, SCALAR, SCALAR])
                    .with_defaults(TABLE_LOOKUP_DEFAULTS)
            }
            Self::Hyperlink => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(HYPERLINK_DEFAULTS),
            Self::Indirect => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(INDIRECT_DEFAULTS),
            Self::Lookup => CallContract::positional(Arity::range(2, 3), &[SCALAR, ARRAY, ARRAY])
                .with_defaults(LOOKUP_VECTOR_DEFAULTS),
            Self::Offset => CallContract::positional(
                Arity::range(3, 5),
                &[REFERENCE, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(OFFSET_DEFAULTS),
            Self::Sheet => CallContract::positional(Arity::range(0, 1), &[REFERENCE])
                .with_defaults(SHEET_DEFAULTS),
            Self::Sheets => CallContract::positional(Arity::range(0, 1), &[REFERENCE])
                .with_defaults(SHEETS_DEFAULTS),
            Self::XMatch => {
                CallContract::positional(Arity::range(2, 4), &[SCALAR, ARRAY, SCALAR, SCALAR])
                    .with_defaults(XMATCH_DEFAULTS)
            }
            Self::XLookup => CallContract::positional(
                Arity::range(3, 6),
                &[SCALAR, REFERENCE, REFERENCE, DEFERRED, SCALAR, SCALAR],
            )
            .with_defaults(XLOOKUP_DEFAULTS),
        }
    }
}

impl InformationFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Na => CallContract::uniform(Arity::exact(0), SCALAR),
            Self::FormulaText | Self::IsFormula | Self::IsRef => {
                CallContract::uniform(Arity::exact(1), REFERENCE)
            }
            Self::ErrorType
            | Self::IsBlank
            | Self::IsErr
            | Self::IsError
            | Self::IsEven
            | Self::IsLogical
            | Self::IsNa
            | Self::IsNonText
            | Self::IsNumber
            | Self::IsOdd
            | Self::IsText
            | Self::N
            | Self::T
            | Self::Type => CallContract::uniform(Arity::exact(1), ARRAY),
        }
    }
}

impl TextFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Left | Self::Right => {
                CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                    .with_defaults(LEFT_RIGHT_DEFAULTS)
            }
            Self::Mid => CallContract::uniform(Arity::exact(3), SCALAR),
            Self::Find | Self::Search => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, SCALAR])
                    .with_defaults(FIND_SEARCH_DEFAULTS)
            }
            Self::Substitute => {
                CallContract::positional(Arity::range(3, 4), &[SCALAR, SCALAR, SCALAR, SCALAR])
                    .with_defaults(SUBSTITUTE_DEFAULTS)
            }
            Self::Len | Self::Trim | Self::Upper | Self::Proper => {
                CallContract::uniform(Arity::exact(1), SCALAR)
            }
            Self::Exact | Self::Rept => CallContract::uniform(Arity::exact(2), SCALAR),
            Self::Replace => CallContract::uniform(Arity::exact(4), SCALAR),
            Self::Concat => CallContract::uniform(Arity::range(1, MAX_CONCAT_ARGUMENTS), ARRAY),
            Self::TextJoin => CallContract::repeating(
                Arity::range(3, MAX_TEXT_JOIN_ARGUMENTS),
                &[SCALAR, SCALAR],
                &[ARRAY],
                &[],
            ),
        }
    }
}

impl TextAdditionalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Char | Self::Clean | Self::UniChar | Self::Unicode | Self::Value => {
                CallContract::uniform(Arity::exact(1), SCALAR)
            }
            Self::Concatenate => CallContract::uniform(Arity::range(1, 255), ARRAY),
            Self::Dollar => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(DOLLAR_DEFAULTS),
            Self::TextAfter | Self::TextBefore => CallContract::positional(
                Arity::range(2, 6),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR, DEFERRED],
            )
            .with_defaults(TEXT_BOUNDARY_DEFAULTS),
            Self::ValueToText => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(VALUE_TO_TEXT_DEFAULTS),
        }
    }
}

impl ModernTextFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::ArrayToText => CallContract::positional(Arity::range(1, 2), &[ARRAY, SCALAR])
                .with_defaults(ARRAY_TO_TEXT_DEFAULTS),
            Self::RegexExtract => {
                CallContract::positional(Arity::range(2, 4), &[SCALAR, SCALAR, SCALAR, SCALAR])
                    .with_defaults(REGEX_EXTRACT_DEFAULTS)
            }
            Self::RegexReplace => CallContract::positional(
                Arity::range(3, 5),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(REGEX_REPLACE_DEFAULTS),
            Self::RegexTest => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, SCALAR])
                    .with_defaults(REGEX_TEST_DEFAULTS)
            }
            Self::TextSplit => CallContract::positional(
                Arity::range(2, 6),
                &[SCALAR, ARRAY, ARRAY, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(TEXT_SPLIT_DEFAULTS)
            .with_missing(MissingArgumentPolicy::Preserve),
        }
    }
}

impl DateFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Now | Self::Today => CallContract::uniform(Arity::exact(0), SCALAR),
            Self::Date | Self::DateDif => CallContract::uniform(Arity::exact(3), SCALAR),
            Self::Year | Self::Month | Self::Day => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::EDate | Self::Eomonth => CallContract::uniform(Arity::exact(2), SCALAR),
            Self::YearFrac => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, SCALAR])
                    .with_defaults(ZERO_OPTION_DEFAULTS)
            }
            Self::Weekday => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(ONE_OPTION_DEFAULTS),
            Self::Workday | Self::NetworkDays => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, ARRAY])
                    .with_defaults(EMPTY_COLLECTION_DEFAULTS)
            }
        }
    }
}

impl DateAdditionalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Days => CallContract::uniform(Arity::exact(2), SCALAR),
            Self::Days360 => {
                CallContract::positional(Arity::range(2, 3), &[SCALAR, SCALAR, SCALAR])
                    .with_defaults(ZERO_OPTION_DEFAULTS)
            }
            Self::Hour | Self::IsoWeekNum | Self::Minute | Self::Second => {
                CallContract::uniform(Arity::exact(1), SCALAR)
            }
            Self::Time => CallContract::uniform(Arity::exact(3), SCALAR),
            Self::WeekNum => CallContract::positional(Arity::range(1, 2), &[SCALAR, SCALAR])
                .with_defaults(ONE_OPTION_DEFAULTS),
        }
    }
}

impl DynamicFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::ByCol | Self::ByRow => {
                CallContract::positional(Arity::exact(2), &[ARRAY, CALLABLE])
            }
            Self::IsOmitted => CallContract::uniform(Arity::exact(1), ArgumentMode::BindingName)
                .with_missing(MissingArgumentPolicy::Preserve),
            Self::Lambda => {
                CallContract::special(Arity::range(1, 254), ArgumentLayout::LambdaDefinition)
            }
            Self::Let => {
                CallContract::special(Arity::stepped(3, None, 2), ArgumentLayout::LetBindings)
            }
            Self::MakeArray => {
                CallContract::positional(Arity::exact(3), &[SCALAR, SCALAR, CALLABLE])
            }
            Self::Map => CallContract::special(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                ArgumentLayout::ArraysThenCallable,
            ),
            Self::Reduce | Self::Scan => {
                CallContract::positional(Arity::exact(3), &[SCALAR, ARRAY, CALLABLE])
                    .with_missing(MissingArgumentPolicy::Preserve)
            }
        }
    }
}

impl ArrayFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::ChooseCols | Self::ChooseRows => CallContract::repeating(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                &[ARRAY],
                &[ARRAY],
                &[],
            ),
            Self::Drop | Self::Take => {
                CallContract::positional(Arity::range(2, 3), &[ARRAY, SCALAR, SCALAR])
                    .with_defaults(TAKE_DROP_DEFAULTS)
            }
            Self::Expand => {
                CallContract::positional(Arity::range(2, 4), &[ARRAY, SCALAR, SCALAR, SCALAR])
                    .with_defaults(EXPAND_DEFAULTS)
            }
            Self::Filter => CallContract::positional(Arity::range(2, 3), &[ARRAY, ARRAY, DEFERRED])
                .with_defaults(FILTER_DEFAULTS),
            Self::HStack | Self::VStack => {
                CallContract::uniform(Arity::range(1, MAX_EXCEL_ARGUMENTS), ARRAY)
            }
            Self::MInverse => CallContract::uniform(Arity::exact(1), ARRAY),
            Self::MMult => CallContract::uniform(Arity::exact(2), ARRAY),
            Self::MUnit => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Sequence => {
                CallContract::positional(Arity::range(1, 4), &[SCALAR, SCALAR, SCALAR, SCALAR])
                    .with_defaults(SEQUENCE_DEFAULTS)
            }
            Self::Sort => {
                CallContract::positional(Arity::range(1, 4), &[ARRAY, SCALAR, SCALAR, SCALAR])
                    .with_defaults(SORT_DEFAULTS)
            }
            Self::SortBy => CallContract::repeating(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                &[ARRAY],
                &[ARRAY, SCALAR],
                &[],
            )
            .with_repeating_defaults(SORTBY_DEFAULTS),
            Self::ToCol | Self::ToRow => {
                CallContract::positional(Arity::range(1, 3), &[ARRAY, SCALAR, SCALAR])
                    .with_defaults(FLATTEN_DEFAULTS)
            }
            Self::Transpose => CallContract::uniform(Arity::exact(1), ARRAY),
            Self::TrimRange => {
                CallContract::positional(Arity::range(1, 3), &[ARRAY, SCALAR, SCALAR])
                    .with_defaults(TRIM_RANGE_DEFAULTS)
            }
            Self::Unique => CallContract::positional(Arity::range(1, 3), &[ARRAY, SCALAR, SCALAR])
                .with_defaults(UNIQUE_DEFAULTS),
            Self::WrapCols | Self::WrapRows => {
                CallContract::positional(Arity::range(2, 3), &[ARRAY, SCALAR, SCALAR])
                    .with_defaults(WRAP_DEFAULTS)
            }
        }
    }
}

impl RegressionFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::LinEst | Self::LogEst => {
                CallContract::positional(Arity::range(1, 4), &[ARRAY, ARRAY, SCALAR, SCALAR])
                    .with_missing(MissingArgumentPolicy::Preserve)
                    .with_defaults(REGRESSION_STATISTICS_DEFAULTS)
            }
            Self::Growth | Self::Trend => {
                CallContract::positional(Arity::range(1, 4), &[ARRAY, ARRAY, ARRAY, SCALAR])
                    .with_missing(MissingArgumentPolicy::Preserve)
                    .with_defaults(REGRESSION_PREDICTION_DEFAULTS)
            }
        }
    }
}

impl StatisticalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Correl
            | Self::CovarianceP
            | Self::Intercept
            | Self::Pearson
            | Self::Rsq
            | Self::Slope => CallContract::uniform(Arity::exact(2), ARRAY),
            Self::Large | Self::PercentileInc | Self::QuartileInc | Self::Small => {
                CallContract::positional(Arity::exact(2), &[ARRAY, SCALAR])
            }
            Self::MaxIfs | Self::MinIfs => CallContract::repeating(
                Arity::stepped(3, Some(MAX_EXTREME_IFS_ARGUMENTS), 2),
                &[REFERENCE],
                &[REFERENCE, SCALAR],
                &[],
            ),
            Self::Median | Self::ModeSingle | Self::StDevS | Self::VarS => {
                CallContract::uniform(Arity::range(1, MAX_EXCEL_ARGUMENTS), ARRAY)
            }
            Self::NormSDistLegacy => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::NormSDist => CallContract::uniform(Arity::exact(2), SCALAR),
            Self::PercentRankInc => {
                CallContract::positional(Arity::range(2, 3), &[ARRAY, SCALAR, SCALAR])
                    .with_defaults(PERCENT_RANK_DEFAULTS)
            }
            Self::RankEq => CallContract::positional(Arity::range(2, 3), &[SCALAR, ARRAY, SCALAR])
                .with_defaults(RANK_DEFAULTS),
        }
    }
}

impl StatisticalAdditionalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::AveDev
            | Self::AverageA
            | Self::DevSq
            | Self::GeoMean
            | Self::HarMean
            | Self::MaxA
            | Self::MinA
            | Self::StDevP
            | Self::VarP => CallContract::uniform(Arity::range(1, MAX_EXCEL_ARGUMENTS), ARRAY),
            Self::Gauss | Self::Phi => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::Standardize | Self::ExponDist | Self::PoissonDist => {
                CallContract::uniform(Arity::exact(3), SCALAR)
            }
            Self::NormDist => CallContract::uniform(Arity::exact(4), SCALAR),
        }
    }
}

impl DistributionFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Gamma | Self::GammaLnPrecise => CallContract::uniform(Arity::exact(1), SCALAR),
            Self::BinomDist | Self::GammaDist | Self::NegBinomDist => {
                CallContract::uniform(Arity::exact(4), SCALAR)
            }
            Self::BinomInv | Self::GammaInv | Self::NegBinomDistLegacy => {
                CallContract::uniform(Arity::exact(3), SCALAR)
            }
            Self::BinomDistRange => CallContract::uniform(Arity::range(3, 4), SCALAR),
        }
    }
}

impl FinancialFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::Fv | Self::Nper | Self::Pmt | Self::Pv => CallContract::positional(
                Arity::range(3, 5),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(FINANCIAL_FIVE_ARGUMENT_DEFAULTS),
            Self::Ipmt | Self::Ppmt => CallContract::positional(
                Arity::range(4, 6),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(FINANCIAL_SIX_ARGUMENT_DEFAULTS),
            Self::Npv => CallContract::repeating(
                Arity::range(2, MAX_EXCEL_ARGUMENTS),
                &[SCALAR],
                &[ARRAY],
                &[],
            ),
            Self::Irr => CallContract::positional(Arity::range(1, 2), &[ARRAY, SCALAR])
                .with_defaults(SOLVER_GUESS_DEFAULTS),
            Self::Xirr => CallContract::positional(Arity::range(2, 3), &[ARRAY, ARRAY, SCALAR])
                .with_defaults(XIRR_GUESS_DEFAULTS),
            Self::Rate => CallContract::positional(
                Arity::range(3, 6),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(RATE_DEFAULTS),
            Self::Sln => CallContract::uniform(Arity::exact(3), SCALAR),
            Self::Syd => CallContract::uniform(Arity::exact(4), SCALAR),
            Self::Db => CallContract::positional(
                Arity::range(4, 5),
                &[SCALAR, SCALAR, SCALAR, SCALAR, SCALAR],
            )
            .with_defaults(DB_DEFAULTS),
        }
    }
}

impl FinancialAdditionalFunction {
    const fn call_contract(self) -> CallContract {
        match self {
            Self::DollarDe | Self::DollarFr | Self::Effect | Self::Nominal => {
                CallContract::uniform(Arity::exact(2), SCALAR)
            }
            Self::FvSchedule => CallContract::positional(Arity::exact(2), &[SCALAR, ARRAY]),
            Self::Rri | Self::PDuration => CallContract::uniform(Arity::exact(3), SCALAR),
            Self::Mirr => CallContract::positional(Arity::exact(3), &[ARRAY, SCALAR, SCALAR]),
            Self::IsPmt => CallContract::uniform(Arity::exact(4), SCALAR),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_optional_arguments_have_explicit_absent_defaults() {
        for evaluator in Evaluator::all() {
            let contract = evaluator.call_contract();
            let ArgumentLayout::Positional(modes) = contract.layout() else {
                continue;
            };
            let Some(maximum) = contract.arity.maximum else {
                continue;
            };
            assert_eq!(usize::from(maximum), modes.len(), "{evaluator:?}");
            for position in contract.arity.minimum..maximum {
                assert!(
                    contract
                        .default_at(usize::from(position), DefaultTrigger::Absent)
                        .is_some(),
                    "{evaluator:?} argument {position} has no absent-value contract",
                );
            }
        }
    }

    #[test]
    fn layouts_cover_every_accepted_argument_position() {
        for evaluator in Evaluator::all() {
            let contract = evaluator.call_contract();
            let mut counts = vec![contract.arity.minimum];
            if let Some(maximum) = contract.arity.maximum {
                counts.push(maximum);
            } else {
                counts.push(contract.arity.minimum + contract.arity.step * 2);
            }
            for count in counts {
                if !contract.arity.accepts(usize::from(count)) {
                    continue;
                }
                for position in 0..count {
                    assert!(
                        contract
                            .layout()
                            .mode_at(usize::from(position), usize::from(count))
                            .is_some(),
                        "{evaluator:?} argument {position} of {count}",
                    );
                }
            }
        }
    }

    #[test]
    fn defaults_preserve_absent_and_explicit_missing_distinctions() {
        let index = Evaluator::Legacy(LegacyFunction::Index).call_contract();
        assert!(index.arity().accepts(4));
        assert_eq!(
            index.default_at(1, DefaultTrigger::Missing),
            Some(ArgumentDefaultValue::Number(0.0)),
        );
        assert_eq!(
            index.default_at(2, DefaultTrigger::Absent),
            Some(ArgumentDefaultValue::IndexColumn),
        );

        let match_function = Evaluator::Legacy(LegacyFunction::Match).call_contract();
        assert_eq!(
            match_function.default_at(2, DefaultTrigger::Absent),
            Some(ArgumentDefaultValue::Number(1.0)),
        );
        assert_eq!(
            match_function.missing_behavior_at(2),
            MissingArgumentBehavior::CoerceToBlank,
        );

        let indirect = Evaluator::Lookup(LookupFunction::Indirect).call_contract();
        assert_eq!(
            indirect.default_at(1, DefaultTrigger::Absent),
            Some(ArgumentDefaultValue::Logical(true)),
        );
        assert_eq!(
            indirect.missing_behavior_at(1),
            MissingArgumentBehavior::CoerceToBlank,
        );

        let sort_by = Evaluator::Array(ArrayFunction::SortBy).call_contract();
        for position in [2, 4, 254] {
            assert_eq!(
                sort_by.default_at(position, DefaultTrigger::Absent),
                Some(ArgumentDefaultValue::Number(1.0)),
                "SORTBY order at position {position} must default to ascending",
            );
            assert_eq!(
                sort_by.default_at(position, DefaultTrigger::Missing),
                Some(ArgumentDefaultValue::Number(1.0)),
                "SORTBY missing order at position {position} must default to ascending",
            );
        }
        assert_eq!(sort_by.default_at(3, DefaultTrigger::Absent), None);
    }

    #[test]
    fn function_specific_variable_arity_limits_match_excel() {
        for (evaluator, maximum) in [
            (Evaluator::Logical(LogicalFunction::Switch), 254_usize),
            (Evaluator::Text(TextFunction::Concat), 253),
            (Evaluator::Text(TextFunction::TextJoin), 254),
            (Evaluator::Statistical(StatisticalFunction::MaxIfs), 253),
            (Evaluator::Statistical(StatisticalFunction::MinIfs), 253),
        ] {
            let arity = evaluator.call_contract().arity();
            assert!(arity.accepts(maximum), "{evaluator:?}");
            assert!(!arity.accepts(maximum + 1), "{evaluator:?}");
        }
    }
}
