/// Exact base-10 value used to decide whether parsed numeric inputs really cancel.
///
/// The calculated number remains an `f64`; this trace is only a decision oracle for the sums and
/// differences that can cancel — including the products a kernel such as `SUMPRODUCT` forms before
/// summing them. Keeping the coefficient and power of ten separately avoids both a wider near-zero
/// threshold and the false positives that an interval error bound would permit.
///
/// Every operation is checked and returns `None` on overflow, so a trace that cannot be represented
/// exactly stops the chain instead of asserting a cancellation it did not prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecimalTrace {
    coefficient: i128,
    exponent: i32,
}

impl DecimalTrace {
    pub(in crate::calculation) const ZERO: Self = Self {
        coefficient: 0,
        exponent: 0,
    };
    pub(in crate::calculation) const ONE: Self = Self {
        coefficient: 1,
        exponent: 0,
    };

    /// Parses the exact source spelling of one finite decimal literal.
    pub(in crate::calculation) fn from_literal(literal: &str) -> Option<Self> {
        let (negative, unsigned) = match literal.as_bytes().first() {
            Some(b'-') => (true, &literal[1..]),
            Some(b'+') => (false, &literal[1..]),
            _ => (false, literal),
        };
        let (mantissa, explicit_exponent) = match unsigned
            .split_once('e')
            .or_else(|| unsigned.split_once('E'))
        {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
            None => (unsigned, 0),
        };
        let (integer, fraction) = mantissa
            .split_once('.')
            .map_or((mantissa, ""), |(integer, fraction)| (integer, fraction));
        if integer.is_empty() && fraction.is_empty() {
            return None;
        }
        if !integer.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }

        let mut coefficient = 0_i128;
        for byte in integer.bytes().chain(fraction.bytes()) {
            coefficient = coefficient
                .checked_mul(10)?
                .checked_add(i128::from(byte - b'0'))?;
        }
        if negative {
            coefficient = coefficient.checked_neg()?;
        }
        let fraction_digits = i32::try_from(fraction.len()).ok()?;
        Some(
            Self {
                coefficient,
                exponent: explicit_exponent.checked_sub(fraction_digits)?,
            }
            .normalized(),
        )
    }

    /// Converts an already-typed or XLSX numeric value through its shortest round-trip decimal.
    pub(in crate::calculation) fn from_number(value: f64) -> Option<Self> {
        value
            .is_finite()
            .then(|| value.to_string())
            .and_then(|literal| Self::from_literal(&literal))
    }

    pub(in crate::calculation) fn add(self, right: Self) -> Option<Self> {
        self.combine(right, false)
    }

    pub(in crate::calculation) fn subtract(self, right: Self) -> Option<Self> {
        self.combine(right, true)
    }

    pub(in crate::calculation) fn negate(self) -> Option<Self> {
        Some(Self {
            coefficient: self.coefficient.checked_neg()?,
            exponent: self.exponent,
        })
    }

    pub(in crate::calculation) fn percent(self) -> Option<Self> {
        Some(
            Self {
                coefficient: self.coefficient,
                exponent: self.exponent.checked_sub(2)?,
            }
            .normalized(),
        )
    }

    /// Multiplies two exact decimals, for kernels that form products before summing them.
    ///
    /// A product of two finite decimals is itself a finite decimal, so this stays exact: the
    /// coefficients multiply and the powers of ten add.
    pub(in crate::calculation) fn multiply(self, right: Self) -> Option<Self> {
        Some(
            Self {
                coefficient: self.coefficient.checked_mul(right.coefficient)?,
                exponent: self.exponent.checked_add(right.exponent)?,
            }
            .normalized(),
        )
    }

    pub(in crate::calculation) const fn is_zero(self) -> bool {
        self.coefficient == 0
    }

    fn combine(self, right: Self, subtract: bool) -> Option<Self> {
        let target_exponent = self.exponent.min(right.exponent);
        // Both differences are non-negative by construction but still overflow `i32` when the
        // operands sit at opposite ends of the exponent range, so they fail closed like every
        // other step here rather than panicking on a literal such as `1E-2147483648`.
        let left = scale_coefficient(
            self.coefficient,
            self.exponent.checked_sub(target_exponent)?,
        )?;
        let mut right = scale_coefficient(
            right.coefficient,
            right.exponent.checked_sub(target_exponent)?,
        )?;
        if subtract {
            right = right.checked_neg()?;
        }
        Some(
            Self {
                coefficient: left.checked_add(right)?,
                exponent: target_exponent,
            }
            .normalized(),
        )
    }

    fn normalized(mut self) -> Self {
        if self.coefficient == 0 {
            return Self::ZERO;
        }
        while self.coefficient % 10 == 0 {
            self.coefficient /= 10;
            let Some(exponent) = self.exponent.checked_add(1) else {
                break;
            };
            self.exponent = exponent;
        }
        self
    }
}

