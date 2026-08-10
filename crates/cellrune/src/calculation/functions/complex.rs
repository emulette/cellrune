use super::super::value::ErrorKind;

pub(super) const EXCEL_COMPLEX_NUMBER_BOUNDARY: f64 = 1e308;
const LN_MAX_FINITE: f64 = 709.782_712_893_384;
const LN_MIN_SUBNORMAL: f64 = -744.440_071_921_381_2;

/// The imaginary-unit spelling used by Excel engineering functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComplexSuffix {
    I,
    J,
}

impl ComplexSuffix {
    pub(super) fn from_text(text: &str) -> Option<Self> {
        match text {
            "i" => Some(Self::I),
            "j" => Some(Self::J),
            _ => None,
        }
    }

    const fn as_char(self) -> char {
        match self {
            Self::I => 'i',
            Self::J => 'j',
        }
    }
}

/// A finite complex number used only inside the engineering-function runtime.
///
/// Worksheet complex values are always represented by their canonical text, so this is deliberately
/// not a public calculation value variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ComplexValue {
    real: f64,
    imaginary: f64,
    suffix: ComplexSuffix,
}

impl ComplexValue {
    pub(super) const fn zero(suffix: ComplexSuffix) -> Self {
        Self {
            real: 0.0,
            imaginary: 0.0,
            suffix,
        }
    }

    pub(super) fn new(real: f64, imaginary: f64, suffix: ComplexSuffix) -> Result<Self, ErrorKind> {
        if !real.is_finite() || !imaginary.is_finite() {
            return Err(ErrorKind::Num);
        }
        Ok(Self {
            real,
            imaginary,
            suffix,
        })
    }

