use std::cmp::Ordering;

use super::coerce::compare_text_case_insensitive;
use super::limits::CalculationLimitKind;
use super::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriteriaOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CriteriaRhs {
    Number(f64),
    Text(String),
    Logical(bool),
    Blank,
    Error(ErrorKind),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Criteria {
    pub op: CriteriaOp,
    pub rhs: CriteriaRhs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WildcardStepBudget {
    used: u64,
    limit: u64,
}

impl WildcardStepBudget {
    pub const fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }

    fn charge(&mut self) -> Result<(), ErrorKind> {
        self.used = self.used.checked_add(1).ok_or(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ))?;
        if self.used > self.limit {
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ))
        } else {
            Ok(())
        }
    }
}

pub fn parse_criteria(value: &Value) -> Result<Criteria, ErrorKind> {
    match value {
        Value::Error(kind) if kind.is_engine_issue() => Err(*kind),
        Value::Error(kind) => Ok(Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Error(*kind),
        }),
        Value::Number(number) => Ok(Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Number(*number),
        }),
        Value::Logical(logical) => Ok(Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Logical(*logical),
        }),
        Value::Blank => Ok(Criteria {
            op: CriteriaOp::Eq,
            rhs: CriteriaRhs::Blank,
        }),
        Value::Text(text) => {
            let (op, rest) = if let Some(rest) = text.strip_prefix("<>") {
                (CriteriaOp::Ne, rest)
            } else if let Some(rest) = text.strip_prefix("<=") {
                (CriteriaOp::Le, rest)
            } else if let Some(rest) = text.strip_prefix(">=") {
                (CriteriaOp::Ge, rest)
            } else if let Some(rest) = text.strip_prefix('<') {
                (CriteriaOp::Lt, rest)
            } else if let Some(rest) = text.strip_prefix('>') {
                (CriteriaOp::Gt, rest)
            } else if let Some(rest) = text.strip_prefix('=') {
                (CriteriaOp::Eq, rest)
            } else {
                (CriteriaOp::Eq, text.as_str())
            };
            let rhs = if rest.is_empty() {
                CriteriaRhs::Blank
            } else if let Ok(number) = rest.trim().parse::<f64>() {
                CriteriaRhs::Number(number)
            } else if rest.eq_ignore_ascii_case("TRUE") {
                CriteriaRhs::Logical(true)
            } else if rest.eq_ignore_ascii_case("FALSE") {
                CriteriaRhs::Logical(false)
            } else {
                CriteriaRhs::Text(rest.to_owned())
            };
            Ok(Criteria { op, rhs })
        }
    }
}

impl Criteria {
    pub fn matches(
        &self,
        cell: &Value,
        budget: &mut WildcardStepBudget,
    ) -> Result<bool, ErrorKind> {
        match self.op {
            CriteriaOp::Eq => self.eq_matches(cell, budget),
            CriteriaOp::Ne => Ok(!self.eq_matches(cell, budget)?),
            CriteriaOp::Lt => self.ord_matches(cell, |ordering| ordering == Ordering::Less),
            CriteriaOp::Le => self.ord_matches(cell, |ordering| ordering != Ordering::Greater),
            CriteriaOp::Gt => self.ord_matches(cell, |ordering| ordering == Ordering::Greater),
            CriteriaOp::Ge => self.ord_matches(cell, |ordering| ordering != Ordering::Less),
        }
    }

    fn eq_matches(&self, cell: &Value, budget: &mut WildcardStepBudget) -> Result<bool, ErrorKind> {
        match &self.rhs {
            CriteriaRhs::Blank => Ok(cell.is_blank_like()),
            CriteriaRhs::Number(expected) => Ok(match cell {
                Value::Number(actual) => actual == expected,
                Value::Text(text) => text
                    .trim()
                    .parse::<f64>()
                    .is_ok_and(|actual| actual == *expected),
                _ => false,
            }),
            CriteriaRhs::Text(pattern) => match cell {
                Value::Text(text) if !text.is_empty() => wildcard_match(pattern, text, budget),
                _ => Ok(false),
            },
            CriteriaRhs::Logical(expected) => {
                Ok(matches!(cell, Value::Logical(actual) if actual == expected))
            }
            CriteriaRhs::Error(expected) => {
                Ok(matches!(cell, Value::Error(actual) if actual == expected))
            }
        }
    }

