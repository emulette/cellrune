/// Typed fixed-income model shared by every worksheet adapter and numeric kernel.
///
/// Raw `f64` serials and integer codes are confined to the worksheet boundary in the sibling
/// adapter modules. Once validated, the schedule and kernels receive these typed values so day
/// counts, coupon schedules, and pricing reductions cannot silently mix basis codes with actual
/// day measures.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DayCountBasis {
    Us30360,
    ActualActual,
    Actual360,
    Actual365,
    European30360,
}

impl DayCountBasis {
    pub(super) const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Us30360),
            1 => Some(Self::ActualActual),
            2 => Some(Self::Actual360),
            3 => Some(Self::Actual365),
            4 => Some(Self::European30360),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CouponFrequency {
    Annual,
    Semiannual,
    Quarterly,
}

impl CouponFrequency {
    pub(super) const fn from_code(code: i32) -> Option<Self> {
        match code {
            1 => Some(Self::Annual),
            2 => Some(Self::Semiannual),
            4 => Some(Self::Quarterly),
            _ => None,
        }
    }

    pub(super) const fn as_f64(self) -> f64 {
        match self {
            Self::Annual => 1.0,
            Self::Semiannual => 2.0,
            Self::Quarterly => 4.0,
        }
    }

    pub(super) const fn months(self) -> i64 {
        match self {
            Self::Annual => 12,
            Self::Semiannual => 6,
            Self::Quarterly => 3,
        }
    }
}