    pub(super) fn parse(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty() || text.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(ErrorKind::Num);
        }
        let Some(last) = text.as_bytes().last().copied() else {
            return Err(ErrorKind::Num);
        };
        let suffix = match last {
            b'i' => ComplexSuffix::I,
            b'j' => ComplexSuffix::J,
            _ => {
                let real = parse_number(text).ok_or(ErrorKind::Num)?;
                return Self::new(real, 0.0, ComplexSuffix::I);
            }
        };
        let body = &text[..text.len() - 1];
        if let Some(separator) = binary_separator(body) {
            let real = parse_number(&body[..separator]).ok_or(ErrorKind::Num)?;
            let imaginary = match &body[separator..] {
                "+" => 1.0,
                "-" => -1.0,
                coefficient => parse_number(coefficient).ok_or(ErrorKind::Num)?,
            };
            return Self::new(real, imaginary, suffix);
        }
        let imaginary = match body {
            "" | "+" => 1.0,
            "-" => -1.0,
            _ => parse_number(body).ok_or(ErrorKind::Num)?,
        };
        Self::new(0.0, imaginary, suffix)
    }

    pub(super) const fn real(self) -> f64 {
        self.real
    }

    pub(super) const fn imaginary(self) -> f64 {
        self.imaginary
    }

    pub(super) const fn suffix(self) -> ComplexSuffix {
        self.suffix
    }

    pub(super) fn format(self) -> String {
        let real_is_zero = self.real == 0.0;
        let imaginary_is_zero = self.imaginary == 0.0;
        if imaginary_is_zero {
            return format_component(self.real);
        }

        let suffix = self.suffix.as_char();
        let imaginary_is_one = self.imaginary.abs() == 1.0;
        if real_is_zero {
            let sign = if self.imaginary.is_sign_negative() {
                "-"
            } else {
                ""
            };
            let coefficient = if imaginary_is_one {
                String::new()
            } else {
                format_component(self.imaginary.abs())
            };
            return format!("{sign}{coefficient}{suffix}");
        }

        let separator = if self.imaginary.is_sign_negative() {
            '-'
        } else {
            '+'
        };
        let coefficient = if imaginary_is_one {
            String::new()
        } else {
            format_component(self.imaginary.abs())
        };
        format!(
            "{}{separator}{coefficient}{suffix}",
            format_component(self.real)
        )
    }

    pub(super) fn magnitude(self) -> f64 {
        self.real.hypot(self.imaginary)
    }

    pub(super) fn argument(self) -> f64 {
        self.imaginary.atan2(self.real)
    }

    pub(super) fn conjugate(self) -> Self {
        // Negating a finite component cannot make it non-finite.
        Self {
            real: self.real,
            imaginary: -self.imaginary,
            suffix: self.suffix,
        }
    }

    pub(super) fn subtract(self, other: Self) -> Result<Self, ErrorKind> {
        self.require_same_suffix(other)?;
        Self::new(
            self.real - other.real,
            self.imaginary - other.imaginary,
            self.suffix,
        )
    }

    pub(super) fn divide(self, other: Self) -> Result<Self, ErrorKind> {
        self.require_same_suffix(other)?;
        if other.real == 0.0 && other.imaginary == 0.0 {
            return Err(ErrorKind::Div0);
        }

        let reciprocal = ScaledComplex::from_value(other).reciprocal()?;
        ScaledComplex::from_value(self)
            .multiply(reciprocal)
            .finish(self.suffix)
    }

    pub(super) fn product(values: &[Self]) -> Result<Self, ErrorKind> {
        let Some(first) = values.first().copied() else {
            return Err(ErrorKind::Value);
        };
        let mut product = ScaledComplex::from_value(first);
        for value in &values[1..] {
            if value.suffix != first.suffix {
                return Err(ErrorKind::Value);
            }
            product = product.multiply(ScaledComplex::from_value(*value));
        }
        product.finish(first.suffix)
    }

    pub(super) fn sum(values: &[Self]) -> Result<Self, ErrorKind> {
        let Some(first) = values.first().copied() else {
            return Err(ErrorKind::Value);
        };
        let mut exponent = 0_i32;
        for value in values {
            if value.suffix != first.suffix {
                return Err(ErrorKind::Value);
            }
            exponent = exponent.max(component_exponent(value.real));
            exponent = exponent.max(component_exponent(value.imaginary));
        }
        let mut real = CompensatedSum::default();
        let mut imaginary = CompensatedSum::default();
        for value in values {
            real.add(libm::scalbn(value.real, -exponent));
            imaginary.add(libm::scalbn(value.imaginary, -exponent));
        }
        Self::new(
            libm::scalbn(real.total(), exponent),
            libm::scalbn(imaginary.total(), exponent),
            first.suffix,
        )
    }

    pub(super) fn exponential(self) -> Result<Self, ErrorKind> {
        let log_two = core::f64::consts::LN_2;
        let binary_exponent = self.real / log_two;
        if !binary_exponent.is_finite() {
            return if self.real.is_sign_negative() {
                Self::new(0.0, self.imaginary.signum() * 0.0, self.suffix)
            } else {
                Err(ErrorKind::Num)
            };
        }
        let exponent = binary_exponent.floor() as i32;
        let residual = self.real - f64::from(exponent) * log_two;
        let scale = residual.exp();
        Self::new(
            libm::scalbn(scale * self.imaginary.cos(), exponent),
            libm::scalbn(scale * self.imaginary.sin(), exponent),
            self.suffix,
        )
    }

    pub(super) fn logarithm(self) -> Result<Self, ErrorKind> {
        let magnitude = self.log_magnitude().ok_or(ErrorKind::Num)?;
        Self::new(magnitude, self.argument(), self.suffix)
    }

    pub(super) fn square_root(self) -> Result<Self, ErrorKind> {
        if self.real == 0.0 && self.imaginary == 0.0 {
            return Self::new(0.0, self.imaginary, self.suffix);
        }
        let scale_exponent = component_exponent(self.real).max(component_exponent(self.imaginary));
        let scaled_real = libm::scalbn(self.real, -scale_exponent);
        let scaled_imaginary = libm::scalbn(self.imaginary, -scale_exponent);
        let scaled_radius = scaled_real.hypot(scaled_imaginary);
        let radius_sqrt = scaled_square_root(scaled_radius, scale_exponent);
        let (real, imaginary) = if self.real >= 0.0 {
            let term = ((1.0 + scaled_real / scaled_radius) / 2.0).sqrt();
            let real = radius_sqrt * term;
            (real, self.imaginary / (2.0 * real))
        } else {
            let term = ((1.0 - scaled_real / scaled_radius) / 2.0).sqrt();
            let imaginary_magnitude = radius_sqrt * term;
            (
                self.imaginary.abs() / (2.0 * imaginary_magnitude),
                imaginary_magnitude.copysign(self.imaginary),
            )
        };
        Self::new(real, imaginary, self.suffix)
    }

    pub(super) fn power(
        self,
        exponent: f64,
        mut work: impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        if self.real == 0.0 && self.imaginary == 0.0 {
            if exponent <= 0.0 {
                return Err(ErrorKind::Num);
            }
            return Self::new(0.0, 0.0, self.suffix);
        }
        if exponent.fract() == 0.0 && exponent.abs() < i64::MAX as f64 {
            return self.integer_power(exponent as i64, &mut work);
        }
        work()?;
        let log_magnitude = self.log_magnitude().ok_or(ErrorKind::Num)?;
        let log_scale = exponent * log_magnitude;
        let angle = scaled_angle(exponent, self.argument());
        Self::new(
            component_from_log_scale(log_scale, angle.cos()),
            component_from_log_scale(log_scale, angle.sin()),
            self.suffix,
        )
    }

    fn integer_power(
        self,
        exponent: i64,
        work: &mut impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        let reciprocal = exponent < 0;
        let mut remaining = exponent.unsigned_abs();
        let mut factor = ScaledComplex::from_value(self);
        let mut result = ScaledComplex::one();
        while remaining > 0 {
            work()?;
            if remaining & 1 == 1 {
                result = result.multiply(factor);
            }
            remaining >>= 1;
            if remaining > 0 {
                factor = factor.multiply(factor);
            }
        }
        if reciprocal {
            result.reciprocal()?.finish(self.suffix)
        } else {
            result.finish(self.suffix)
        }
    }

    fn log_magnitude(self) -> Option<f64> {
        let high = self.real.abs().max(self.imaginary.abs());
        if high == 0.0 {
            return None;
        }
        let low = self.real.abs().min(self.imaginary.abs());
        Some(high.ln() + 0.5 * (low / high).powi(2).ln_1p())
    }

    fn require_same_suffix(self, other: Self) -> Result<(), ErrorKind> {
        if self.suffix == other.suffix {
            Ok(())
        } else {
            Err(ErrorKind::Value)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScaledComplex {
    real: f64,
    imaginary: f64,
    exponent: i32,
}

#[derive(Debug, Default)]
struct CompensatedSum {
    total: f64,
    compensation: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) {
        let next = self.total + value;
        self.compensation += if self.total.abs() >= value.abs() {
            (self.total - next) + value
        } else {
            (value - next) + self.total
        };
        self.total = next;
    }

    fn total(self) -> f64 {
        self.total + self.compensation
    }
}

impl ScaledComplex {
    const fn one() -> Self {
        Self {
            real: 1.0,
            imaginary: 0.0,
            exponent: 0,
        }
    }

    fn from_value(value: ComplexValue) -> Self {
        Self::normalized(value.real, value.imaginary, 0)
    }

    fn multiply(self, other: Self) -> Self {
        Self::normalized(
            self.real * other.real - self.imaginary * other.imaginary,
            self.real * other.imaginary + self.imaginary * other.real,
            self.exponent.saturating_add(other.exponent),
        )
    }

    fn finish(self, suffix: ComplexSuffix) -> Result<ComplexValue, ErrorKind> {
        ComplexValue::new(
            libm::scalbn(self.real, self.exponent),
            libm::scalbn(self.imaginary, self.exponent),
            suffix,
        )
    }

    fn reciprocal(self) -> Result<Self, ErrorKind> {
        let denominator = self
            .real
            .mul_add(self.real, self.imaginary * self.imaginary);
        if denominator == 0.0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Self::normalized(
            self.real / denominator,
            -self.imaginary / denominator,
            self.exponent.saturating_neg(),
        ))
    }

    fn normalized(real: f64, imaginary: f64, exponent: i32) -> Self {
        let scale_exponent = component_exponent(real).max(component_exponent(imaginary));
        if scale_exponent == 0 && real == 0.0 && imaginary == 0.0 {
            return Self {
                real,
                imaginary,
                exponent,
            };
        }
        Self {
            real: libm::scalbn(real, -scale_exponent),
            imaginary: libm::scalbn(imaginary, -scale_exponent),
            exponent: exponent.saturating_add(scale_exponent),
        }
    }
}

