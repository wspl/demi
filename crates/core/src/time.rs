//! Times in JSON: an RFC 3339 string in UTC with millisecond precision
//! (`storage.md` § Encodings and digests), and the clock wall time is read
//! from.

use std::{borrow::Cow, fmt, str::FromStr};

use jiff::fmt::temporal::DateTimePrinter;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Prints `2026-09-21T14:13:20.000Z`: UTC, always three fractional digits,
/// so that the text of two times orders as the times do.
const PRINTER: DateTimePrinter = DateTimePrinter::new().precision(Some(3));

/// A moment in whole milliseconds, which JSON writes as
/// `2026-09-21T14:13:20.000Z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

/// Why a text or a count is not a [`Timestamp`].
#[derive(Debug, thiserror::Error)]
pub enum TimestampError {
    #[error("not an RFC 3339 time: {0}")]
    Format(jiff::Error),
    #[error("{0} is outside the supported range of times")]
    Range(i64),
    #[error("a time is whole milliseconds; {0} is finer")]
    Precision(String),
}

impl Timestamp {
    /// 1970-01-01T00:00:00.000Z.
    pub const UNIX_EPOCH: Timestamp = Timestamp(jiff::Timestamp::UNIX_EPOCH);

    /// The moment `millisecond` milliseconds after the Unix epoch.
    pub fn from_millisecond(millisecond: i64) -> Result<Self, TimestampError> {
        jiff::Timestamp::from_millisecond(millisecond)
            .map(Self)
            .map_err(|_| TimestampError::Range(millisecond))
    }

    /// The moment `time` names, without the digits finer than a millisecond.
    pub fn truncate(time: jiff::Timestamp) -> Self {
        let millisecond = time.as_millisecond();
        // Cutting a time jiff holds to its millisecond stays inside jiff's
        // range, so the fallback is never taken.
        Self(jiff::Timestamp::from_millisecond(millisecond).unwrap_or(time))
    }

    /// Milliseconds since the Unix epoch.
    pub fn as_millisecond(self) -> i64 {
        self.0.as_millisecond()
    }

    pub fn to_jiff(self) -> jiff::Timestamp {
        self.0
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(time: Timestamp) -> Self {
        time.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&PRINTER.timestamp_to_string(&self.0))
    }
}

impl FromStr for Timestamp {
    type Err = TimestampError;

    fn from_str(text: &str) -> Result<Self, TimestampError> {
        let time: jiff::Timestamp = text.parse().map_err(TimestampError::Format)?;
        if time.subsec_nanosecond() % 1_000_000 != 0 {
            return Err(TimestampError::Precision(text.to_owned()));
        }
        Ok(Self(time))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<'de, str>::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

impl JsonSchema for Timestamp {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Timestamp".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "format": "date-time" })
    }
}

/// Where wall-clock time is read: the times records and answers carry. It is
/// injected so that a test can fix it (`concurrency.md` § Tests and time);
/// durations measured in memory use Tokio's clock instead.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The system's clock, cut to the millisecond.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::truncate(jiff::Timestamp::now())
    }
}
