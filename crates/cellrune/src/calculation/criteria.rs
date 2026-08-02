use std::cmp::Ordering;

use super::coerce::compare_text_case_insensitive;
use super::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CriteriaOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
enum CompiledCriteriaRhs {
    Number(f64),
    EqualityText(CompiledWildcardPattern),
    OrderedText(String),
    Logical(bool),
    Blank,
    Error(ErrorKind),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledCriteria {
    op: CriteriaOp,
    rhs: CompiledCriteriaRhs,
}

pub fn compile_criteria_with_work(
    value: &Value,
    mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<CompiledCriteria, ErrorKind> {
    match value {
        Value::Error(kind) if kind.is_engine_issue() => Err(*kind),
        Value::Error(kind) => Ok(CompiledCriteria {
            op: CriteriaOp::Eq,
            rhs: CompiledCriteriaRhs::Error(*kind),
        }),
        Value::Number(number) => Ok(CompiledCriteria {
            op: CriteriaOp::Eq,
            rhs: CompiledCriteriaRhs::Number(*number),
        }),
        Value::Logical(logical) => Ok(CompiledCriteria {
            op: CriteriaOp::Eq,
            rhs: CompiledCriteriaRhs::Logical(*logical),
        }),
        Value::Blank => Ok(CompiledCriteria {
            op: CriteriaOp::Eq,
            rhs: CompiledCriteriaRhs::Blank,
        }),
        Value::Text(text) => {
            charge_preprocessing(text, &mut on_work)?;
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
                CompiledCriteriaRhs::Blank
            } else if let Ok(number) = rest.trim().parse::<f64>() {
                CompiledCriteriaRhs::Number(number)
            } else if rest.eq_ignore_ascii_case("TRUE") {
                CompiledCriteriaRhs::Logical(true)
            } else if rest.eq_ignore_ascii_case("FALSE") {
                CompiledCriteriaRhs::Logical(false)
            } else if matches!(op, CriteriaOp::Eq | CriteriaOp::Ne) {
                CompiledCriteriaRhs::EqualityText(CompiledWildcardPattern::compile_precharged(rest))
            } else {
                CompiledCriteriaRhs::OrderedText(rest.to_owned())
            };
            Ok(CompiledCriteria { op, rhs })
        }
    }
}

impl CompiledCriteria {
    pub fn exact_equality_with_work(
        value: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<Option<Self>, ErrorKind> {
        let rhs = match value {
            Value::Number(number) => CompiledCriteriaRhs::Number(*number),
            Value::Logical(logical) => CompiledCriteriaRhs::Logical(*logical),
            Value::Text(text) => {
                charge_preprocessing(text, &mut on_work)?;
                CompiledCriteriaRhs::EqualityText(CompiledWildcardPattern::compile_precharged(text))
            }
            Value::Blank | Value::Error(_) => return Ok(None),
        };
        Ok(Some(Self {
            op: CriteriaOp::Eq,
            rhs,
        }))
    }

    pub fn matches_with_work(
        &self,
        cell: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        match self.op {
            CriteriaOp::Eq => self.eq_matches(cell, &mut on_work),
            CriteriaOp::Ne => Ok(!self.eq_matches(cell, &mut on_work)?),
            CriteriaOp::Lt => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering == Ordering::Less)
            }
            CriteriaOp::Le => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering != Ordering::Greater)
            }
            CriteriaOp::Gt => {
                self.ord_matches(cell, &mut on_work, |ordering| ordering == Ordering::Greater)
            }
            CriteriaOp::Ge => {
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
            CompiledCriteriaRhs::Blank => Ok(cell.is_blank_like()),
            CompiledCriteriaRhs::Number(expected) => match cell {
                Value::Number(actual) => Ok(actual == expected),
                Value::Text(text) => {
                    charge_preprocessing(text, on_work)?;
                    Ok(text
                        .trim()
                        .parse::<f64>()
                        .is_ok_and(|actual| actual == *expected))
                }
                _ => Ok(false),
            },
            CompiledCriteriaRhs::EqualityText(pattern) => match cell {
                Value::Text(text) if !text.is_empty() => pattern.matches_with_work(text, on_work),
                _ => Ok(false),
            },
            CompiledCriteriaRhs::OrderedText(_) => {
                unreachable!("ordered text is constructed only for ordering operators")
            }
            CompiledCriteriaRhs::Logical(expected) => {
                Ok(matches!(cell, Value::Logical(actual) if actual == expected))
            }
            CompiledCriteriaRhs::Error(expected) => {
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
            CompiledCriteriaRhs::Number(expected) => {
                let actual = match cell {
                    Value::Number(number) => Some(*number),
                    Value::Text(text) => {
                        charge_preprocessing(text, on_work)?;
                        text.trim().parse::<f64>().ok()
                    }
                    _ => None,
                };
                Ok(actual
                    .and_then(|actual| actual.partial_cmp(expected))
                    .is_some_and(accept))
            }
            CompiledCriteriaRhs::OrderedText(expected) => match cell {
                Value::Text(text) if !text.is_empty() => {
                    charge_text_comparison_work(text, expected, on_work)?;
                    Ok(accept(compare_text_case_insensitive(text, expected)))
                }
                _ => Ok(false),
            },
            CompiledCriteriaRhs::EqualityText(_) => {
                unreachable!("equality text is constructed only for equality operators")
            }
            CompiledCriteriaRhs::Logical(expected) => Ok(matches!(
                cell,
                Value::Logical(actual) if accept(actual.cmp(expected))
            )),
            CompiledCriteriaRhs::Blank | CompiledCriteriaRhs::Error(_) => Ok(false),
        }
    }

