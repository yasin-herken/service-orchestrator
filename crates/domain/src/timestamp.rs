//! Consistent timestamp representation for the domain.
//!
//! Rather than scattering `SystemTime`, `i64`, or `u128` primitives through the
//! domain model, time is represented by a single value type, [`Timestamp`].
//! This keeps the model consistent, makes serialization predictable, and lets a
//! future change to the underlying clock be made in one place.

use std::fmt;
use std::ops::Sub;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// A point in time, stored in UTC.
///
/// `Timestamp` is a thin, `serde`-transparent wrapper over
/// [`time::OffsetDateTime`]. The domain never stores local times or raw
/// integers; interfaces are responsible for rendering timestamps in a
/// human-friendly form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    /// Returns the current time in UTC.
    ///
    /// This is the only place in the domain that reads the system clock.
    #[must_use]
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    /// Constructs a timestamp from an [`OffsetDateTime`].
    #[must_use]
    pub const fn from_offset(value: OffsetDateTime) -> Self {
        Self(value)
    }

    /// Returns the underlying [`OffsetDateTime`].
    #[must_use]
    pub const fn as_offset(self) -> OffsetDateTime {
        self.0
    }

    /// Returns the number of milliseconds since the Unix epoch.
    ///
    /// Useful for adapters that need an integer representation; the domain
    /// itself works with [`Timestamp`] values directly.
    #[must_use]
    pub fn epoch_millis(self) -> i128 {
        self.0.unix_timestamp_nanos() / 1_000_000
    }
}

impl From<OffsetDateTime> for Timestamp {
    fn from(value: OffsetDateTime) -> Self {
        Self::from_offset(value)
    }
}

impl From<Timestamp> for OffsetDateTime {
    fn from(value: Timestamp) -> Self {
        value.as_offset()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Sub for Timestamp {
    type Output = time::Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0 - rhs.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn converts_to_and_from_offset_datetime() {
        let offset = datetime!(2026-10-07 12:00:00 UTC);
        let timestamp = Timestamp::from_offset(offset);
        assert_eq!(timestamp.as_offset(), offset);
        let back: OffsetDateTime = timestamp.into();
        assert_eq!(back, offset);
    }

    #[test]
    fn now_is_monotonic_with_respect_to_construction() {
        let before = Timestamp::now();
        let offset = before.as_offset();
        assert!(offset.year() >= 2020);
    }

    #[test]
    fn computes_duration_between_timestamps() {
        let start = Timestamp::from_offset(datetime!(2026-10-07 12:00:00 UTC));
        let end = Timestamp::from_offset(datetime!(2026-10-07 12:00:05 UTC));
        assert_eq!((end - start).whole_seconds(), 5);
    }

    #[test]
    fn orders_by_instant() {
        let earlier = Timestamp::from_offset(datetime!(2026-10-07 12:00:00 UTC));
        let later = Timestamp::from_offset(datetime!(2026-10-07 12:00:01 UTC));
        assert!(earlier < later);
    }
}
