use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, Neg as _, Sub, SubAssign};
use std::time::{Duration, Instant};

use str0m_proto::Bitrate;
use str0m_proto::DataSize;

/// Wrapper for [`Instant`] that provides additional time points in the past or future.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Timestamp {
    /// A time in the past that already happened.
    DistantPast,

    /// An exact instant.
    Exact(Instant),

    /// A time in the future that will never happen.
    DistantFuture,
}

/// Wrapper for [`Duration`] that can be negative and provides a duration to a
/// distant future or past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimeDelta {
    /// Time delta to some event in distant past that already happened.
    NegativeInfinity,

    /// An exact negative duration.
    Negative(Duration),

    /// An exact positive duration.
    Positive(Duration),

    /// Time delta to some event in distant future that will never happen.
    PositiveInfinity,
}

impl TimeDelta {
    pub(super) const ZERO: Self = Self::Positive(Duration::ZERO);

    /// Returns the number of seconds contained by this [`TimeDelta`] as `f64`.
    pub fn as_secs_f64(&self) -> f64 {
        match self {
            Self::NegativeInfinity => f64::NEG_INFINITY,
            Self::Negative(d) => d.as_secs_f64().neg(),
            Self::Positive(d) => d.as_secs_f64(),
            Self::PositiveInfinity => f64::INFINITY,
        }
    }
}

impl Add<TimeDelta> for Timestamp {
    type Output = Self;

    fn add(self, rhs: TimeDelta) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, TimeDelta::PositiveInfinity) => Self::DistantFuture,
            (Self::DistantPast, _) | (_, TimeDelta::NegativeInfinity) => Self::DistantPast,
            (Self::Exact(i), TimeDelta::Negative(d)) => Self::Exact(i - d),
            (Self::Exact(i), TimeDelta::Positive(d)) => Self::Exact(i + d),
        }
    }
}

impl Sub<TimeDelta> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: TimeDelta) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, TimeDelta::NegativeInfinity) => Self::DistantFuture,
            (Self::DistantPast, _) | (_, TimeDelta::PositiveInfinity) => Self::DistantPast,
            (Self::Exact(i), TimeDelta::Negative(d)) => Self::Exact(i + d),
            (Self::Exact(i), TimeDelta::Positive(d)) => Self::Exact(i - d),
        }
    }
}

impl Sub<Self> for Timestamp {
    type Output = TimeDelta;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, Self::DistantPast) => TimeDelta::PositiveInfinity,
            (Self::DistantPast, _) | (_, Self::DistantFuture) => TimeDelta::NegativeInfinity,
            (Self::Exact(this), Self::Exact(that)) => match this.cmp(&that) {
                Ordering::Less => TimeDelta::Negative(that - this),
                Ordering::Equal => TimeDelta::ZERO,
                Ordering::Greater => TimeDelta::Positive(this - that),
            },
        }
    }
}

impl Add<Duration> for Timestamp {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        self + TimeDelta::from(rhs)
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        self - TimeDelta::from(rhs)
    }
}

impl Sub<Instant> for Timestamp {
    type Output = TimeDelta;

    fn sub(self, rhs: Instant) -> Self::Output {
        self.sub(Self::from(rhs))
    }
}

impl SubAssign<TimeDelta> for Timestamp {
    fn sub_assign(&mut self, rhs: TimeDelta) {
        *self = *self - rhs;
    }
}

impl AddAssign<TimeDelta> for Timestamp {
    fn add_assign(&mut self, rhs: TimeDelta) {
        *self = *self + rhs;
    }
}

impl SubAssign<Duration> for Timestamp {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

impl AddAssign<Duration> for Timestamp {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::DistantPast, Self::DistantPast) => Ordering::Equal,
            (Self::DistantPast, _) => Ordering::Less,
            (_, Self::DistantPast) => Ordering::Greater,
            (Self::DistantFuture, Self::DistantFuture) => Ordering::Equal,
            (Self::DistantFuture, _) => Ordering::Greater,
            (_, Self::DistantFuture) => Ordering::Less,
            (Self::Exact(v1), Self::Exact(v2)) => v1.cmp(v2),
        }
    }
}

