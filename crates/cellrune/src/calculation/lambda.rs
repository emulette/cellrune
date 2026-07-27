use std::collections::BTreeSet;

use super::ast::Expr;
use super::functions::normalize_name;
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

pub(super) fn is_lambda_local(name: &str, scope: &[String]) -> bool {
    let key = canonical_parameter_name(name);
    scope.iter().rev().any(|local| local == &key)
}

pub(super) fn walk_lambda_scope<F>(
    name: &str,
    args: &[Expr],
    scope: &mut Vec<String>,
    mut walk: F,
) -> bool
where
    F: FnMut(&Expr, &mut Vec<String>),
{
    if normalize_name(name) != "MAP" {
        return false;
    }
    let Some((lambda_expr, array_exprs)) = args.split_last() else {
        return false;
    };
    let Some(lambda) = definition(lambda_expr) else {
        return false;
    };
    for arg in array_exprs {
        walk(arg, scope);
    }
    let previous_local_count = scope.len();
    scope.extend(lambda.parameters().iter().cloned());
    walk(lambda.body(), scope);
    scope.truncate(previous_local_count);
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map_expression() -> Expr {
        Expr::Call {
            name: "MAP".to_owned(),
            args: vec![
                Expr::Name("outside".to_owned()),
                Expr::Call {
                    name: "LAMBDA".to_owned(),
                    args: vec![
                        Expr::Name("_xlpm.Item".to_owned()),
                        Expr::Name("item".to_owned()),
                    ],
                },
            ],
        }
    }

    #[test]
    fn lambda_scope_walks_arguments_before_the_scoped_body() {
        let Expr::Call { name, args } = map_expression() else {
            unreachable!("fixture is a call");
        };
        let mut scope = vec!["outer".to_owned()];
        let mut observations = Vec::new();

        assert!(walk_lambda_scope(
            &name,
            &args,
            &mut scope,
            |expr, active_scope| {
                let Expr::Name(name) = expr else {
                    panic!("fixture callback receives names");
                };
                observations.push((name.clone(), active_scope.clone()));
            }
        ));

        assert_eq!(
            observations,
            vec![
                ("outside".to_owned(), vec!["outer".to_owned()]),
                (
                    "item".to_owned(),
                    vec!["outer".to_owned(), "item".to_owned()]
                ),
            ]
        );
        assert_eq!(scope, vec!["outer"]);
    }

    #[test]
    fn lambda_local_matching_is_case_insensitive_and_prefix_agnostic() {
        let scope = vec!["item".to_owned()];

        assert!(is_lambda_local("ITEM", &scope));
        assert!(is_lambda_local("_xlpm.Item", &scope));
        assert!(!is_lambda_local("other", &scope));
    }
}