/// Exact rational value used when a function transforms decimal inputs before summing them.
///
/// The bounded `i128` representation deliberately fails closed: if an operation does not fit,
/// callers stop tracing and preserve the `f64` result rather than guessing that it is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RationalTrace {
    numerator: i128,
    denominator: i128,
}

impl RationalTrace {
    pub(in crate::calculation) const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    pub(in crate::calculation) fn from_decimal(decimal: DecimalTrace) -> Option<Self> {
        let rational = if decimal.exponent >= 0 {
            Self {
                numerator: scale_coefficient(decimal.coefficient, decimal.exponent)?,
                denominator: 1,
            }
        } else {
            Self {
                numerator: decimal.coefficient,
                denominator: scale_coefficient(1, decimal.exponent.checked_neg()?)?,
            }
        };
        rational.normalized()
    }

    pub(in crate::calculation) fn add(self, right: Self) -> Option<Self> {
        let denominator_gcd = signed_gcd(self.denominator, right.denominator)?;
        let left_scale = right.denominator.checked_div(denominator_gcd)?;
        let right_scale = self.denominator.checked_div(denominator_gcd)?;
        Self {
            numerator: self
                .numerator
                .checked_mul(left_scale)?
                .checked_add(right.numerator.checked_mul(right_scale)?)?,
            denominator: self.denominator.checked_mul(left_scale)?,
        }
        .normalized()
    }

    pub(in crate::calculation) fn divide(self, divisor: Self) -> Option<Self> {
        if divisor.numerator == 0 {
            return None;
        }
        let (numerator, denominator) = if divisor.numerator.is_negative() {
            (
                divisor.denominator.checked_neg()?,
                divisor.numerator.checked_neg()?,
            )
        } else {
            (divisor.denominator, divisor.numerator)
        };
        self.multiply(Self {
            numerator,
            denominator,
        })
    }

    pub(in crate::calculation) fn pow(self, mut exponent: u32) -> Option<Self> {
        let mut result = Self::ONE;
        let mut base = self;
        while exponent > 0 {
            if exponent % 2 == 1 {
                result = result.multiply(base)?;
            }
            exponent /= 2;
            if exponent > 0 {
                base = base.multiply(base)?;
            }
        }
        Some(result)
    }

    pub(in crate::calculation) const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    fn multiply(self, right: Self) -> Option<Self> {
        let left_gcd = signed_gcd(self.numerator, right.denominator)?;
        let right_gcd = signed_gcd(right.numerator, self.denominator)?;
        Self {
            numerator: self
                .numerator
                .checked_div(left_gcd)?
                .checked_mul(right.numerator.checked_div(right_gcd)?)?,
            denominator: self
                .denominator
                .checked_div(right_gcd)?
                .checked_mul(right.denominator.checked_div(left_gcd)?)?,
        }
        .normalized()
    }

    fn normalized(self) -> Option<Self> {
        if self.denominator <= 0 || self.numerator == i128::MIN {
            return None;
        }
        if self.numerator == 0 {
            return Some(Self::ZERO);
        }
        let gcd = signed_gcd(self.numerator, self.denominator)?;
        Some(Self {
            numerator: self.numerator.checked_div(gcd)?,
            denominator: self.denominator.checked_div(gcd)?,
        })
    }
}

