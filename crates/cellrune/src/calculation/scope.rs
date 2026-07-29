use std::sync::Arc;

use super::decimal::DecimalTrace;
use super::operators::broadcast_index;
use super::runtime::{Array, RectSpan};
use super::value::{ErrorKind, Value};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScalarEvaluation {
    pub(super) value: Value,
    pub(super) decimal_trace: Option<DecimalTrace>,
}

impl ScalarEvaluation {
    pub(super) const fn untracked(value: Value) -> Self {
        Self {
            value,
            decimal_trace: None,
        }
    }

    pub(super) fn engine_issue(&self) -> Option<ErrorKind> {
        match self.value {
            Value::Error(kind) if kind.is_engine_issue() => Some(kind),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ArrayEvaluation {
    pub(super) array: Array,
    pub(super) decimal_traces: Vec<Option<DecimalTrace>>,
}

impl ArrayEvaluation {
    /// Exact decimal of the element read at this broadcast position.
    pub(super) fn decimal_at(&self, row: u32, column: u32) -> Option<DecimalTrace> {
        let index = broadcast_index(self.array.rows, self.array.cols, row, column)?;
        self.decimal_traces.get(index).copied().flatten()
    }

    pub(super) fn untracked(array: Array) -> Self {
        let decimal_traces = vec![None; array.data.len()];
        Self {
            array,
            decimal_traces,
        }
    }

    pub(super) fn scalar(evaluated: ScalarEvaluation) -> Self {
        Self {
            array: Array::scalar(evaluated.value),
            decimal_traces: vec![evaluated.decimal_trace],
        }
    }

    pub(super) fn engine_issue(&self) -> Option<ErrorKind> {
        self.array.data.iter().find_map(|value| match value {
            Value::Error(kind) if kind.is_engine_issue() => Some(*kind),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ScopeValue {
    Missing,
    Scalar(ScalarEvaluation),
    Array(Arc<ArrayEvaluation>),
    Reference(RectSpan),
}

impl ScopeValue {
    pub(super) fn engine_issue(&self) -> Option<ErrorKind> {
        match self {
            Self::Scalar(evaluated) => evaluated.engine_issue(),
            Self::Array(evaluated) => evaluated.engine_issue(),
            Self::Missing | Self::Reference(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScopeEntry {
    name: Box<str>,
    value: Arc<ScopeValue>,
}

impl ScopeEntry {
    pub(super) fn new(name: String, value: ScopeValue) -> Self {
        Self {
            name: name.into_boxed_str(),
            value: Arc::new(value),
        }
    }

    pub(super) fn placeholder(name: String) -> Self {
        Self::new(name, ScopeValue::Missing)
    }

    fn matches(&self, name: &str) -> bool {
        self.name.as_ref() == canonical_local_name(name)
    }

    fn value(&self) -> &ScopeValue {
        self.value.as_ref()
    }

    pub(super) fn set_value(&mut self, value: ScopeValue) {
        self.value = Arc::new(value);
    }
}

pub(super) fn scope_value<'scope>(
    entries: &'scope [ScopeEntry],
    name: &str,
) -> Option<&'scope ScopeValue> {
    entries
        .iter()
        .rev()
        .find(|entry| entry.matches(name))
        .map(ScopeEntry::value)
}

pub(super) fn canonical_local_name(name: &str) -> String {
    local_name_base(name).to_lowercase()
}

fn local_name_base(name: &str) -> &str {
    name.get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("_xlpm."))
        .map_or(name, |_| &name[6..])
}
