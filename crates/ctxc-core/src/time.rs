//! Wall-clock timestamps.
//!
//! CtxC stores time as milliseconds since the Unix epoch: it is compact in
//! SQLite, sorts correctly, and is unambiguous across platforms. Formatting is
//! implemented here rather than pulled in as a dependency, since a timestamp
//! type and RFC 3339 rendering are all the core needs.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Milliseconds since 1970-01-01T00:00:00Z, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    /// The current time, read from the system clock.
    pub fn now() -> Self {
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(delta) => delta.as_millis() as i64,
            // Clock set before the epoch; measure backwards instead of panicking.
            Err(err) => -(err.duration().as_millis() as i64),
        };
        Timestamp(millis)
    }

    /// Build a timestamp from raw epoch milliseconds.
    pub const fn from_millis(millis: i64) -> Self {
        Timestamp(millis)
    }

    /// Epoch milliseconds, as stored.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Render as RFC 3339 in UTC, e.g. `2026-08-19T09:30:00.000Z`.
    pub fn to_rfc3339(self) -> String {
        let (days, millis_of_day) = div_floor(self.0, 86_400_000);
        let (year, month, day) = civil_from_days(days);

        let millis = millis_of_day % 1_000;
        let seconds_of_day = millis_of_day / 1_000;
        let (hour, minute, second) = (
            seconds_of_day / 3_600,
            (seconds_of_day % 3_600) / 60,
            seconds_of_day % 60,
        );

        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

/// Euclidean division: quotient floors toward negative infinity so that times
/// before the epoch still land on the correct calendar day.
fn div_floor(value: i64, divisor: i64) -> (i64, i64) {
    let mut quotient = value / divisor;
    let mut remainder = value % divisor;
    if remainder < 0 {
        quotient -= 1;
        remainder += divisor;
    }
    (quotient, remainder)
}

/// Convert days since the epoch to a proleptic Gregorian date.
///
/// Hinnant's `civil_from_days`, which is exact for the whole range we can
/// represent in milliseconds.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153; // March = 0
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (86_400_000, "1970-01-02T00:00:00.000Z"),
            (946_684_800_000, "2000-01-01T00:00:00.000Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (1_700_000_000_000, "2023-11-14T22:13:20.000Z"),
            (1_700_000_000_123, "2023-11-14T22:13:20.123Z"),
        ];
        for (millis, expected) in cases {
            assert_eq!(Timestamp::from_millis(millis).to_rfc3339(), expected);
        }
    }

    #[test]
    fn formats_instants_before_the_epoch() {
        assert_eq!(
            Timestamp::from_millis(-1).to_rfc3339(),
            "1969-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn now_is_after_2020() {
        assert!(Timestamp::now().as_millis() > 1_577_836_800_000);
    }

    #[test]
    fn serializes_as_a_number() {
        let json = serde_json::to_string(&Timestamp::from_millis(42)).unwrap();
        assert_eq!(json, "42");
    }
}