fn binary_separator(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    (1..bytes.len()).rev().find(|&index| {
        matches!(bytes[index], b'+' | b'-') && !matches!(bytes[index - 1], b'e' | b'E')
    })
}

fn parse_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }
    let digits_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let has_integer_digits = index > digits_start;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if !has_integer_digits && index == fraction_start {
            return None;
        }
    } else if !has_integer_digits {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return None;
        }
    }
    (index == bytes.len())
        .then(|| text.parse::<f64>().ok())
        .flatten()
        .filter(|number| number.is_finite())
}

fn component_exponent(value: f64) -> i32 {
    if value == 0.0 {
        0
    } else {
        libm::ilogb(value.abs()) + 1
    }
}

fn scaled_square_root(value: f64, exponent: i32) -> f64 {
    let half_exponent = exponent.div_euclid(2);
    let odd_exponent = exponent.rem_euclid(2);
    let mantissa = (value * if odd_exponent == 0 { 1.0 } else { 2.0 }).sqrt();
    libm::scalbn(mantissa, half_exponent)
}

fn scaled_angle(exponent: f64, argument: f64) -> f64 {
    if exponent == 0.0 || argument == 0.0 {
        return exponent * argument;
    }
    // Reduce the exponent before multiplying. This preserves a finite unit-magnitude result
    // such as i^1e308 instead of turning the angle into infinity first.
    let period = 2.0 * core::f64::consts::PI / argument.abs();
    if period.is_finite() {
        exponent.rem_euclid(period) * argument
    } else {
        exponent * argument
    }
}

