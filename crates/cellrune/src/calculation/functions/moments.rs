use super::super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VarianceKind {
    Sample,
    Population,
}

#[derive(Debug, Clone, Copy, Default)]
struct CompensatedSum {
    sum: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) -> Result<(), ErrorKind> {
        if !value.is_finite() {
            return Err(ErrorKind::Num);
        }
        let next = self.sum + value;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        let correction = if self.sum.abs() >= value.abs() {
            (self.sum - next) + value
        } else {
            (value - next) + self.sum
        };
        let next_correction = self.correction + correction;
        if !next_correction.is_finite() {
            return Err(ErrorKind::Num);
        }
        self.sum = next;
        self.correction = next_correction;
        Ok(())
    }

    fn merge(&mut self, other: Self) -> Result<(), ErrorKind> {
        self.add(other.sum)?;
        self.add(other.correction)
    }

    fn total(self) -> Result<f64, ErrorKind> {
        let total = self.sum + self.correction;
        if total.is_finite() {
            Ok(total)
        } else {
            Err(ErrorKind::Num)
        }
    }
}

/// Mergeable Welford/Chan moments for stable sums, means, and variances.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct NumericMoments {
    count: u64,
    mean: f64,
    second_moment: f64,
    sum: CompensatedSum,
}

impl NumericMoments {
    const PARTITION_SIZE: u64 = 256;

    pub(super) const fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            second_moment: 0.0,
            sum: CompensatedSum {
                sum: 0.0,
                correction: 0.0,
            },
        }
    }

    pub(super) fn add(&mut self, value: f64) -> Result<(), ErrorKind> {
        let next_count = self.count.checked_add(1).ok_or(ErrorKind::Num)?;
        let mut next_sum = self.sum;
        next_sum.add(value)?;
        let (next_mean, next_second_moment) = if self.count == 0 {
            (value, 0.0)
        } else {
            let delta = value - self.mean;
            let mean = self.mean + delta / next_count as f64;
            let second_moment = self.second_moment + delta * (value - mean);
            if !mean.is_finite() || !second_moment.is_finite() {
                return Err(ErrorKind::Num);
            }
            (mean, second_moment)
        };
        self.count = next_count;
        self.mean = next_mean;
        self.second_moment = next_second_moment;
        self.sum = next_sum;
        Ok(())
    }

    pub(super) fn collect_with_work(
        values: impl IntoIterator<Item = f64>,
        mut on_value: impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        let mut combined = Self::new();
        let mut partition = Self::new();
        for value in values {
            on_value()?;
            partition.add(value)?;
            if partition.count == Self::PARTITION_SIZE {
                combined.merge(partition)?;
                partition = Self::new();
            }
        }
        combined.merge(partition)?;
        Ok(combined)
    }

    #[cfg(test)]
    fn collect(values: impl IntoIterator<Item = f64>) -> Result<Self, ErrorKind> {
        Self::collect_with_work(values, || Ok(()))
    }

    pub(super) fn merge(&mut self, other: Self) -> Result<(), ErrorKind> {
        if other.count == 0 {
            return Ok(());
        }
        if self.count == 0 {
            *self = other;
            return Ok(());
        }
        let next_count = self.count.checked_add(other.count).ok_or(ErrorKind::Num)?;
        let delta = other.mean - self.mean;
        let other_weight = other.count as f64 / next_count as f64;
        let cross_weight = self.count as f64 * other.count as f64 / next_count as f64;
        let next_mean = self.mean + delta * other_weight;
        let next_second_moment =
            self.second_moment + other.second_moment + delta * delta * cross_weight;
        if !next_mean.is_finite() || !next_second_moment.is_finite() {
            return Err(ErrorKind::Num);
        }
        let mut next_sum = self.sum;
        next_sum.merge(other.sum)?;
        self.count = next_count;
        self.mean = next_mean;
        self.second_moment = next_second_moment;
        self.sum = next_sum;
        Ok(())
    }

    pub(super) fn sum(self) -> Result<f64, ErrorKind> {
        self.sum.total()
    }

    pub(super) fn mean(self) -> Result<f64, ErrorKind> {
        if self.count == 0 {
            return Err(ErrorKind::Div0);
        }
        if self.mean.is_finite() {
            Ok(self.mean)
        } else {
            Err(ErrorKind::Num)
        }
    }

    pub(super) fn variance(self, kind: VarianceKind) -> Result<f64, ErrorKind> {
        let divisor = match kind {
            VarianceKind::Sample if self.count < 2 => return Err(ErrorKind::Div0),
            VarianceKind::Sample => (self.count - 1) as f64,
            VarianceKind::Population if self.count == 0 => return Err(ErrorKind::Div0),
            VarianceKind::Population => self.count as f64,
        };
        let variance = self.second_moment.max(0.0) / divisor;
        if variance.is_finite() {
            Ok(variance)
        } else {
            Err(ErrorKind::Num)
        }
    }
}