impl From<Instant> for Timestamp {
    fn from(value: Instant) -> Self {
        Self::Exact(value)
    }
}

impl Add<Self> for TimeDelta {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::PositiveInfinity, _) | (_, Self::PositiveInfinity) => Self::PositiveInfinity,
            (Self::NegativeInfinity, _) | (_, Self::NegativeInfinity) => Self::NegativeInfinity,
            (Self::Negative(this), Self::Negative(that)) => Self::Negative(this + that),
            (Self::Positive(this), Self::Positive(that)) => Self::Positive(this + that),
            (Self::Positive(this), Self::Negative(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Negative(that - this),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Positive(this - that),
            },
            (Self::Negative(this), Self::Positive(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Positive(that - this),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Negative(this - that),
            },
        }
    }
}

impl Sub<Self> for TimeDelta {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::PositiveInfinity, _) | (_, Self::NegativeInfinity) => Self::PositiveInfinity,
            (Self::NegativeInfinity, _) | (_, Self::PositiveInfinity) => Self::NegativeInfinity,
            (Self::Positive(this), Self::Negative(that)) => Self::Positive(this + that),
            (Self::Negative(this), Self::Positive(that)) => Self::Negative(this + that),
            (Self::Positive(this), Self::Positive(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Negative(that - this),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Positive(this - that),
            },
            (Self::Negative(this), Self::Negative(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Positive(that - this),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Negative(this - that),
            },
        }
    }
}

impl PartialOrd for TimeDelta {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for TimeDelta {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::NegativeInfinity, Self::NegativeInfinity) => Ordering::Equal,
            (Self::NegativeInfinity, _) => Ordering::Less,
            (_, Self::NegativeInfinity) => Ordering::Greater,
            (Self::PositiveInfinity, Self::PositiveInfinity) => Ordering::Equal,
            (Self::PositiveInfinity, _) => Ordering::Greater,
            (_, Self::PositiveInfinity) => Ordering::Less,
            (Self::Negative(_), Self::Positive(_)) => Ordering::Less,
            (Self::Positive(_), Self::Negative(_)) => Ordering::Greater,
            (Self::Positive(this), Self::Positive(that)) => this.cmp(that),
            (Self::Negative(this), Self::Negative(that)) => that.cmp(this),
        }
    }
}

impl PartialEq<Duration> for TimeDelta {
    fn eq(&self, other: &Duration) -> bool {
        *self == Self::from(*other)
    }
}

impl PartialOrd<Duration> for TimeDelta {
    fn partial_cmp(&self, other: &Duration) -> Option<Ordering> {
        Some(Self::cmp(self, &Self::from(*other)))
    }
}

impl SubAssign<Self> for TimeDelta {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl AddAssign<Self> for TimeDelta {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Div<u32> for TimeDelta {
    type Output = Self;

    #[inline]
    fn div(self, rhs: u32) -> Self {
        match self {
            Self::NegativeInfinity | Self::PositiveInfinity => self,
            Self::Negative(duration) => Self::Negative(duration / rhs),
            Self::Positive(duration) => Self::Positive(duration / rhs),
        }
    }
}

impl From<Duration> for TimeDelta {
    fn from(value: Duration) -> Self {
        Self::Positive(value)
    }
}

impl Div<TimeDelta> for DataSize {
    type Output = Bitrate;

    fn div(self, rhs: TimeDelta) -> Self::Output {
        let bytes = self.as_bytes_f64();
        let s = rhs.as_secs_f64();

        if s == 0.0 {
            return Bitrate::ZERO;
        }

        let bps = (bytes * 8.0) / s;

        bps.into()
    }
}

impl fmt::Display for TimeDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeDelta::NegativeInfinity => write!(f, "-Inf"),
            TimeDelta::Negative(v) => write!(f, "-{:.03}", v.as_secs_f32()),
            TimeDelta::Positive(v) => write!(f, "{:.03}", v.as_secs_f32()),
            TimeDelta::PositiveInfinity => write!(f, "+Inf"),
        }
    }
}