    pub fn matches_blank(&self) -> bool {
        self.matches_with_work(&Value::Blank, |_| Ok(()))
            .expect("blank matching does not consume wildcard steps")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternToken {
    Literal(char),
    AnyOne,
    AnySequence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledWildcardPattern {
    tokens: Vec<PatternToken>,
}

impl CompiledWildcardPattern {
    pub fn compile_with_work(
        pattern: &str,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        charge_preprocessing(pattern, &mut on_work)?;
        Ok(Self::compile_precharged(pattern))
    }

    pub(in crate::calculation) fn compile_precharged(pattern: &str) -> Self {
        Self {
            tokens: compile_pattern(pattern),
        }
    }

    pub(in crate::calculation) fn push_any_sequence(&mut self) {
        if self.tokens.last() != Some(&PatternToken::AnySequence) {
            self.tokens.push(PatternToken::AnySequence);
        }
    }

    pub fn matches_with_work(
        &self,
        text: &str,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        charge_preprocessing(text, &mut on_work)?;
        let characters: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
        let mut token_index = 0_usize;
        let mut char_index = 0_usize;
        let mut star_token: Option<usize> = None;
        let mut star_char = 0_usize;
        while char_index < characters.len() {
            on_work(1)?;
            let matched = match self.tokens.get(token_index) {
                Some(PatternToken::AnyOne) => true,
                Some(PatternToken::Literal(literal)) => *literal == characters[char_index],
                _ => false,
            };
            if matched {
                token_index += 1;
                char_index += 1;
            } else if self.tokens.get(token_index) == Some(&PatternToken::AnySequence) {
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
        while self.tokens.get(token_index) == Some(&PatternToken::AnySequence) {
            on_work(1)?;
            token_index += 1;
        }
        Ok(token_index == self.tokens.len())
    }
}

/// Preprocessing uses one conservative work unit per input UTF-8 byte. The charge happens before
/// allocation or Unicode case folding, covers all linear passes over that input exactly once, and
/// is followed by separate charges for wildcard state transitions.
fn charge_preprocessing(
    input: &str,
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    on_work(input.len() as u64)
}

pub fn charge_value_comparison_work(
    left: &Value,
    right: &Value,
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    match (left, right) {
        (Value::Text(left), Value::Text(right)) => {
            charge_text_comparison_work(left, right, on_work)
        }
        (Value::Text(text), Value::Blank) | (Value::Blank, Value::Text(text)) => {
            charge_preprocessing(text, on_work)
        }
        _ => Ok(()),
    }
}

pub(in crate::calculation) fn charge_text_comparison_work(
    left: &str,
    right: &str,
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    let units =
        (left.len() as u64)
            .checked_add(right.len() as u64)
            .ok_or(ErrorKind::ResourceLimit(
                super::limits::CalculationLimitKind::FunctionIterations,
            ))?;
    on_work(units)
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

#[cfg(test)]
mod tests {
    use super::{
        CompiledCriteria, CompiledCriteriaRhs, CompiledWildcardPattern,
        charge_value_comparison_work, compile_criteria_with_work,
    };
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::session::CancellationToken;
    use crate::calculation::value::ErrorKind;
    use crate::calculation::value::Value;

    fn charge(used: &mut u64, limit: u64, units: u64) -> Result<(), ErrorKind> {
        *used = used.checked_add(units).ok_or(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ))?;
        if *used > limit {
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ))
        } else {
            Ok(())
        }
    }

    fn compiled_pattern(pattern: &str) -> CompiledWildcardPattern {
        CompiledWildcardPattern::compile_with_work(pattern, |_| Ok(()))
            .expect("unlimited pattern work")
    }

    #[test]
    fn wildcard_matching_is_case_insensitive_and_bounded() {
        for (pattern, text) in [("a*?~*", "Axx*"), ("~a", "~a"), ("a~", "a~"), ("a~~", "a~")] {
            let compiled = compiled_pattern(pattern);
            assert_eq!(
                compiled.matches_with_work(text, |_| Ok(())),
                Ok(true),
                "pattern={pattern}, text={text}",
            );
        }
    }

    #[test]
    fn compiled_criteria_reuses_one_equality_pattern_and_keeps_ordered_text_raw() {
        let criterion = compile_criteria_with_work(&Value::Text("a*".to_owned()), |_| Ok(()))
            .expect("valid criterion");
        assert!(matches!(
            criterion.rhs,
            CompiledCriteriaRhs::EqualityText(_)
        ));
        let ordered = compile_criteria_with_work(&Value::Text(">a*".to_owned()), |_| Ok(()))
            .expect("valid ordered criterion");
        assert_eq!(
            ordered.rhs,
            CompiledCriteriaRhs::OrderedText("a*".to_owned())
        );

        let mut charged = 0_u64;
        for value in ["alpha", "amber"] {
            assert_eq!(
                criterion.matches_with_work(&Value::Text(value.to_owned()), |units| {
                    charged += units;
                    Ok(())
                },),
                Ok(true)
            );
        }
        assert!(charged >= "alpha".len() as u64 + "amber".len() as u64);

        let exact =
            CompiledCriteria::exact_equality_with_work(&Value::Text("a*".to_owned()), |_| Ok(()))
                .expect("within budget")
                .expect("text criterion");
        assert_eq!(
            exact.matches_with_work(&Value::Text("alpha".to_owned()), |_| Ok(())),
            Ok(true)
        );
    }

    #[test]
    fn bare_text_keeps_countif_exact_semantics() {
        let countif = compile_criteria_with_work(&Value::Text("Dav".to_owned()), |_| Ok(()))
            .expect("valid COUNTIF criterion");
        assert_eq!(
            countif.matches_with_work(&Value::Text("David".to_owned()), |_| Ok(())),
            Ok(false)
        );
    }

    #[test]
    fn preprocessing_is_precharged_and_observes_the_real_cancellation_token() {
        let long_pattern = "a".repeat(128);
        let mut pattern_work = 0_u64;
        assert_eq!(
            CompiledWildcardPattern::compile_with_work(&long_pattern, |units| {
                charge(&mut pattern_work, 16, units)
            }),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );

        let mut text_work = 0_u64;
        let pattern = compiled_pattern("a*");
        assert_eq!(
            pattern.matches_with_work(&"a".repeat(64), |units| {
                charge(&mut text_work, 32, units)
            }),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );

        let token = CancellationToken::new();
        let pattern = compiled_pattern("a*");
        let mut polls = 0_u64;
        assert_eq!(
            pattern.matches_with_work(&"a".repeat(128), |_| {
                polls += 1;
                token.cancel();
                if token.is_cancelled() {
                    Err(ErrorKind::ResourceLimit(
                        CalculationLimitKind::FunctionIterations,
                    ))
                } else {
                    Ok(())
                }
            },),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
        assert_eq!(polls, 1, "cancellation stops before lowercase allocation");
    }

    #[test]
    fn numeric_and_ordered_text_work_is_charged_before_comparison() {
        let numeric = compile_criteria_with_work(&Value::Number(12.0), |_| Ok(()))
            .expect("numeric criterion");
        let mut numeric_work = 0_u64;
        assert_eq!(
            numeric.matches_with_work(&Value::Text(" 12 ".to_owned()), |units| {
                numeric_work += units;
                Ok(())
            }),
            Ok(true)
        );
        assert_eq!(numeric_work, 4);

        let ordered = compile_criteria_with_work(&Value::Text(">alpha".to_owned()), |_| Ok(()))
            .expect("ordered criterion");
        let mut ordered_work = 0_u64;
        assert_eq!(
            ordered.matches_with_work(&Value::Text("Beta".to_owned()), |units| {
                ordered_work += units;
                Ok(())
            }),
            Ok(true)
        );
        assert_eq!(ordered_work, ("Beta".len() + "alpha".len()) as u64);

        let mut comparison_work = 0_u64;
        charge_value_comparison_work(
            &Value::Text("left".to_owned()),
            &Value::Text("right".to_owned()),
            &mut |units| {
                comparison_work += units;
                Ok(())
            },
        )
        .expect("comparison work");
        assert_eq!(comparison_work, 9);
    }
}
