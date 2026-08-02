use std::cmp::Ordering;

use super::super::coerce::compare_text_case_insensitive;
use super::super::criteria::{CompiledWildcardPattern, charge_text_comparison_work};
use super::super::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseCriteriaOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
enum DatabaseCriteriaRhs {
    Number(f64),
    EqualityText(CompiledWildcardPattern),
    OrderedText(String),
    Logical(bool),
    Blank,
    Error(ErrorKind),
}

/// A database-field criterion with Excel's database-specific bare-text prefix semantics.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CompiledDatabaseCriteria {
    op: DatabaseCriteriaOp,
    rhs: DatabaseCriteriaRhs,
}

impl CompiledDatabaseCriteria {
    pub(super) fn compile_with_work(
        value: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        let (op, rhs) = match value {
            Value::Error(kind) if kind.is_engine_issue() => return Err(*kind),
            Value::Error(kind) => (DatabaseCriteriaOp::Eq, DatabaseCriteriaRhs::Error(*kind)),
            Value::Number(number) => (DatabaseCriteriaOp::Eq, DatabaseCriteriaRhs::Number(*number)),
            Value::Logical(logical) => (
                DatabaseCriteriaOp::Eq,
                DatabaseCriteriaRhs::Logical(*logical),
            ),
            Value::Blank => (DatabaseCriteriaOp::Eq, DatabaseCriteriaRhs::Blank),
            Value::Text(text) => {
                on_work(text.len() as u64)?;
                let (op, rest, explicit_operator) = parse_operator(text);
                let rhs = if rest.is_empty() {
                    DatabaseCriteriaRhs::Blank
                } else if let Ok(number) = rest.trim().parse::<f64>() {
                    DatabaseCriteriaRhs::Number(number)
                } else if rest.eq_ignore_ascii_case("TRUE") {
                    DatabaseCriteriaRhs::Logical(true)
                } else if rest.eq_ignore_ascii_case("FALSE") {
                    DatabaseCriteriaRhs::Logical(false)
                } else if matches!(op, DatabaseCriteriaOp::Eq | DatabaseCriteriaOp::Ne) {
                    let mut pattern = CompiledWildcardPattern::compile_precharged(rest);
                    if !explicit_operator {
                        pattern.push_any_sequence();
                    }
                    DatabaseCriteriaRhs::EqualityText(pattern)
                } else {
                    DatabaseCriteriaRhs::OrderedText(rest.to_owned())
                };
                (op, rhs)
            }
        };
        Ok(Self { op, rhs })
    }

    pub(super) fn matches_with_work(
        &self,
        cell: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        match self.op {
            DatabaseCriteriaOp::Eq => self.eq_matches(cell, &mut on_work),
            DatabaseCriteriaOp::Ne => Ok(!self.eq_matches(cell, &mut on_work)?),
            DatabaseCriteriaOp::Lt => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering == Ordering::Less)
            }
            DatabaseCriteriaOp::Le => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering != Ordering::Greater)
            }
            DatabaseCriteriaOp::Gt => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering == Ordering::Greater)
            }
            DatabaseCriteriaOp::Ge => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering != Ordering::Less)
            }
        }
    }

    fn eq_matches(
        &self,
        cell: &Value,
        on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        match &self.rhs {
            DatabaseCriteriaRhs::Blank => Ok(cell.is_blank_like()),
            DatabaseCriteriaRhs::Number(expected) => match cell {
                Value::Number(actual) => Ok(actual == expected),
                Value::Text(text) => {
                    on_work(text.len() as u64)?;
                    Ok(text
                        .trim()
                        .parse::<f64>()
                        .is_ok_and(|actual| actual == *expected))
                }
                _ => Ok(false),
            },
            DatabaseCriteriaRhs::EqualityText(pattern) => match cell {
                Value::Text(text) if !text.is_empty() => pattern.matches_with_work(text, on_work),
                _ => Ok(false),
            },
            DatabaseCriteriaRhs::OrderedText(_) => {
                unreachable!("ordered text is constructed only for ordering operators")
            }
            DatabaseCriteriaRhs::Logical(expected) => {
                Ok(matches!(cell, Value::Logical(actual) if actual == expected))
            }
            DatabaseCriteriaRhs::Error(expected) => {
                Ok(matches!(cell, Value::Error(actual) if actual == expected))
            }
        }
    }

    fn ord_matches(
        &self,
        cell: &Value,
        on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
        accept: impl Fn(Ordering) -> bool,
    ) -> Result<bool, ErrorKind> {
        match &self.rhs {
            DatabaseCriteriaRhs::Number(expected) => {
                let actual = match cell {
                    Value::Number(number) => Some(*number),
                    Value::Text(text) => {
                        on_work(text.len() as u64)?;
                        text.trim().parse::<f64>().ok()
                    }
                    _ => None,
                };
                Ok(actual
                    .and_then(|actual| actual.partial_cmp(expected))
                    .is_some_and(accept))
            }
            DatabaseCriteriaRhs::OrderedText(expected) => match cell {
                Value::Text(text) if !text.is_empty() => {
                    charge_text_comparison_work(text, expected, on_work)?;
                    Ok(accept(compare_text_case_insensitive(text, expected)))
                }
                _ => Ok(false),
            },
            DatabaseCriteriaRhs::EqualityText(_) => {
                unreachable!("equality text is constructed only for equality operators")
            }
            DatabaseCriteriaRhs::Logical(expected) => Ok(matches!(
                cell,
                Value::Logical(actual) if accept(actual.cmp(expected))
            )),
            DatabaseCriteriaRhs::Blank | DatabaseCriteriaRhs::Error(_) => Ok(false),
        }
    }
}

fn parse_operator(text: &str) -> (DatabaseCriteriaOp, &str, bool) {
    for (prefix, op) in [
        ("<>", DatabaseCriteriaOp::Ne),
        ("<=", DatabaseCriteriaOp::Le),
        (">=", DatabaseCriteriaOp::Ge),
        ("<", DatabaseCriteriaOp::Lt),
        (">", DatabaseCriteriaOp::Gt),
        ("=", DatabaseCriteriaOp::Eq),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (op, rest, true);
        }
    }
    (DatabaseCriteriaOp::Eq, text, false)
}

#[cfg(test)]
mod tests {
    use super::CompiledDatabaseCriteria;
    use crate::calculation::value::Value;

    #[test]
    fn bare_text_is_prefix_while_explicit_equality_is_exact() {
        let bare =
            CompiledDatabaseCriteria::compile_with_work(&Value::Text("Dav".to_owned()), |_| Ok(()))
                .expect("valid database criterion");
        let exact = CompiledDatabaseCriteria::compile_with_work(
            &Value::Text("=Dav".to_owned()),
            |_| Ok(()),
        )
        .expect("valid database criterion");

        assert_eq!(
            bare.matches_with_work(&Value::Text("David".to_owned()), |_| Ok(())),
            Ok(true)
        );
        assert_eq!(
            exact.matches_with_work(&Value::Text("David".to_owned()), |_| Ok(())),
            Ok(false)
        );
        assert_eq!(
            exact.matches_with_work(&Value::Text("DAV".to_owned()), |_| Ok(())),
            Ok(true)
        );
    }
}
