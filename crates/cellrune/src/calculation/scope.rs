use std::sync::Arc;

use super::functions::BuiltinCallable;
use super::runtime::{Array, ReferenceValue};
use super::value::{ErrorKind, Value};
use crate::{DefinedName, DefinedNameScope, SheetId, WorkbookSnapshot};

pub(super) fn resolve_defined_name_scoped<'a>(
    workbook: &'a WorkbookSnapshot,
    ambient_sheet: Option<SheetId>,
    lookup_scope: Option<DefinedNameScope>,
    name: &str,
) -> Option<(usize, &'a DefinedName)> {
    match lookup_scope {
        Some(DefinedNameScope::Workbook) => workbook.defined_name(DefinedNameScope::Workbook, name),
        Some(DefinedNameScope::Sheet(sheet_id)) => workbook
            .defined_name(DefinedNameScope::Sheet(sheet_id), name)
            .or_else(|| workbook.defined_name(DefinedNameScope::Workbook, name)),
        None => ambient_sheet
            .and_then(|sheet_id| workbook.defined_name(DefinedNameScope::Sheet(sheet_id), name))
            .or_else(|| workbook.defined_name(DefinedNameScope::Workbook, name)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScalarEvaluation {
    pub(super) value: Value,
}

impl ScalarEvaluation {
    pub(super) const fn new(value: Value) -> Self {
        Self { value }
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
}

impl ArrayEvaluation {
    pub(super) const fn new(array: Array) -> Self {
        Self { array }
    }

    pub(super) fn scalar(evaluated: ScalarEvaluation) -> Self {
        Self::new(Array::scalar(evaluated.value))
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
    Reference(ReferenceValue),
    Callable(CallableValue),
}

impl ScopeValue {
    pub(super) fn engine_issue(&self) -> Option<ErrorKind> {
        match self {
            Self::Scalar(evaluated) => evaluated.engine_issue(),
            Self::Array(evaluated) => evaluated.engine_issue(),
            Self::Missing | Self::Reference(_) | Self::Callable(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum CallableValue {
    Lambda(Arc<LambdaClosure>),
    Builtin(BuiltinCallable),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LambdaClosure {
    pub(super) parameters: Vec<String>,
    pub(super) body: super::ast::Expr,
    pub(super) captured: Vec<ScopeEntry>,
    pub(super) lookup_scope: Option<DefinedNameScope>,
    pub(super) defined_name: Option<DefinedLambdaId>,
    pub(super) prepared_arguments: super::prepared_arguments::PreparedArguments,
}

impl LambdaClosure {
    /// The body with the prepared arguments it keeps.
    pub(super) fn argument_owner(&self) -> super::prepared_arguments::ArgumentOwner<'_> {
        super::prepared_arguments::ArgumentOwner::new(&self.body, &self.prepared_arguments)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct DefinedLambdaId {
    scope: DefinedNameScope,
    lookup_key: Box<str>,
}

impl DefinedLambdaId {
    pub(super) fn from_defined_name(defined_name: &DefinedName) -> Self {
        Self {
            scope: defined_name.scope(),
            lookup_key: defined_name.lookup_key().into(),
        }
    }

    pub(super) const fn scope(&self) -> DefinedNameScope {
        self.scope
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

    /// Replaces the bound value, reusing its allocation when nothing else holds it.
    pub(super) fn set_value(&mut self, value: ScopeValue) {
        match Arc::get_mut(&mut self.value) {
            Some(bound) => *bound = value,
            None => self.value = Arc::new(value),
        }
    }

    fn matches(&self, name: &str) -> bool {
        self.name.as_ref() == canonical_local_name(name)
    }

    fn value(&self) -> &ScopeValue {
        self.value.as_ref()
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
