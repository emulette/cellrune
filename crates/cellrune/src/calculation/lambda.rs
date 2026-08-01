use std::collections::BTreeSet;

use super::ast::Expr;
use super::functions::{
    DynamicFunction, Evaluator, function_arguments_are_reachable, function_evaluator,
};
use super::scope::canonical_local_name;
use super::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

const MAX_LAMBDA_PARAMETERS: usize = 253;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalNamePolicy {
    Let,
    Lambda,
}

impl LocalNamePolicy {
    fn allows_period(self) -> bool {
        matches!(self, Self::Let)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedLocalName(String);

impl ValidatedLocalName {
    pub(super) fn into_string(self) -> String {
        self.0
    }
}

pub(super) fn validate_local_name(
    name: &str,
    policy: LocalNamePolicy,
) -> Option<ValidatedLocalName> {
    let canonical = canonical_local_name(name);
    if canonical.chars().count() > 255 {
        return None;
    }
    let mut characters = canonical.chars();
    let first = characters.next()?;
    if !(first.is_alphabetic() || matches!(first, '_' | '\\')) {
        return None;
    }
    if characters.any(|character| {
        !(character.is_alphanumeric()
            || character == '_'
            || (policy.allows_period() && character == '.'))
    }) {
        return None;
    }
    if conflicts_with_reference_syntax(&canonical) {
        return None;
    }
    Some(ValidatedLocalName(canonical))
}

pub(super) fn is_local_name(name: &str, scope: &[String]) -> bool {
    let key = canonical_local_name(name);
    scope.iter().rev().any(|local| local == &key)
}

pub(super) fn walk_local_scope<F>(
    name: &str,
    args: &[Expr],
    scope: &mut Vec<String>,
    max_let_bindings: u64,
    walk: F,
) -> bool
where
    F: FnMut(&Expr, &mut Vec<String>),
{
    let evaluator = function_evaluator(name);
    if matches!(
        evaluator,
        Some(Evaluator::Dynamic(
            DynamicFunction::Map | DynamicFunction::Lambda | DynamicFunction::Let
        ))
    ) && !function_arguments_are_reachable(name, args, max_let_bindings)
    {
        return true;
    }
    match evaluator {
        Some(Evaluator::Dynamic(DynamicFunction::Map)) => walk_map_scope(args, scope, walk),
        Some(Evaluator::Dynamic(DynamicFunction::Lambda)) => walk_lambda_scope(args, scope, walk),
        Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
            walk_let_scope(args, scope, walk);
            true
        }
        _ => false,
    }
}

fn walk_lambda_scope<F>(args: &[Expr], scope: &mut Vec<String>, mut walk: F) -> bool
where
    F: FnMut(&Expr, &mut Vec<String>),
{
    let Some(lambda) = definition_from_args(args) else {
        return false;
    };
    let previous_local_count = scope.len();
    scope.extend(lambda.parameters().iter().cloned());
    walk(lambda.body(), scope);
    scope.truncate(previous_local_count);
    true
}

fn walk_map_scope<F>(args: &[Expr], scope: &mut Vec<String>, mut walk: F) -> bool
where
    F: FnMut(&Expr, &mut Vec<String>),
{
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

fn walk_let_scope<F>(args: &[Expr], scope: &mut Vec<String>, mut walk: F)
where
    F: FnMut(&Expr, &mut Vec<String>),
{
    let previous_local_count = scope.len();
    let Some((final_expr, pairs)) = args.split_last() else {
        return;
    };
    for pair in pairs.chunks_exact(2) {
        walk(&pair[1], scope);
        if let Expr::Name(name) = &pair[0] {
            scope.push(canonical_local_name(name));
        }
    }
    walk(final_expr, scope);
    scope.truncate(previous_local_count);
}

fn conflicts_with_reference_syntax(name: &str) -> bool {
    conflicts_with_a1_reference(name) || conflicts_with_r1c1_reference(name)
}

fn conflicts_with_a1_reference(name: &str) -> bool {
    let letter_count = name.bytes().take_while(u8::is_ascii_alphabetic).count();
    if letter_count == 0 || letter_count == name.len() {
        return false;
    }
    let (letters, digits) = name.split_at(letter_count);
    if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Some(row) = digits.parse::<u32>().ok() else {
        return false;
    };
    let column = letters.bytes().try_fold(0_u32, |column, byte| {
        column
            .checked_mul(26)?
            .checked_add(u32::from(byte.to_ascii_uppercase() - b'A' + 1))
    });
    column.is_some_and(|column| column <= EXCEL_MAX_COLUMNS) && row <= EXCEL_MAX_ROWS
}

fn conflicts_with_r1c1_reference(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if matches!(upper.as_str(), "R" | "C") {
        return true;
    }
    let Some(after_r) = upper.strip_prefix('R') else {
        return false;
    };
    let r_digits = after_r.bytes().take_while(u8::is_ascii_digit).count();
    let (row, after_row) = after_r.split_at(r_digits);
    let Some(after_c) = after_row.strip_prefix('C') else {
        return false;
    };
    if !after_c.bytes().all(|byte| byte.is_ascii_digit())
        || (!row.is_empty() && row.starts_with('0'))
        || (!after_c.is_empty() && after_c.starts_with('0'))
    {
        return false;
    }
    let row_is_valid = row.is_empty()
        || row
            .parse::<u32>()
            .ok()
            .is_some_and(|row| row <= EXCEL_MAX_ROWS);
    let column_is_valid = after_c.is_empty()
        || after_c
            .parse::<u32>()
            .ok()
            .is_some_and(|column| column <= EXCEL_MAX_COLUMNS);
    row_is_valid && column_is_valid
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
    if !is_lambda_function(name) {
        return None;
    }
    definition_from_args(args)
}

pub(super) fn definition_from_args(args: &[Expr]) -> Option<LambdaDefinition<'_>> {
    if args.is_empty() || args.len() > MAX_LAMBDA_PARAMETERS + 1 {
        return None;
    }
    let (body, raw_parameters) = args.split_last()?;
    if matches!(body, Expr::Missing) {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut parameters = Vec::with_capacity(raw_parameters.len());
    for parameter in raw_parameters {
        let Expr::Name(name) = parameter else {
            return None;
        };
        let canonical = validate_local_name(name, LocalNamePolicy::Lambda)?.into_string();
        if !seen.insert(canonical.clone()) {
            return None;
        }
        parameters.push(canonical);
    }
    Some(LambdaDefinition { parameters, body })
}

fn is_lambda_function(name: &str) -> bool {
    function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Lambda))
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
    fn local_scope_walks_map_arguments_before_the_scoped_body() {
        let Expr::Call { name, args } = map_expression() else {
            unreachable!("fixture is a call");
        };
        let mut scope = vec!["outer".to_owned()];
        let mut observations = Vec::new();

        assert!(walk_local_scope(
            &name,
            &args,
            &mut scope,
            u64::MAX,
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
    fn local_scope_walks_let_values_sequentially() {
        let args = vec![
            Expr::Name("first".to_owned()),
            Expr::Name("outside".to_owned()),
            Expr::Name("second".to_owned()),
            Expr::Name("first".to_owned()),
            Expr::Name("second".to_owned()),
        ];
        let mut scope = vec!["outer".to_owned()];
        let mut observations = Vec::new();

        assert!(walk_local_scope(
            "LET",
            &args,
            &mut scope,
            u64::MAX,
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
                ("outside".to_owned(), vec!["outer".to_owned()],),
                (
                    "first".to_owned(),
                    vec!["outer".to_owned(), "first".to_owned()],
                ),
                (
                    "second".to_owned(),
                    vec!["outer".to_owned(), "first".to_owned(), "second".to_owned(),],
                ),
            ]
        );
        assert_eq!(scope, vec!["outer"]);
    }

    #[test]
    fn local_matching_is_case_insensitive_and_prefix_agnostic() {
        let scope = vec!["item".to_owned()];

        assert!(is_local_name("ITEM", &scope));
        assert!(is_local_name("_xlpm.Item", &scope));
        assert!(!is_local_name("other", &scope));
    }

    #[test]
    fn local_name_validation_rejects_reference_conflicts_and_policy_violations() {
        for invalid in [
            "A1",
            "XFD1048576",
            "R1C1",
            "RC",
            "R",
            "c",
            "1name",
            "has space",
        ] {
            assert!(validate_local_name(invalid, LocalNamePolicy::Let).is_none());
        }
        assert!(validate_local_name("XFE1", LocalNamePolicy::Let).is_some());
        assert!(validate_local_name("_xlpm.total.value", LocalNamePolicy::Let).is_some());
        assert!(validate_local_name("total.value", LocalNamePolicy::Lambda).is_none());
        assert_eq!(
            validate_local_name("_xlpm.Total", LocalNamePolicy::Lambda)
                .expect("valid parameter")
                .into_string(),
            "total"
        );
    }
}
