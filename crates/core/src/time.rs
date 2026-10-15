//! Time as Cards stores it: UTC milliseconds since the Unix epoch, in `i64`.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current time in UTC milliseconds since the Unix epoch. A clock set
/// before 1970 gives a negative value; one beyond `i64` range saturates.
#[must_use]
pub fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

/// A UTC date and time of day, broken into fields (proleptic Gregorian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcParts {
    /// Year, e.g. 2026. Can be zero or negative for very old times.
    pub year: i64,
    /// Month, 1–12.
    pub month: i64,
    /// Day of month, 1–31.
    pub day: i64,
    /// Hour, 0–23.
    pub hour: i64,
    /// Minute, 0–59.
    pub minute: i64,
    /// Second, 0–59 (Unix time has no leap seconds).
    pub second: i64,
}

/// Splits UTC milliseconds into date and time fields. Milliseconds are
/// dropped (rounded towards the past, also before 1970).
#[must_use]
pub fn utc_parts(ms: i64) -> UtcParts {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    UtcParts {
        year,
        month,
        day,
        hour: secs_of_day / 3600,
        minute: secs_of_day % 3600 / 60,
        second: secs_of_day % 60,
    }
}

/// Days since 1970-01-01 to (year, month, day).
///
/// Howard Hinnant's algorithm (<https://howardhinnant.github.io/date_algorithms.html>):
/// shift the year to start on 1 March so the leap day is last, then split
/// into 400-year eras of 146 097 days.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468; // days since 0000-03-01
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era, [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of March-based year, [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
        let p = utc_parts(ms);
        (p.year, p.month, p.day, p.hour, p.minute, p.second)
    }

    #[test]
    fn epoch() {
        assert_eq!(parts(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn known_dates() {
        // Values from `date -u -d @<secs> +%s`.
        assert_eq!(parts(951_782_400_000), (2000, 2, 29, 0, 0, 0));
        assert_eq!(parts(1_792_067_696_789), (2026, 10, 15, 12, 34, 56));
        assert_eq!(parts(4_107_542_399_999), (2100, 2, 28, 23, 59, 59));
        assert_eq!(parts(4_107_542_400_000), (2100, 3, 1, 0, 0, 0));
    }

    #[test]
    fn before_epoch_rounds_towards_the_past() {
        assert_eq!(parts(-1), (1969, 12, 31, 23, 59, 59));
        assert_eq!(parts(-86_400_000), (1969, 12, 31, 0, 0, 0));
        assert_eq!(parts(-2_208_988_800_000), (1900, 1, 1, 0, 0, 0));
    }

    /// Inverse of `civil_from_days`, written independently for the test.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    #[test]
    fn days_round_trip_and_are_consecutive() {
        let mut prev = civil_from_days(-800_000);
        for days in -799_999..800_000 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
            assert!((1..=12).contains(&m) && (1..=31).contains(&d));
            // Each day is the next day after the previous one.
            let next_day = (prev.0, prev.1, prev.2 + 1);
            let next_month = (prev.0, prev.1 + 1, 1);
            let next_year = (prev.0 + 1, 1, 1);
            assert!(
                (y, m, d) == next_day || (y, m, d) == next_month || (y, m, d) == next_year,
                "{prev:?} -> {:?}",
                (y, m, d)
            );
            prev = (y, m, d);
        }
    }

    #[test]
    fn now_is_after_this_code_was_written() {
        assert!(now_ms() > 1_792_000_000_000);
    }
}