/// Mergeable paired moments for covariance and regression kernels.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PairedMoments {
    count: u64,
    left_mean: f64,
    right_mean: f64,
    left_second_moment: f64,
    right_second_moment: f64,
    co_moment: f64,
    left_sum: CompensatedSum,
    right_sum: CompensatedSum,
}

impl PairedMoments {
    const PARTITION_SIZE: u64 = 256;

    pub(super) const fn new() -> Self {
        Self {
            count: 0,
            left_mean: 0.0,
            right_mean: 0.0,
            left_second_moment: 0.0,
            right_second_moment: 0.0,
            co_moment: 0.0,
            left_sum: CompensatedSum {
                sum: 0.0,
                correction: 0.0,
            },
            right_sum: CompensatedSum {
                sum: 0.0,
                correction: 0.0,
            },
        }
    }

    pub(super) fn add(&mut self, left: f64, right: f64) -> Result<(), ErrorKind> {
        let next_count = self.count.checked_add(1).ok_or(ErrorKind::Num)?;
        let mut next_left_sum = self.left_sum;
        let mut next_right_sum = self.right_sum;
        next_left_sum.add(left)?;
        next_right_sum.add(right)?;
        let (left_mean, right_mean, left_m2, right_m2, co_moment) = if self.count == 0 {
            (left, right, 0.0, 0.0, 0.0)
        } else {
            let left_delta = left - self.left_mean;
            let right_delta = right - self.right_mean;
            let left_mean = self.left_mean + left_delta / next_count as f64;
            let right_mean = self.right_mean + right_delta / next_count as f64;
            (
                left_mean,
                right_mean,
                self.left_second_moment + left_delta * (left - left_mean),
                self.right_second_moment + right_delta * (right - right_mean),
                self.co_moment + left_delta * (right - right_mean),
            )
        };
        if [left_mean, right_mean, left_m2, right_m2, co_moment]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(ErrorKind::Num);
        }
        self.count = next_count;
        self.left_mean = left_mean;
        self.right_mean = right_mean;
        self.left_second_moment = left_m2;
        self.right_second_moment = right_m2;
        self.co_moment = co_moment;
        self.left_sum = next_left_sum;
        self.right_sum = next_right_sum;
        Ok(())
    }

    pub(super) fn collect_with_work(
        values: impl IntoIterator<Item = (f64, f64)>,
        mut on_value: impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<Self, ErrorKind> {
        let mut combined = Self::new();
        let mut partition = Self::new();
        for (left, right) in values {
            on_value()?;
            partition.add(left, right)?;
            if partition.count == Self::PARTITION_SIZE {
                combined.merge(partition)?;
                partition = Self::new();
            }
        }
        combined.merge(partition)?;
        Ok(combined)
    }

    #[cfg(test)]
    fn collect(values: impl IntoIterator<Item = (f64, f64)>) -> Result<Self, ErrorKind> {
        Self::collect_with_work(values, || Ok(()))
    }

    pub(super) fn merge(&mut self, other: Self) -> Result<(), ErrorKind> {
        if other.count == 0 {
            return Ok(());
        }
        if self.count == 0 {
            *self = other;
            return Ok(());
        }
        let next_count = self.count.checked_add(other.count).ok_or(ErrorKind::Num)?;
        let left_delta = other.left_mean - self.left_mean;
        let right_delta = other.right_mean - self.right_mean;
        let other_weight = other.count as f64 / next_count as f64;
        let cross_weight = self.count as f64 * other.count as f64 / next_count as f64;
        let next = Self {
            count: next_count,
            left_mean: self.left_mean + left_delta * other_weight,
            right_mean: self.right_mean + right_delta * other_weight,
            left_second_moment: self.left_second_moment
                + other.left_second_moment
                + left_delta * left_delta * cross_weight,
            right_second_moment: self.right_second_moment
                + other.right_second_moment
                + right_delta * right_delta * cross_weight,
            co_moment: self.co_moment + other.co_moment + left_delta * right_delta * cross_weight,
            left_sum: self.left_sum,
            right_sum: self.right_sum,
        };
        if [
            next.left_mean,
            next.right_mean,
            next.left_second_moment,
            next.right_second_moment,
            next.co_moment,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return Err(ErrorKind::Num);
        }
        let mut next = next;
        next.left_sum.merge(other.left_sum)?;
        next.right_sum.merge(other.right_sum)?;
        *self = next;
        Ok(())
    }

    pub(super) fn left_mean(self) -> Result<f64, ErrorKind> {
        if self.count == 0 {
            Err(ErrorKind::Div0)
        } else {
            Ok(self.left_mean)
        }
    }

    pub(super) fn right_mean(self) -> Result<f64, ErrorKind> {
        if self.count == 0 {
            Err(ErrorKind::Div0)
        } else {
            Ok(self.right_mean)
        }
    }

    pub(super) const fn left_second_moment(self) -> f64 {
        self.left_second_moment
    }

    pub(super) const fn right_second_moment(self) -> f64 {
        self.right_second_moment
    }

    pub(super) const fn co_moment(self) -> f64 {
        self.co_moment
    }

    pub(super) fn covariance(self, kind: VarianceKind) -> Result<f64, ErrorKind> {
        let divisor = match kind {
            VarianceKind::Sample if self.count < 2 => return Err(ErrorKind::Div0),
            VarianceKind::Sample => (self.count - 1) as f64,
            VarianceKind::Population if self.count == 0 => return Err(ErrorKind::Div0),
            VarianceKind::Population => self.count as f64,
        };
        let covariance = self.co_moment / divisor;
        if covariance.is_finite() {
            Ok(covariance)
        } else {
            Err(ErrorKind::Num)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NumericMoments, PairedMoments, VarianceKind};
    use crate::calculation::value::ErrorKind;

    fn numeric(values: &[f64]) -> NumericMoments {
        NumericMoments::collect(values.iter().copied()).expect("finite moment input")
    }

    fn paired(values: &[(f64, f64)]) -> PairedMoments {
        PairedMoments::collect(values.iter().copied()).expect("finite paired input")
    }

    #[test]
    fn numeric_moments_keep_small_variance_beside_a_large_offset() {
        let moments = numeric(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0, 1.0e12 + 4.0]);
        assert_eq!(moments.count, 4);
        assert_eq!(moments.mean(), Ok(1.0e12 + 2.5));
        assert_eq!(moments.variance(VarianceKind::Population), Ok(1.25));
        assert_eq!(moments.variance(VarianceKind::Sample), Ok(5.0 / 3.0));
    }

    #[test]
    fn moments_merge_matches_single_pass_and_preserves_compensated_sum() {
        let values = [1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0, 1.0e12 + 4.0];
        let all = numeric(&values);
        let mut merged = numeric(&values[..2]);
        merged.merge(numeric(&values[2..])).expect("finite merge");
        assert_eq!(merged.count, all.count);
        assert_eq!(merged.mean(), all.mean());
        assert_eq!(
            merged.variance(VarianceKind::Population),
            all.variance(VarianceKind::Population)
        );

        let compensated = numeric(&[1.0e16, 1.0, -1.0e16]);
        assert_eq!(compensated.sum(), Ok(1.0));
    }

    #[test]
    fn paired_moments_merge_stably_for_covariance_and_regression_terms() {
        let values = [
            (1.0e12 + 1.0, 3.0e12 + 3.0),
            (1.0e12 + 2.0, 3.0e12 + 6.0),
            (1.0e12 + 3.0, 3.0e12 + 9.0),
            (1.0e12 + 4.0, 3.0e12 + 12.0),
        ];
        let all = paired(&values);
        let mut merged = paired(&values[..2]);
        merged.merge(paired(&values[2..])).expect("finite merge");
        assert_eq!(merged.count, all.count);
        assert_eq!(merged.left_second_moment(), 5.0);
        assert_eq!(merged.right_second_moment(), 45.0);
        assert_eq!(merged.co_moment(), 15.0);
        assert_eq!(merged.covariance(VarianceKind::Population), Ok(3.75));
    }

    #[test]
    fn moments_reject_non_finite_inputs_and_enforce_sample_sizes() {
        let mut numeric = NumericMoments::new();
        assert_eq!(numeric.add(f64::INFINITY), Err(ErrorKind::Num));
        assert_eq!(
            numeric.variance(VarianceKind::Population),
            Err(ErrorKind::Div0)
        );
        numeric.add(1.0).expect("finite value");
        assert_eq!(numeric.variance(VarianceKind::Sample), Err(ErrorKind::Div0));
        let mut paired = PairedMoments::new();
        assert_eq!(paired.add(f64::NAN, 1.0), Err(ErrorKind::Num));
    }
}
