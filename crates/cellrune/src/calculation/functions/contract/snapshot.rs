use std::fmt::Write as _;

use super::{
    ArgumentDefaultValue, ArgumentLayout, ArgumentMode, CallContract, DefaultTrigger,
    MissingArgumentPolicy,
};

impl CallContract {
    pub(in crate::calculation::functions) fn stable_snapshot(self) -> String {
        let mut snapshot = String::new();
        write!(
            snapshot,
            "arity={}:{}:{};layout=",
            self.arity.minimum,
            self.arity
                .maximum
                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
            self.arity.step,
        )
        .expect("writing to String cannot fail");
        write_layout(&mut snapshot, self.layout);
        write!(
            snapshot,
            ";missing={};defaults=",
            missing_policy_name(self.missing)
        )
        .expect("writing to String cannot fail");
        for default in self.defaults {
            write!(
                snapshot,
                "{}:{}:",
                default.position,
                trigger_name(default.trigger)
            )
            .expect("writing to String cannot fail");
            write_default_value(&mut snapshot, default.value);
            snapshot.push(',');
        }
        snapshot.push_str(";repeating_defaults=");
        for default in self.repeating_defaults {
            write!(
                snapshot,
                "{}:{}:{}:",
                default.first_position,
                default.step,
                trigger_name(default.trigger)
            )
            .expect("writing to String cannot fail");
            write_default_value(&mut snapshot, default.value);
            snapshot.push(',');
        }
        snapshot
    }
}

fn write_layout(snapshot: &mut String, layout: ArgumentLayout) {
    match layout {
        ArgumentLayout::Uniform(mode) => {
            write!(snapshot, "uniform:{}", argument_mode_name(mode))
                .expect("writing to String cannot fail");
        }
        ArgumentLayout::Positional(modes) => {
            snapshot.push_str("positional:");
            write_modes(snapshot, modes);
        }
        ArgumentLayout::Repeating {
            leading,
            repeated,
            trailing,
        } => {
            snapshot.push_str("repeating:");
            write_modes(snapshot, leading);
            snapshot.push('/');
            write_modes(snapshot, repeated);
            snapshot.push('/');
            write_modes(snapshot, trailing);
        }
        ArgumentLayout::LetBindings => snapshot.push_str("let_bindings"),
        ArgumentLayout::LambdaDefinition => snapshot.push_str("lambda_definition"),
        ArgumentLayout::ArraysThenCallable => snapshot.push_str("arrays_then_callable"),
    }
}

fn write_modes(snapshot: &mut String, modes: &[ArgumentMode]) {
    for mode in modes {
        snapshot.push_str(argument_mode_name(*mode));
        snapshot.push(',');
    }
}

const fn argument_mode_name(mode: ArgumentMode) -> &'static str {
    match mode {
        ArgumentMode::Scalar => "scalar",
        ArgumentMode::Array => "array",
        ArgumentMode::Reference => "reference",
        ArgumentMode::Callable => "callable",
        ArgumentMode::Deferred => "deferred",
        ArgumentMode::BindingName => "binding_name",
    }
}

const fn trigger_name(trigger: DefaultTrigger) -> &'static str {
    match trigger {
        DefaultTrigger::Absent => "absent",
        DefaultTrigger::Missing => "missing",
        DefaultTrigger::AbsentOrMissing => "absent_or_missing",
    }
}

const fn missing_policy_name(policy: MissingArgumentPolicy) -> &'static str {
    match policy {
        MissingArgumentPolicy::CoerceToBlank => "coerce_to_blank",
        MissingArgumentPolicy::Preserve => "preserve",
    }
}

fn write_default_value(snapshot: &mut String, value: ArgumentDefaultValue) {
    match value {
        ArgumentDefaultValue::Omitted => snapshot.push_str("omitted"),
        ArgumentDefaultValue::Number(number) => {
            write!(snapshot, "number:{:016x}", number.to_bits())
                .expect("writing to String cannot fail");
        }
        ArgumentDefaultValue::Logical(value) => {
            snapshot.push_str(if value {
                "logical:true"
            } else {
                "logical:false"
            });
        }
        ArgumentDefaultValue::NotAvailable => snapshot.push_str("not_available"),
        ArgumentDefaultValue::CalculationError => snapshot.push_str("calculation_error"),
        ArgumentDefaultValue::CallerReference => snapshot.push_str("caller_reference"),
        ArgumentDefaultValue::CriteriaRange => snapshot.push_str("criteria_range"),
        ArgumentDefaultValue::EmptyCollection => snapshot.push_str("empty_collection"),
        ArgumentDefaultValue::IndexColumn => snapshot.push_str("index_column"),
        ArgumentDefaultValue::LookupVector => snapshot.push_str("lookup_vector"),
        ArgumentDefaultValue::NoPadding => snapshot.push_str("no_padding"),
        ArgumentDefaultValue::NoSheetQualifier => snapshot.push_str("no_sheet_qualifier"),
        ArgumentDefaultValue::NoUpperBound => snapshot.push_str("no_upper_bound"),
        ArgumentDefaultValue::AllOccurrences => snapshot.push_str("all_occurrences"),
        ArgumentDefaultValue::SourceRows => snapshot.push_str("source_rows"),
        ArgumentDefaultValue::SourceColumns => snapshot.push_str("source_columns"),
        ArgumentDefaultValue::LinkLocation => snapshot.push_str("link_location"),
    }
}
