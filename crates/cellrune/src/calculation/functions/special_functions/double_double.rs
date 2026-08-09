//! Double-double (hi + lo) arithmetic for the incomplete-beta continued
//! fraction.
//!
//! The 0.1.13 CF recurrence contract (FORMULAS.md §2.2) evaluates every
//! recurrence variable in ~106-bit arithmetic and rounds only the final
//! fraction to f64; an ordinary f64 recurrence loses up to 6.8e-9 relative
//! next to the central band for asymmetric large shapes. The operations are
//! transcribed from the validated `double_double.py` reference: `TwoSum` with
//! the `fma`-based product error, and a reciprocal refined by two Newton
//! iterations.

#[derive(Debug, Clone, Copy)]
pub(super) struct DoubleDouble {
    pub(super) hi: f64,
    pub(super) lo: f64,
}

impl DoubleDouble {
    pub(super) const fn new(hi: f64) -> Self {
        Self { hi, lo: 0.0 }
    }

    const fn pair(hi: f64, lo: f64) -> Self {
        Self { hi, lo }
    }

    /// Error-free sum of two f64 values (Knuth TwoSum).
    fn two_sum(left: f64, right: f64) -> (f64, f64) {
        let total = left + right;
        let right_virtual = total - left;
        let error = (left - (total - right_virtual)) + (right - right_virtual);
        (total, error)
    }

    fn normalized(hi: f64, lo: f64) -> Self {
        let (total, error) = Self::two_sum(hi, lo);
        Self::pair(total, error)
    }

    pub(super) fn add(self, other: Self) -> Self {
        let (total, error) = Self::two_sum(self.hi, other.hi);
        Self::normalized(total, error + self.lo + other.lo)
    }

    pub(super) fn sub(self, other: Self) -> Self {
        self.add(other.neg())
    }

    pub(super) fn neg(self) -> Self {
        Self::pair(-self.hi, -self.lo)
    }

    /// Product with the first-rounding error recovered through `fma`
    /// (evaluated in the reference's `fma(hi, hi, -product)` order).
    pub(super) fn mul(self, other: Self) -> Self {
        let product = self.hi * other.hi;
        let error = self.hi.mul_add(other.hi, -product) + self.hi * other.lo + self.lo * other.hi;
        Self::normalized(product, error)
    }

    /// Reciprocal refined by two Newton iterations over the f64 seed; the
    /// residual is evaluated in double-double (reference `reciprocal()`).
    pub(super) fn reciprocal(self) -> Self {
        let first = 1.0 / self.hi;
        let mut result = Self::new(first);
        let residual = Self::new(1.0).sub(self.mul(result));
        result = result.add(Self::new(residual.hi / self.hi));
        let residual = Self::new(1.0).sub(self.mul(result));
        result.add(Self::new(residual.hi / self.hi))
    }

    pub(super) fn div(self, other: Self) -> Self {
        self.mul(other.reciprocal())
    }

    /// abs(hi) + abs(lo): the magnitude used by the Lentz floor and the
    /// convergence test, both of which the reference expresses this way.
    pub(super) fn magnitude(self) -> f64 {
        self.hi.abs() + self.lo.abs()
    }

    /// Round the unevaluated sum back to f64 (`float(DoubleDouble)` in the
    /// reference).
    pub(super) fn as_f64(self) -> f64 {
        self.hi + self.lo
    }
}