    fn ord_matches(
        &self,
        cell: &Value,
        accept: impl Fn(Ordering) -> bool,
    ) -> Result<bool, ErrorKind> {
        Ok(match &self.rhs {
            CriteriaRhs::Number(expected) => {
                let actual = match cell {
                    Value::Number(number) => Some(*number),
                    Value::Text(text) => text.trim().parse::<f64>().ok(),
                    _ => None,
                };
                actual
                    .and_then(|actual| actual.partial_cmp(expected))
                    .is_some_and(accept)
            }
            CriteriaRhs::Text(expected) => match cell {
                Value::Text(text) if !text.is_empty() => {
                    accept(compare_text_case_insensitive(text, expected))
                }
                _ => false,
            },
            CriteriaRhs::Logical(expected) => {
                matches!(cell, Value::Logical(actual) if accept(actual.cmp(expected)))
            }
            CriteriaRhs::Blank => false,
            CriteriaRhs::Error(_) => false,
        })
    }

    pub fn matches_blank(&self) -> bool {
        let mut budget = WildcardStepBudget::new(u64::MAX);
        self.matches(&Value::Blank, &mut budget)
            .expect("blank matching does not consume wildcard steps")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternToken {
    Literal(char),
    AnyOne,
    AnySequence,
}

fn compile_pattern(pattern: &str) -> Vec<PatternToken> {
    let mut tokens = Vec::new();
    let mut characters = pattern.chars().flat_map(char::to_lowercase).peekable();
    while let Some(character) = characters.next() {
        match character {
            '~' if characters
                .peek()
                .is_some_and(|next| matches!(next, '?' | '*' | '~')) =>
            {
                tokens.push(PatternToken::Literal(
                    characters.next().expect("peeked wildcard escape"),
                ));
            }
            '~' => tokens.push(PatternToken::Literal('~')),
            '?' => tokens.push(PatternToken::AnyOne),
            '*' => tokens.push(PatternToken::AnySequence),
            _ => tokens.push(PatternToken::Literal(character)),
        }
    }
    tokens
}

pub fn wildcard_match(
    pattern: &str,
    text: &str,
    budget: &mut WildcardStepBudget,
) -> Result<bool, ErrorKind> {
    wildcard_match_with_step(pattern, text, budget, || Ok(()))
}

pub fn wildcard_match_with_step(
    pattern: &str,
    text: &str,
    budget: &mut WildcardStepBudget,
    mut on_step: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<bool, ErrorKind> {
    let tokens = compile_pattern(pattern);
    let characters: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut token_index = 0_usize;
    let mut char_index = 0_usize;
    let mut star_token: Option<usize> = None;
    let mut star_char = 0_usize;
    while char_index < characters.len() {
        budget.charge()?;
        on_step()?;
        let matched = match tokens.get(token_index) {
            Some(PatternToken::AnyOne) => true,
            Some(PatternToken::Literal(literal)) => *literal == characters[char_index],
            _ => false,
        };
        if matched {
            token_index += 1;
            char_index += 1;
        } else if tokens.get(token_index) == Some(&PatternToken::AnySequence) {
            star_token = Some(token_index);
            star_char = char_index;
            token_index += 1;
        } else if let Some(star) = star_token {
            token_index = star + 1;
            star_char += 1;
            char_index = star_char;
        } else {
            return Ok(false);
        }
    }
    while tokens.get(token_index) == Some(&PatternToken::AnySequence) {
        budget.charge()?;
        on_step()?;
        token_index += 1;
    }
    Ok(token_index == tokens.len())
}

#[cfg(test)]
mod tests {
    use super::{WildcardStepBudget, wildcard_match};
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    #[test]
    fn wildcard_matching_is_case_insensitive_and_bounded() {
        let mut ample = WildcardStepBudget::new(100);
        assert_eq!(wildcard_match("a*?~*", "Axx*", &mut ample), Ok(true));
        assert_eq!(wildcard_match("~a", "~a", &mut ample), Ok(true));
        assert_eq!(wildcard_match("a~", "a~", &mut ample), Ok(true));
        assert_eq!(wildcard_match("a~~", "a~", &mut ample), Ok(true));

        let mut exhausted = WildcardStepBudget::new(2);
        assert_eq!(
            wildcard_match("*z", "aaaa", &mut exhausted),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
    }
}
