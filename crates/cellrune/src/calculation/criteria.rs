use std::cmp::Ordering;

use super::coerce::compare_text_case_insensitive;
use super::excel_digits::approximately_equal;
use super::numeric_text::NumericText;
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
    /// Whether a number criterion treats numbers that differ only past their fifteenth significant
    /// digit as equal, as the comparison operators do.
    approximate: bool,
    /// How a text cell reads as the number an equality criterion compares it to; `None` for a
    /// lookup's equality, under which a number equals number cells only.
    numeric_text: Option<NumericText>,
}

/// Orders a cell's number against a criterion's number. With `approximate`, numbers that differ
/// only past their fifteenth significant digit are equal. `None` when either is not a number.
pub(in crate::calculation) fn criterion_number_ordering(
    actual: f64,
    expected: f64,
    approximate: bool,
) -> Option<Ordering> {
    if approximate && approximately_equal(actual, expected) {
        return Some(Ordering::Equal);
    }
    actual.partial_cmp(&expected)
}

/// Compiles a COUNTIF-style criterion. `approximate` makes a number criterion compare as the
/// comparison operators do; see [`criterion_number_ordering`]. Text after the operator, and text
/// cells an equality compares to a number, read as numbers as `numeric_text` reads them.
pub fn compile_criteria_with_work(
    value: &Value,
    approximate: bool,
    numeric_text: NumericText,
    on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<CompiledCriteria, ErrorKind> {
    let (op, rhs) = compile_operator_and_value(value, numeric_text, on_work)?;
    Ok(CompiledCriteria {
        op,
        rhs,
        approximate,
        numeric_text: Some(numeric_text),
    })
}

fn compile_operator_and_value(
    value: &Value,
    numeric_text: NumericText,
    mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(CriteriaOp, CompiledCriteriaRhs), ErrorKind> {
    match value {
        Value::Error(kind) if kind.is_engine_issue() => Err(*kind),
        Value::Error(kind) => Ok((CriteriaOp::Eq, CompiledCriteriaRhs::Error(*kind))),
        Value::Number(number) => Ok((CriteriaOp::Eq, CompiledCriteriaRhs::Number(*number))),
        Value::Logical(logical) => Ok((CriteriaOp::Eq, CompiledCriteriaRhs::Logical(*logical))),
        Value::Blank => Ok((CriteriaOp::Eq, CompiledCriteriaRhs::Blank)),
        Value::Text(text) => {
            charge_preprocessing(&mut on_work)?;
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
            } else if let Some(number) = numeric_text.number(rest) {
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
            Ok((op, rhs))
        }
    }
}

impl CompiledCriteria {
    /// The exact equality of a `MATCH` lookup: text matches text case-insensitively and with
    /// wildcards, and a number matches number cells only (`MATCH(5,{"5";5},0)` is 2, Mac Excel
    /// 16.114).
    pub fn exact_equality_with_work(
        value: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<Option<Self>, ErrorKind> {
        let rhs = match value {
            Value::Number(number) => CompiledCriteriaRhs::Number(*number),
            Value::Logical(logical) => CompiledCriteriaRhs::Logical(*logical),
            Value::Text(text) => {
                charge_preprocessing(&mut on_work)?;
                CompiledCriteriaRhs::EqualityText(CompiledWildcardPattern::compile_precharged(text))
            }
            Value::Blank | Value::Error(_) => return Ok(None),
        };
        Ok(Some(Self {
            op: CriteriaOp::Eq,
            rhs,
            approximate: false,
            numeric_text: None,
        }))
    }

    pub fn matches_with_work(
        &self,
        cell: &Value,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        match self.op {
            CriteriaOp::Eq => self.eq_matches(cell, &mut on_work, true),
            // A text cell is unequal to a number even when its text reads as that number: `"<>5"`
            // counts a cell holding the text `5` (Mac Excel 16.114).
            CriteriaOp::Ne => Ok(!self.eq_matches(cell, &mut on_work, false)?),
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

    /// Whether `cell` equals the criterion's value; with `text_numbers`, a text cell equals a number
    /// its text reads as.
    fn eq_matches(
        &self,
        cell: &Value,
        on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
        text_numbers: bool,
    ) -> Result<bool, ErrorKind> {
        match &self.rhs {
            CompiledCriteriaRhs::Blank => Ok(cell.is_blank_like()),
            CompiledCriteriaRhs::Number(expected) => {
                let actual = match cell {
                    Value::Number(number) => Some(*number),
                    Value::Text(text) => match self.numeric_text {
                        Some(numeric_text) if text_numbers => {
                            charge_preprocessing(on_work)?;
                            numeric_text.number(text)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                Ok(actual.is_some_and(|actual| {
                    criterion_number_ordering(actual, *expected, self.approximate)
                        == Some(Ordering::Equal)
                }))
            }
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
            // A number orders only number cells: `">4"` does not count the text `5` (Mac Excel
            // 16.114).
            CompiledCriteriaRhs::Number(expected) => Ok(match cell {
                Value::Number(actual) => {
                    criterion_number_ordering(*actual, *expected, self.approximate)
                        .is_some_and(accept)
                }
                _ => false,
            }),
            CompiledCriteriaRhs::OrderedText(expected) => match cell {
                Value::Text(text) if !text.is_empty() => {
                    charge_text_comparison(on_work)?;
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

    /// The one value this criterion matches cells equal to, or `None` for a comparison, an
    /// inequality, a wildcard pattern, a blank, or an error.
    ///
    /// A number matches number cells equal to it and text cells whose text reads as it.
    /// A text matches nonempty text cells whose case-folded characters equal it. A logical matches
    /// the equal logical.
    pub(in crate::calculation) fn exact_value(&self) -> Option<ExactCriterion> {
        if self.op != CriteriaOp::Eq {
            return None;
        }
        match &self.rhs {
            CompiledCriteriaRhs::Number(number) => Some(ExactCriterion::Number {
                number: *number,
                approximate: self.approximate,
            }),
            CompiledCriteriaRhs::EqualityText(pattern) => {
                pattern.literal().map(ExactCriterion::Text)
            }
            CompiledCriteriaRhs::Logical(logical) => Some(ExactCriterion::Logical(*logical)),
            CompiledCriteriaRhs::OrderedText(_)
            | CompiledCriteriaRhs::Blank
            | CompiledCriteriaRhs::Error(_) => None,
        }
    }

    pub fn matches_blank(&self) -> bool {
        self.matches_with_work(&Value::Blank, |_| Ok(()))
            .expect("blank matching does not consume wildcard steps")
    }
}

/// The value an exact-equality criterion matches; see [`CompiledCriteria::exact_value`].
#[derive(Debug, Clone, PartialEq)]
pub(in crate::calculation) enum ExactCriterion {
    /// A number; with `approximate`, cells whose numbers differ from it only past their fifteenth
    /// significant digit match too.
    Number {
        number: f64,
        approximate: bool,
    },
    /// The case-folded characters a text cell must hold.
    Text(String),
    Logical(bool),
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
    has_wildcard: bool,
}

impl CompiledWildcardPattern {
    pub fn compile_with_work(
        pattern: &str,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        charge_preprocessing(&mut on_work)?;
        Ok(Self::compile_precharged(pattern))
    }

    pub(in crate::calculation) fn compile_precharged(pattern: &str) -> Self {
        let tokens = compile_pattern(pattern);
        let has_wildcard = tokens
            .iter()
            .any(|token| !matches!(token, PatternToken::Literal(_)));
        Self {
            tokens,
            has_wildcard,
        }
    }

    pub(in crate::calculation) fn push_any_sequence(&mut self) {
        if self.tokens.last() != Some(&PatternToken::AnySequence) {
            self.tokens.push(PatternToken::AnySequence);
        }
        self.has_wildcard = true;
    }

    /// The case-folded characters a text must equal, for a pattern without wildcards.
    fn literal(&self) -> Option<String> {
        self.tokens
            .iter()
            .map(|token| match token {
                PatternToken::Literal(character) => Some(*character),
                PatternToken::AnyOne | PatternToken::AnySequence => None,
            })
            .collect()
    }

    pub fn matches_with_work(
        &self,
        text: &str,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        charge_preprocessing(&mut on_work)?;
        // ASCII text lowercases one byte to one character, so it is matched in place; other text
        // is case-folded into characters first. Both take the same steps.
        if text.is_ascii() {
            let bytes = text.as_bytes();
            return self.match_folded(
                bytes.len(),
                |index| bytes[index].to_ascii_lowercase() as char,
                on_work,
            );
        }
        let characters: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
        self.match_folded(characters.len(), |index| characters[index], on_work)
    }

    /// Matches the case-folded text of `length` characters that `character` reads by index.
    fn match_folded(
        &self,
        length: usize,
        character: impl Fn(usize) -> char,
        mut on_work: impl FnMut(u64) -> Result<(), ErrorKind>,
    ) -> Result<bool, ErrorKind> {
        let mut token_index = 0_usize;
        let mut char_index = 0_usize;
        let mut star_token: Option<usize> = None;
        let mut star_char = 0_usize;
        while char_index < length {
            // A pattern without wildcards is one case-insensitive comparison, charged with the
            // preprocessing above. Only wildcard state transitions are charged per step.
            if self.has_wildcard {
                on_work(1)?;
            }
            let matched = match self.tokens.get(token_index) {
                Some(PatternToken::AnyOne) => true,
                Some(PatternToken::Literal(literal)) => *literal == character(char_index),
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

/// Preprocessing one input costs one work unit, as reading one cell does on the other lookup
/// paths. The charge happens before allocation or Unicode case folding and is followed by
/// separate charges for wildcard state transitions.
fn charge_preprocessing(
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    on_work(1)
}

pub fn charge_value_comparison_work(
    left: &Value,
    right: &Value,
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    match (left, right) {
        (Value::Text(_), Value::Text(_) | Value::Blank) | (Value::Blank, Value::Text(_)) => {
            charge_text_comparison(on_work)
        }
        _ => Ok(()),
    }
}

/// A case-insensitive text comparison costs one work unit regardless of the text lengths.
pub(in crate::calculation) fn charge_text_comparison(
    on_work: &mut impl FnMut(u64) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    on_work(1)
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
    use crate::DateSystem;
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::locale::Locale;
    use crate::calculation::numeric_text::NumericText;
    use crate::calculation::session::CancellationToken;
    use crate::calculation::value::ErrorKind;
    use crate::calculation::value::Value;

    const US: NumericText = NumericText::new(Locale::EnUs, DateSystem::Excel1900);

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
    fn ascii_text_matches_in_the_steps_of_folded_characters() {
        for pattern in ["a*b?c", "*", "abc", "~*x*", "?*?", "A*A*A*B"] {
            let compiled = compiled_pattern(pattern);
            for text in ["", "ABC", "aXbYc", "aaaaaaaaab", "*x-y", "abc~", "AbAbAbB"] {
                let mut folded_work = 0_u64;
                let characters: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
                let folded = compiled.match_folded(
                    characters.len(),
                    |index| characters[index],
                    |units| {
                        folded_work += units;
                        Ok(())
                    },
                );
                let mut work = 0_u64;
                let matched = compiled.matches_with_work(text, |units| {
                    work += units;
                    Ok(())
                });
                assert_eq!(matched, folded, "{pattern} on {text}");
                assert_eq!(work, folded_work + 1, "{pattern} on {text}");
            }
        }
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
        let criterion =
            compile_criteria_with_work(&Value::Text("a*".to_owned()), false, US, |_| Ok(()))
                .expect("valid criterion");
        assert!(matches!(
            criterion.rhs,
            CompiledCriteriaRhs::EqualityText(_)
        ));
        let ordered =
            compile_criteria_with_work(&Value::Text(">a*".to_owned()), false, US, |_| Ok(()))
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
        let countif =
            compile_criteria_with_work(&Value::Text("Dav".to_owned()), false, US, |_| Ok(()))
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
                charge(&mut pattern_work, 0, units)
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
        let numeric = compile_criteria_with_work(&Value::Number(12.0), false, US, |_| Ok(()))
            .expect("numeric criterion");
        let mut numeric_work = 0_u64;
        assert_eq!(
            numeric.matches_with_work(&Value::Text(" 12 ".to_owned()), |units| {
                numeric_work += units;
                Ok(())
            }),
            Ok(true)
        );
        assert_eq!(numeric_work, 1);

        let ordered =
            compile_criteria_with_work(&Value::Text(">alpha".to_owned()), false, US, |_| Ok(()))
                .expect("ordered criterion");
        let mut ordered_work = 0_u64;
        assert_eq!(
            ordered.matches_with_work(&Value::Text("Beta".to_owned()), |units| {
                ordered_work += units;
                Ok(())
            }),
            Ok(true)
        );
        assert_eq!(ordered_work, 1);

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
        assert_eq!(comparison_work, 1);
    }
}
