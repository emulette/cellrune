use std::collections::BTreeSet;

use super::ast::Expr;
use super::value::Value;

const MAX_LAMBDA_PARAMETERS: usize = 253;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LambdaBinding {
    name: String,
    value: Value,
}

impl LambdaBinding {
    pub(super) fn new(name: String, value: Value) -> Self {
        Self { name, value }
    }

    fn matches(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(parameter_base(name))
    }

    pub(super) fn value(&self) -> &Value {
        &self.value
    }

    pub(super) fn set_value(&mut self, value: Value) {
        self.value = value;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LambdaDefinition<'formula> {
    parameters: Vec<String>,
    body: &'formula Expr,
}

impl<'formula> LambdaDefinition<'formula> {
    pub(super) fn parameters(&self) -> &[String] {
        &self.parameters
    }

    pub(super) const fn body(&self) -> &'formula Expr {
        self.body
    }
}

pub(super) fn definition(expr: &Expr) -> Option<LambdaDefinition<'_>> {
    let Expr::Call { name, args } = expr else {
        return None;
    };
    if !is_lambda_function(name) || args.is_empty() || args.len() > MAX_LAMBDA_PARAMETERS + 1 {
        return None;
    }
    let (body, raw_parameters) = args.split_last()?;
    let mut seen = BTreeSet::new();
    let mut parameters = Vec::with_capacity(raw_parameters.len());
    for parameter in raw_parameters {
        let Expr::Name(name) = parameter else {
            return None;
        };
        let canonical = canonical_parameter_name(name);
        if canonical.is_empty() || canonical.contains('.') || !seen.insert(canonical.clone()) {
            return None;
        }
        parameters.push(canonical);
    }
    Some(LambdaDefinition { parameters, body })
}

pub(super) fn binding_value<'bindings>(
    bindings: &'bindings [LambdaBinding],
    name: &str,
) -> Option<&'bindings Value> {
    bindings
        .iter()
        .rev()
        .find(|binding| binding.matches(name))
        .map(LambdaBinding::value)
}

pub(super) fn canonical_parameter_name(name: &str) -> String {
    parameter_base(name).to_ascii_lowercase()
}

fn parameter_base(name: &str) -> &str {
    name.get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("_xlpm."))
        .map_or(name, |_| &name[6..])
}

fn is_lambda_function(name: &str) -> bool {
    let base = name
        .get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("_xlfn."))
        .map_or(name, |_| &name[6..]);
    base.eq_ignore_ascii_case("LAMBDA")
}
