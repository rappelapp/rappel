//! When "today" began for studying: 4 a.m. local time (ADR 0014).
//!
//! Core has no time zones (ADR 0006). The app computes the day start with
//! `GLib`, which knows the local zone and its daylight-saving rules, and
//! passes it to core as a UTC timestamp.

use gtk::glib;

/// The local hour at which a study day begins. Studying past midnight
/// still counts as the evening's day.
pub const DAY_START_HOUR: i32 = 4;

/// The start of the study day containing `now_ms` in time zone `tz`, UTC
/// milliseconds: 04:00 local of today if that has passed, else of
/// yesterday.
///
/// # Errors
/// If `GLib` cannot represent the date (far outside any real clock).
pub fn day_start_ms(now_ms: i64, tz: &glib::TimeZone) -> Result<i64, glib::BoolError> {
    let local = local_time(now_ms, tz)?;
    let mut start = at_four(&local, tz)?;
    if start.to_unix().saturating_mul(1000) > now_ms {
        // The calendar day before, then 04:00 of it: right across a
        // daylight-saving change, unlike subtracting 24 hours.
        start = at_four(&local.add_days(-1)?, tz)?;
    }
    Ok(start.to_unix().saturating_mul(1000))
}

/// The start of the study day after the one containing `now_ms` in time
/// zone `tz`, UTC milliseconds: 04:00 local of the next calendar day.
/// That day may be 23 or 25 hours away across a daylight-saving change.
///
/// # Errors
/// If `GLib` cannot represent the date (far outside any real clock).
pub fn next_day_start_ms(now_ms: i64, tz: &glib::TimeZone) -> Result<i64, glib::BoolError> {
    let start = local_time(day_start_ms(now_ms, tz)?, tz)?;
    let next = at_four(&start.add_days(1)?, tz)?;
    Ok(next.to_unix().saturating_mul(1000))
}

/// `ms` (UTC milliseconds) as a date and time in `tz`.
fn local_time(ms: i64, tz: &glib::TimeZone) -> Result<glib::DateTime, glib::BoolError> {
    glib::DateTime::from_unix_utc(ms.div_euclid(1000))?.to_timezone(tz)
}

/// 04:00 in `tz` on the calendar day of `date`.
fn at_four(date: &glib::DateTime, tz: &glib::TimeZone) -> Result<glib::DateTime, glib::BoolError> {
    let (year, month, day) = date.ymd();
    glib::DateTime::new(tz, year, month, day, DAY_START_HOUR, 0, 0.0)
}

/// The day start for now in the local time zone. If `GLib` fails, which
/// needs a clock far outside any real date, 04:00 UTC is used instead.
pub fn local_day_start_ms(now_ms: i64) -> i64 {
    day_start_ms(now_ms, &glib::TimeZone::local()).unwrap_or_else(|err| {
        tracing::warn!("cannot compute the local day start, using UTC: {err}");
        let offset = i64::from(DAY_START_HOUR) * 3_600_000;
        now_ms - (now_ms - offset).rem_euclid(86_400_000)
    })
}

/// The next day start after now in the local time zone. If `GLib` fails,
/// 24 hours after [`local_day_start_ms`] is used instead.
pub fn local_next_day_start_ms(now_ms: i64) -> i64 {
    next_day_start_ms(now_ms, &glib::TimeZone::local()).unwrap_or_else(|err| {
        tracing::warn!("cannot compute the next local day start: {err}");
        local_day_start_ms(now_ms) + 86_400_000
    })
}

#[cfg(test)]
mod tests {
    use cards_core::time::{UtcParts, utc_ms};

    use super::*;

    fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        utc_ms(UtcParts {
            year,
            month,
            day,
            hour,
            minute,
            second: 0,
        })
        .unwrap()
    }

    /// Central Europe as a POSIX rule, so the test needs no tzdata: CET
    /// (UTC+1), CEST (UTC+2) from the last Sunday of March 02:00 to the
    /// last Sunday of October 03:00.
    fn paris() -> glib::TimeZone {
        // `new` falls back to UTC on a bad identifier; the expected values
        // below differ from UTC, so that would fail the tests.
        glib::TimeZone::new(Some("CET-1CEST,M3.5.0,M10.5.0/3"))
    }

    #[test]
    fn the_day_starts_at_four_local_time() {
        let tz = glib::TimeZone::from_offset(-5 * 3600); // UTC−5
        // 2026-10-15 03:59 local = 08:59 UTC: still the 14th's day.
        assert_eq!(
            day_start_ms(utc(2026, 10, 15, 8, 59), &tz).unwrap(),
            (utc(2026, 10, 14, 9, 0))
        );
        // 04:00 local = 09:00 UTC: the 15th's day begins.
        assert_eq!(
            day_start_ms(utc(2026, 10, 15, 9, 0), &tz).unwrap(),
            (utc(2026, 10, 15, 9, 0))
        );
        // 23:30 local on the 15th = 04:30 UTC on the 16th.
        assert_eq!(
            day_start_ms(utc(2026, 10, 16, 4, 30), &tz).unwrap(),
            (utc(2026, 10, 15, 9, 0))
        );
    }

    #[test]
    fn milliseconds_before_four_belong_to_the_previous_day() {
        let tz = glib::TimeZone::utc();
        let four = utc(2026, 1, 2, 4, 0);
        assert_eq!(
            day_start_ms(four - 1, &tz).unwrap(),
            (utc(2026, 1, 1, 4, 0))
        );
        assert_eq!(day_start_ms(four, &tz).unwrap(), (four));
    }

    #[test]
    fn daylight_saving_changes_keep_four_local() {
        let tz = paris();
        // 2026-03-29: clocks go from 02:00 CET to 03:00 CEST. 04:00 CEST
        // is 02:00 UTC.
        assert_eq!(
            day_start_ms(utc(2026, 3, 29, 8, 0), &tz).unwrap(),
            (utc(2026, 3, 29, 2, 0))
        );
        // The next night, 03:00 CEST = 01:00 UTC: still the 29th's day.
        assert_eq!(
            day_start_ms(utc(2026, 3, 30, 1, 0), &tz).unwrap(),
            (utc(2026, 3, 29, 2, 0))
        );
        // 2026-10-25: clocks go from 03:00 CEST back to 02:00 CET. 04:00 CET
        // is 03:00 UTC.
        assert_eq!(
            day_start_ms(utc(2026, 10, 25, 12, 0), &tz).unwrap(),
            (utc(2026, 10, 25, 3, 0))
        );
        // The day before the change, 04:00 CEST is 02:00 UTC.
        assert_eq!(
            day_start_ms(utc(2026, 10, 25, 1, 30), &tz).unwrap(),
            (utc(2026, 10, 24, 2, 0))
        );
    }

    #[test]
    fn the_local_zone_gives_a_start_within_the_last_day() {
        let now = cards_core::time::now_ms();
        let start = local_day_start_ms(now);
        assert!(start <= now && now - start < 25 * 3_600_000);
    }

    #[test]
    fn the_next_day_starts_at_four_the_next_morning() {
        let tz = glib::TimeZone::from_offset(-5 * 3600); // UTC−5
        // 03:59 local on the 15th belongs to the 14th: next start the 15th.
        assert_eq!(
            next_day_start_ms(utc(2026, 10, 15, 8, 59), &tz).unwrap(),
            utc(2026, 10, 15, 9, 0)
        );
        // 04:00 local on the 15th: next start the 16th.
        assert_eq!(
            next_day_start_ms(utc(2026, 10, 15, 9, 0), &tz).unwrap(),
            utc(2026, 10, 16, 9, 0)
        );
    }

    #[test]
    fn the_next_day_start_follows_daylight_saving_changes() {
        let tz = paris();
        // The day of 2026-03-28 (CET) ends at 04:00 CEST on the 29th,
        // 02:00 UTC: only 23 hours after it began at 03:00 UTC.
        let start = utc(2026, 3, 28, 3, 0);
        assert_eq!(day_start_ms(start + 3_600_000, &tz).unwrap(), start);
        assert_eq!(
            next_day_start_ms(start + 3_600_000, &tz).unwrap(),
            utc(2026, 3, 29, 2, 0)
        );
        // The day of 2026-10-24 (CEST) ends at 04:00 CET on the 25th,
        // 03:00 UTC: 25 hours after it began at 02:00 UTC.
        let start = utc(2026, 10, 24, 2, 0);
        assert_eq!(
            next_day_start_ms(start + 3_600_000, &tz).unwrap(),
            utc(2026, 10, 25, 3, 0)
        );
    }

    #[test]
    fn the_local_next_day_start_is_within_the_next_day() {
        let now = cards_core::time::now_ms();
        let next = local_next_day_start_ms(now);
        assert!(next > now && next - now <= 25 * 3_600_000);
        assert_eq!(local_day_start_ms(next), next);
    }
}