fn scale_coefficient(coefficient: i128, decimal_places: i32) -> Option<i128> {
    let decimal_places = u32::try_from(decimal_places).ok()?;
    coefficient.checked_mul(10_i128.checked_pow(decimal_places)?)
}

fn signed_gcd(left: i128, right: i128) -> Option<i128> {
    if left == i128::MIN || right == i128::MIN {
        return None;
    }
    let mut left = left.unsigned_abs();
    let mut right = right.unsigned_abs();
    while right != 0 {
        (left, right) = (right, left % right);
    }
    i128::try_from(left).ok()
}

#[cfg(test)]
mod tests {
    use super::{DecimalTrace, RationalTrace};

    #[test]
    fn traces_distinguish_exact_cancellation_from_real_small_differences() {
        let inherited_zero = DecimalTrace::from_literal("100.1")
            .and_then(|value| value.subtract(DecimalTrace::from_literal("100.0")?))
            .and_then(|value| value.subtract(DecimalTrace::from_literal("0.1")?))
            .expect("fixture fits");
        assert!(inherited_zero.is_zero());

        for literal in ["0.099999999999999", "0.0999999999999996"] {
            let real_difference = DecimalTrace::from_literal("100.1")
                .and_then(|value| value.subtract(DecimalTrace::from_literal("100.0")?))
                .and_then(|value| value.subtract(DecimalTrace::from_literal(literal)?))
                .expect("fixture fits");
            assert!(!real_difference.is_zero(), "{literal}");
        }
    }

    #[test]
    fn traces_parse_exponents_signs_and_fraction_only_literals() {
        let one = DecimalTrace::from_literal("1").expect("integer");
        assert_eq!(DecimalTrace::from_literal("+1.0"), Some(one));
        assert_eq!(DecimalTrace::from_literal(".1e1"), Some(one));
        assert_eq!(
            DecimalTrace::from_literal("-2.5").and_then(DecimalTrace::negate),
            DecimalTrace::from_literal("2.5")
        );
    }

    #[test]
    fn extreme_exponents_stop_tracing_instead_of_overflowing() {
        let tiny = DecimalTrace::from_literal("1E-2147483648").expect("finite literal");
        assert_eq!(tiny.add(DecimalTrace::ONE), None);
        assert_eq!(DecimalTrace::ONE.subtract(tiny), None);
        assert_eq!(tiny.multiply(tiny), None);
    }

    #[test]
    fn products_of_exact_decimals_stay_exact() {
        let product = DecimalTrace::from_literal("0.1")
            .and_then(|left| left.multiply(DecimalTrace::from_literal("0.2")?))
            .expect("product fits");
        assert_eq!(
            product,
            DecimalTrace::from_literal("0.02").expect("literal")
        );
        assert!(
            product
                .subtract(DecimalTrace::from_literal("0.02").expect("literal"))
                .is_some_and(DecimalTrace::is_zero)
        );
    }

    #[test]
    fn rational_traces_preserve_discounted_decimal_cancellation() {
        let base = RationalTrace::from_decimal(
            DecimalTrace::ONE
                .add(DecimalTrace::from_literal("0.1").expect("rate"))
                .expect("base"),
        )
        .expect("base rational");
        let first =
            RationalTrace::from_decimal(DecimalTrace::from_literal("11").expect("first cashflow"))
                .and_then(|cashflow| cashflow.divide(base))
                .expect("first discounted cashflow");
        let second = RationalTrace::from_decimal(
            DecimalTrace::from_literal("-12.1").expect("second cashflow"),
        )
        .and_then(|cashflow| cashflow.divide(base.pow(2)?))
        .expect("second discounted cashflow");
        assert!(first.add(second).is_some_and(RationalTrace::is_zero));

        let nearby = RationalTrace::from_decimal(
            DecimalTrace::from_literal("-12.099999999999999").expect("nearby second cashflow"),
        )
        .and_then(|cashflow| cashflow.divide(base.pow(2)?))
        .and_then(|cashflow| first.add(cashflow))
        .expect("nearby discounted total");
        assert!(!nearby.is_zero());
    }
}
