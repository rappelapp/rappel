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
    let local = glib::DateTime::from_unix_utc(now_ms.div_euclid(1000))?.to_timezone(tz)?;
    let at_four = |date: &glib::DateTime| {
        let (year, month, day) = date.ymd();
        glib::DateTime::new(tz, year, month, day, DAY_START_HOUR, 0, 0.0)
    };
    let mut start = at_four(&local)?;
    if start.to_unix().saturating_mul(1000) > now_ms {
        // The calendar day before, then 04:00 of it: right across a
        // daylight-saving change, unlike subtracting 24 hours.
        start = at_four(&local.add_days(-1)?)?;
    }
    Ok(start.to_unix().saturating_mul(1000))
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
}