fn component_from_log_scale(log_scale: f64, direction: f64) -> f64 {
    if direction == 0.0 {
        return direction;
    }
    let log_component = log_scale + direction.abs().ln();
    let magnitude = if log_component > LN_MAX_FINITE {
        f64::INFINITY
    } else if log_component < LN_MIN_SUBNORMAL {
        0.0
    } else {
        log_component.exp()
    };
    magnitude.copysign(direction)
}

fn format_component(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let exponent = value.abs().log10().floor() as i32;
    if (-15..=15).contains(&exponent) {
        let precision = (14 - exponent).max(0) as usize;
        trim_decimal(format!("{value:.precision$}"))
    } else {
        let scientific = format!("{value:.14e}");
        let Some((mantissa, exponent)) = scientific.split_once('e') else {
            return scientific;
        };
        let exponent = if exponent.starts_with('-') {
            exponent.to_owned()
        } else {
            format!("+{}", exponent.trim_start_matches('+'))
        };
        format!("{}E{exponent}", trim_decimal(mantissa.to_owned()))
    }
}

fn trim_decimal(mut text: String) -> String {
    if let Some(point) = text.find('.') {
        let mut end = text.len();
        while end > point + 1 && text.as_bytes()[end - 1] == b'0' {
            end -= 1;
        }
        if end == point + 1 {
            end = point;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{ComplexSuffix, ComplexValue};
    use crate::calculation::value::ErrorKind;

    fn complex(real: f64, imaginary: f64) -> ComplexValue {
        ComplexValue::new(real, imaginary, ComplexSuffix::I).expect("finite complex")
    }

    fn assert_close(actual: f64, expected: f64, absolute: f64, relative: f64) {
        let tolerance = absolute + relative * expected.abs();
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual:?}, expected={expected:?}, tolerance={tolerance:?}"
        );
    }

    #[test]
    fn parser_and_formatter_cover_excel_complex_spellings() {
        for (input, real, imaginary, suffix, formatted) in [
            ("-1.25e+3", -1250.0, 0.0, ComplexSuffix::I, "-1250"),
            ("-2.5e-4j", 0.0, -0.00025, ComplexSuffix::J, "-0.00025j"),
            ("-i", 0.0, -1.0, ComplexSuffix::I, "-i"),
            ("1+i", 1.0, 1.0, ComplexSuffix::I, "1+i"),
            (
                "1e-3-2e+4i",
                0.001,
                -20000.0,
                ComplexSuffix::I,
                "0.001-20000i",
            ),
        ] {
            let parsed = ComplexValue::parse(input).expect(input);
            assert_eq!(parsed.real(), real, "{input}");
            assert_eq!(parsed.imaginary(), imaginary, "{input}");
            assert_eq!(parsed.suffix(), suffix, "{input}");
            assert_eq!(parsed.format(), formatted, "{input}");
        }
        assert_eq!(complex(0.0, 1.0).format(), "i");
        assert_eq!(complex(0.0, -1.0).format(), "-i");
        assert_eq!(complex(-0.0, -0.0).format(), "0");
        assert_eq!(complex(1e15, 0.0).format(), "1000000000000000");
        assert_eq!(complex(1e16, 0.0).format(), "1E+16");
        assert_eq!(complex(1e-10, 0.0).format(), "0.0000000001");
        assert_eq!(complex(1e-16, 0.0).format(), "1E-16");
        assert_eq!(
            complex(1e15, 1e-15).format(),
            "1000000000000000+0.000000000000001i"
        );
        assert_eq!(complex(1e99, 1e-99).format(), "1E+99+1E-99i");
        assert_eq!(
            complex(-13.128_783_081_462_158, -15.200_784_463_067_954).format(),
            "-13.1287830814622-15.200784463068i"
        );
        for value in [
            f64::from_bits(1),
            1e-300,
            1e-10,
            1e15,
            1e16,
            9.999_999_999_999_99e307,
        ] {
            let formatted = complex(value, -value).format();
            let reparsed = ComplexValue::parse(&formatted).expect("formatter output must parse");
            assert!(reparsed.real().is_finite(), "{formatted}");
            assert!(reparsed.imaginary().is_finite(), "{formatted}");
        }
        for input in ["1 + 2i", "1+-2i", "1e", "iI", "NaNi", "1+2I"] {
            assert_eq!(ComplexValue::parse(input), Err(ErrorKind::Num), "{input}");
        }
    }

    #[test]
    fn construction_and_component_operations_are_stable() {
        let value = complex(3.0, 4.0);
        assert_eq!(value.format(), "3+4i");
        assert_eq!(value.magnitude(), 5.0);
        assert_close(value.argument(), 0.927_295_218_001_612_2, 5e-15, 5e-13);
        assert_eq!(value.conjugate().format(), "3-4i");
        assert_eq!(value.real(), 3.0);
        assert_eq!(value.imaginary(), 4.0);
        assert_close(
            complex(1e308, 1e308).magnitude(),
            1.414_213_562_373_095_1e308,
            5e-14,
            2e-12,
        );
        assert!(complex(f64::MAX, f64::MAX).magnitude().is_infinite());
    }

    #[test]
    fn arithmetic_preserves_suffixes_and_extreme_scaling() {
        let value = complex(3.0, 4.0);
        let divisor = complex(1.0, -2.0);
        assert_eq!(value.divide(divisor).expect("division").format(), "-1+2i");
        assert_eq!(
            value.subtract(divisor).expect("subtraction").format(),
            "2+6i"
        );
        assert_eq!(
            ComplexValue::sum(&[value, divisor]).expect("sum").format(),
            "4+2i"
        );
        assert_eq!(
            ComplexValue::product(&[value, divisor])
                .expect("product")
                .format(),
            "11-2i"
        );
        assert_eq!(
            ComplexValue::product(&[complex(1e200, 1e200), complex(1e-200, -1e-200)])
                .expect("scaled product")
                .format(),
            "2"
        );
        let quotient = complex(1e308, 1.0)
            .divide(complex(1e308, -1e308))
            .expect("scaled division");
        assert_close(quotient.real(), 0.5, 5e-14, 2e-12);
        assert_close(quotient.imaginary(), 0.5, 5e-14, 2e-12);
        let small_denominator = complex(9e307, 0.0)
            .divide(complex(0.5, 0.5))
            .expect("small-denominator scaled division");
        assert_close(small_denominator.real(), 9e307, 0.0, 2e-12);
        assert_close(small_denominator.imaginary(), -9e307, 0.0, 2e-12);
        assert_eq!(
            ComplexValue::sum(&[
                complex(1e50, -1e50),
                complex(1.0, 2.0),
                complex(-1e50, 1e50)
            ])
            .expect("scaled sum")
            .format(),
            "1+2i"
        );
        assert_eq!(
            complex(1.0, 1.0)
                .divide(ComplexValue::new(1.0, 1.0, ComplexSuffix::J).expect("finite")),
            Err(ErrorKind::Value)
        );
    }

    #[test]
    fn powers_use_principal_branches_and_integer_scaling() {
        assert_eq!(
            complex(3.0, 4.0)
                .power(3.0, || Ok(()))
                .expect("integer power")
                .format(),
            "-117+44i"
        );
        let fractional = complex(-4.0, 1e-40)
            .power(0.5, || Ok(()))
            .expect("fractional power");
        assert_close(fractional.real(), 2.5e-41, 2e-13, 0.0);
        assert_close(fractional.imaginary(), 2.0, 2e-13, 0.0);
        let finite_components_above_max_radius = complex(1e308, 1e308)
            .power(1.00034, || Ok(()))
            .expect("fractional power with finite Cartesian components");
        assert_close(
            finite_components_above_max_radius.real(),
            1.272_492_326_619_526_4e308,
            0.0,
            2e-12,
        );
        assert_close(
            finite_components_above_max_radius.imaginary(),
            1.273_172_109_094_312_5e308,
            0.0,
            2e-12,
        );
        assert_eq!(
            complex(1e200, 0.0)
                .power(-2.0, || Ok(()))
                .expect("negative scaled power")
                .format(),
            "0"
        );
        let negative = complex(3.0, 4.0)
            .power(-7.0, || Ok(()))
            .expect("negative power");
        assert_close(negative.real(), 1.252_442_112e-5, 5e-15, 5e-13);
        assert_close(negative.imaginary(), -2.641_756_16e-6, 5e-15, 5e-13);
        assert_eq!(complex(0.0, 0.0).power(0.0, || Ok(())), Err(ErrorKind::Num));
        let large_unit_power = complex(0.0, 1.0)
            .power(1e308, || Ok(()))
            .expect("large exponent with unit magnitude");
        assert!(large_unit_power.real().is_finite());
        assert!(large_unit_power.imaginary().is_finite());
    }

    #[test]
    fn transcendental_operations_follow_the_principal_branch() {
        let value = complex(3.0, 4.0);
        let exponential = value.exponential().expect("exponential");
        assert_close(exponential.real(), -13.128_783_081_462_158, 5e-15, 5e-13);
        assert_close(
            exponential.imaginary(),
            -15.200_784_463_067_954,
            5e-15,
            5e-13,
        );
        let logarithm = value.logarithm().expect("logarithm");
        assert_close(logarithm.real(), 1.609_437_912_434_100_3, 5e-15, 5e-13);
        assert_close(logarithm.imaginary(), 0.927_295_218_001_612_2, 5e-15, 5e-13);
        assert_eq!(value.square_root().expect("square root").format(), "2+i");
        let below_cut = complex(-4.0, -1e-40).square_root().expect("square root");
        assert_close(below_cut.real(), 2.5e-41, 2e-13, 0.0);
        assert_close(below_cut.imaginary(), -2.0, 2e-13, 0.0);
        let tiny_log = complex(1e-308, -1e-308).logarithm().expect("tiny log");
        assert_close(tiny_log.real(), -708.849_635_051_886_1, 5e-14, 2e-12);
        assert_close(
            tiny_log.imaginary(),
            -core::f64::consts::FRAC_PI_4,
            5e-14,
            2e-12,
        );
        let huge_root = complex(1e308, 1e308)
            .square_root()
            .expect("huge square root");
        assert_close(huge_root.real(), 1.098_684_113_467_81e154, 5e-14, 2e-12);
        assert_close(
            huge_root.imaginary(),
            4.550_898_605_622_273_4e153,
            5e-14,
            2e-12,
        );
        let max_root = complex(f64::MAX, f64::MAX)
            .square_root()
            .expect("scaled max square root");
        assert!(max_root.real().is_finite());
        assert!(max_root.imaginary().is_finite());
        let balanced_overflow = complex(f64::MAX.ln() + 0.1, core::f64::consts::FRAC_PI_4)
            .exponential()
            .expect("scaled exponential with finite components");
        assert!(balanced_overflow.real().is_finite());
        assert!(balanced_overflow.imaginary().is_finite());
        let underflow = complex(-1000.0, 0.25)
            .exponential()
            .expect("scaled exponential underflow");
        assert_eq!(underflow.real(), 0.0);
        assert_eq!(underflow.imaginary(), 0.0);
        assert_eq!(complex(1000.0, 0.25).exponential(), Err(ErrorKind::Num));
    }

    #[test]
    fn worksheet_adapter_exposes_exactly_the_complex_function_surface() {
        use crate::{
            CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellValue,
            ExcelError, FormulaText, WorkbookDraft, calculate_workbook,
        };

        let mut draft = WorkbookDraft::new();
        let sheet = draft.workbook().sheets()[0].id();
        for (address, formula) in [
            ("A1", "=COMPLEX(3,4,\"i\")"),
            ("A2", "=IMABS(\"3+4i\")"),
            ("A3", "=IMAGINARY(\"3+4i\")"),
            ("A4", "=IMARGUMENT(\"3+4i\")"),
            ("A5", "=IMCONJUGATE(\"3+4i\")"),
            ("A6", "=IMREAL(\"3+4i\")"),
            ("A7", "=IMDIV(\"3+4i\",\"1-2i\")"),
            ("A8", "=IMPOWER(\"3+4i\",3)"),
            ("A9", "=IMPRODUCT(\"3+4i\",\"1-2i\")"),
            ("A10", "=IMSUB(\"3+4i\",\"1-2i\")"),
            ("A11", "=IMSUM(\"3+4i\",\"1-2i\")"),
            ("A12", "=IMEXP(\"3+4i\")"),
            ("A13", "=IMLN(\"3+4i\")"),
            ("A14", "=IMSQRT(\"3+4i\")"),
            (
                "A15",
                "=IMABS(\"1.7976931348623157E308+1.7976931348623157E308i\")",
            ),
            ("A16", "=COMPLEX(1000000000000000,0)"),
            ("A17", "=COMPLEX(1E16,0)"),
            ("A18", "=COMPLEX(1E308,0)"),
            ("A19", "=IMPOWER(\"0.6+0.8i\",1E19)"),
            ("A20", "=IMDIV(\"9E307\",\"0.5+0.5i\")"),
            ("A21", "=IMREAL(IMPOWER(\"1E308+1E308i\",1.00034))"),
            ("A22", "=IMAGINARY(IMPOWER(\"1E308+1E308i\",1.00034))"),
            ("A23", "=IMSUM(C1:C2)"),
            ("A24", "=IMPRODUCT(C1:C2)"),
            ("A25", "=IMSUM(C1)"),
            ("A26", "=IMPRODUCT(C1)"),
            ("A27", "=COMPLEX(9E307,0)"),
            ("A28", "=COMPLEX(1E15,1E-15)"),
        ] {
            draft
                .set_cell_formula(
                    sheet,
                    CellAddress::from_a1(address).expect("valid test address"),
                    FormulaText::from_user_input(formula).expect("valid test formula"),
                )
                .expect("formula edit");
        }
        draft
            .set_cell_value(
                sheet,
                CellAddress::from_a1("C2").expect("valid collection value address"),
                CellValue::Text("3+4j".to_owned()),
            )
            .expect("collection value edit");
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let result = |address: &str| {
            calculation.cell(CalculationCellId::new(
                sheet,
                CellAddress::from_a1(address).expect("valid test address"),
            ))
        };
        for (address, expected) in [
            ("A1", "3+4i"),
            ("A5", "3-4i"),
            ("A7", "-1+2i"),
            ("A8", "-117+44i"),
            ("A9", "11-2i"),
            ("A10", "2+6i"),
            ("A11", "4+2i"),
            ("A12", "-13.1287830814622-15.200784463068i"),
            ("A13", "1.6094379124341+0.927295218001612i"),
            ("A14", "2+i"),
            ("A16", "1000000000000000"),
            ("A17", "1E+16"),
            ("A20", "9E+307-9E+307i"),
            ("A23", "3+4j"),
            ("A24", "0"),
            ("A27", "9E+307"),
            ("A28", "1000000000000000+0.000000000000001i"),
        ] {
            assert_eq!(
                result(address),
                Some(&CalculationCellResult::Value(CellValue::Text(
                    expected.to_owned()
                ))),
                "{address}"
            );
        }
        for (address, expected) in [
            ("A2", 5.0),
            ("A3", 4.0),
            ("A4", 0.927_295_218_001_612_2),
            ("A6", 3.0),
            ("A21", 1.272_492_326_619_526_4e308),
            ("A22", 1.273_172_109_094_312_5e308),
        ] {
            let Some(CalculationCellResult::Value(CellValue::Number(value))) = result(address)
            else {
                panic!(
                    "numeric complex result expected at {address}: {:?}",
                    result(address)
                );
            };
            assert_close(value.get(), expected, 5e-15, 2e-12);
        }
        for address in ["A15", "A18", "A19", "A25", "A26"] {
            assert_eq!(
                result(address),
                Some(&CalculationCellResult::Value(CellValue::Error(
                    ExcelError::Number,
                ))),
                "{address}"
            );
        }
    }
}
