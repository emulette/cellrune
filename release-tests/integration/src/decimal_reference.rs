//! An exact decimal reference for the near-zero arithmetic question.
//!
//! The compatibility mode added in 0.1.3 has to decide one thing: when a sum or difference
//! cancels, is the residue an artefact of writing decimal literals in binary, or a number the
//! workbook author meant? Excel is not the authority on that — arithmetic is. A chain of `+` and
//! `-` over decimal literals has an exact value, and this module computes it.
//!
//! Decimal literals are scaled to integers, so addition and subtraction are exact and the answer
//! is a fact rather than a measurement. That makes this a reference implementation, not a golden
//! file: it calculates the expected result instead of recording whatever the engine produced.
//!
//! Deliberately no new dependency. `i128` at a fixed scale covers every literal these tests use
//! with room to spare, and reaching for a rational-arithmetic crate would put a floor and an
//! advisory surface under a test-only helper.

/// Number of decimal places the fixed-point representation keeps.
///
/// Eighteen places leaves `i128` around twenty digits of integer part, far beyond anything a
/// cancellation case needs, and every literal in these tests has at most a handful of places.
const SCALE: u32 = 18;

/// A decimal number held as an exact scaled integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal(i128);

impl Decimal {
    /// Parses a decimal literal exactly.
    ///
    /// Accepts an optional sign, digits, and an optional fractional part. Exponent notation is
    /// rejected rather than approximated: a case that needs it should be written out.
    ///
    /// # Panics
    ///
    /// Panics on input this fixed-point form cannot hold exactly, which in a test means the case
    /// was written wrong rather than that the engine failed.
    pub fn parse(literal: &str) -> Self {
        let (negative, digits) = match literal.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, literal.strip_prefix('+').unwrap_or(literal)),
        };
        let (integer_part, fraction_part) = match digits.split_once('.') {
            Some((integer, fraction)) => (integer, fraction),
            None => (digits, ""),
        };
        assert!(
            !integer_part.is_empty() || !fraction_part.is_empty(),
            "decimal literal has no digits: {literal}"
        );
        assert!(
            fraction_part.len() <= SCALE as usize,
            "decimal literal needs more than {SCALE} places: {literal}"
        );
        assert!(
            integer_part.bytes().all(|byte| byte.is_ascii_digit())
                && fraction_part.bytes().all(|byte| byte.is_ascii_digit()),
            "not a plain decimal literal: {literal}"
        );

        let mut value: i128 = 0;
        for byte in integer_part.bytes().chain(fraction_part.bytes()) {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(i128::from(byte - b'0')))
                .unwrap_or_else(|| panic!("decimal literal does not fit: {literal}"));
        }
        let remaining_places = SCALE - u32::try_from(fraction_part.len()).expect("checked above");
        value = value
            .checked_mul(10_i128.pow(remaining_places))
            .unwrap_or_else(|| panic!("decimal literal does not fit: {literal}"));
        Self(if negative { -value } else { value })
    }

    /// Returns whether the exact value is zero.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns the nearest `f64`, for comparing against an engine result that did not cancel.
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / 10_f64.powi(SCALE as i32)
    }
}

/// One term of a left-to-right chain of additions and subtractions.
#[derive(Debug, Clone, Copy)]
pub enum Term {
    /// Added to the running total.
    Plus(Decimal),
    /// Subtracted from the running total.
    Minus(Decimal),
}

/// Evaluates a chain of additions and subtractions exactly.
///
/// Left to right, matching how the engine associates `a + b - c`.
pub fn evaluate(first: Decimal, rest: &[Term]) -> Decimal {
    let mut total = first.0;
    for term in rest {
        total = match term {
            Term::Plus(value) => total.checked_add(value.0),
            Term::Minus(value) => total.checked_sub(value.0),
        }
        .expect("exact chain overflowed i128");
    }
    Decimal(total)
}

/// Parses a formula body of the form `a + b - c` into an exact chain.
///
/// Only `+` and `-` between plain decimal literals, which is the whole grammar the near-zero
/// question needs. Anything else panics rather than being silently reinterpreted.
///
/// # Panics
///
/// Panics when the body is not such a chain.
pub fn parse_chain(body: &str) -> (Decimal, Vec<Term>) {
    let mut tokens = body.split_whitespace();
    let first = Decimal::parse(tokens.next().expect("chain has at least one term"));
    let mut rest = Vec::new();
    while let Some(operator) = tokens.next() {
        let literal = tokens
            .next()
            .unwrap_or_else(|| panic!("operator {operator} has no right operand in: {body}"));
        let value = Decimal::parse(literal);
        rest.push(match operator {
            "+" => Term::Plus(value),
            "-" => Term::Minus(value),
            other => panic!("unsupported operator {other} in exact chain: {body}"),
        });
    }
    (first, rest)
}

#[cfg(test)]
mod tests {
    use super::{Decimal, evaluate, parse_chain};

    #[test]
    fn literals_that_binary_cannot_hold_are_exact_here() {
        // The three that IEEE-754 gets wrong, and that the mode exists to correct.
        for body in ["0.1 + 0.2 - 0.3", "0.5 - 0.4 - 0.1", "1.1 - 1.0 - 0.1"] {
            let (first, rest) = parse_chain(body);
            assert!(evaluate(first, &rest).is_zero(), "{body}");
        }
    }

    #[test]
    fn a_difference_the_author_meant_is_not_zero() {
        let (first, rest) = parse_chain("1.1 - 1.0");
        let result = evaluate(first, &rest);
        assert!(!result.is_zero());
        assert!((result.to_f64() - 0.1).abs() < 1e-18);
    }

    #[test]
    fn parsing_covers_signs_and_missing_parts() {
        assert!(Decimal::parse("-0.0").is_zero());
        assert_eq!(Decimal::parse("-2.5").to_f64(), -2.5);
        assert_eq!(Decimal::parse("+3").to_f64(), 3.0);
        assert_eq!(Decimal::parse(".5").to_f64(), 0.5);
        assert_eq!(Decimal::parse("7.").to_f64(), 7.0);
    }
}
