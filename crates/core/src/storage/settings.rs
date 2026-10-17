//! Reading and writing the global settings (ADR 0027).

use std::collections::HashMap;

use rusqlite::{Connection, TransactionBehavior, params};

use super::Store;
use super::schedule::{invalid, write_schedule_cache};
use crate::domain::Settings;
use crate::error::Result;
use crate::scheduling::Scheduler;

const RETENTION: &str = "desired_retention";
const MAX_NEW: &str = "max_new_per_day";
const MAX_REVIEWS: &str = "max_reviews_per_day";
const DAY_START: &str = "day_start_hour";

impl Store {
    /// The settings; a key never written has its default.
    ///
    /// # Errors
    /// SQLite errors, or [`Error::InvalidColumn`](crate::Error::InvalidColumn)
    /// for a stored value that is not valid for its key.
    pub fn settings(&self) -> Result<Settings> {
        settings_in(&self.conn)
    }

    /// Sets the most new cards each deck introduces per study day.
    ///
    /// # Errors
    /// [`Error::InvalidSetting`](crate::Error::InvalidSetting) above
    /// `MAX_DAILY_LIMIT`; SQLite errors.
    pub fn set_max_new_per_day(&mut self, value: u32) -> Result<()> {
        write(
            &self.conn,
            MAX_NEW,
            &Settings::check_max_new(value)?.to_string(),
        )
    }

    /// Sets the most due reviews each deck shows per study day.
    ///
    /// # Errors
    /// [`Error::InvalidSetting`](crate::Error::InvalidSetting) above
    /// `MAX_DAILY_LIMIT`; SQLite errors.
    pub fn set_max_reviews_per_day(&mut self, value: u32) -> Result<()> {
        write(
            &self.conn,
            MAX_REVIEWS,
            &Settings::check_max_reviews(value)?.to_string(),
        )
    }

    /// Sets the local hour at which a study day begins.
    ///
    /// # Errors
    /// [`Error::InvalidSetting`](crate::Error::InvalidSetting) above 23;
    /// SQLite errors.
    pub fn set_day_start_hour(&mut self, value: u8) -> Result<()> {
        write(
            &self.conn,
            DAY_START,
            &Settings::check_day_start_hour(value)?.to_string(),
        )
    }

    /// Sets the desired retention and rebuilds the whole `card_schedule`
    /// cache from the review log with it, in one transaction: either both
    /// happen or neither. Every card's due date may change; the log does
    /// not. Does nothing if the value is already the stored one.
    ///
    /// # Errors
    /// [`Error::InvalidSetting`](crate::Error::InvalidSetting) outside
    /// 0.80–0.97; scheduler and SQLite errors, with the old value and
    /// cache left as they were.
    pub fn set_desired_retention(&mut self, value: f32) -> Result<()> {
        let scheduler = Scheduler::new(value)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // An unreadable stored settings table must not stop the user from
        // writing a good value over it, so only a successful read can say
        // "unchanged".
        if settings_in(&tx).is_ok_and(|s| s.desired_retention.to_bits() == value.to_bits()) {
            return Ok(());
        }
        write(&tx, RETENTION, &value.to_string())?;
        let cards = write_schedule_cache(&tx, &scheduler)?;
        tx.commit()?;
        tracing::info!(cards, "desired retention changed, schedule cache rebuilt");
        Ok(())
    }
}

/// The settings on `conn`, the defaults for keys never written and for a
/// database that has no settings table yet (a migration's rebuild).
pub(crate) fn settings_in(conn: &Connection) -> Result<Settings> {
    let mut settings = Settings::default();
    if !table_exists(conn)? {
        return Ok(settings);
    }
    let mut stored: HashMap<String, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (key, value) = row?;
        stored.insert(key, value);
    }
    let bad = |key: &str, value: &str| invalid("settings.value", &format!("{key} = {value:?}"));
    if let Some(v) = stored.get(RETENTION) {
        settings.desired_retention = v
            .parse()
            .ok()
            .and_then(|n| Settings::check_retention(n).ok())
            .ok_or_else(|| bad(RETENTION, v))?;
    }
    if let Some(v) = stored.get(MAX_NEW) {
        settings.max_new_per_day = v
            .parse()
            .ok()
            .and_then(|n| Settings::check_max_new(n).ok())
            .ok_or_else(|| bad(MAX_NEW, v))?;
    }
    if let Some(v) = stored.get(MAX_REVIEWS) {
        settings.max_reviews_per_day = v
            .parse()
            .ok()
            .and_then(|n| Settings::check_max_reviews(n).ok())
            .ok_or_else(|| bad(MAX_REVIEWS, v))?;
    }
    if let Some(v) = stored.get(DAY_START) {
        settings.day_start_hour = v
            .parse()
            .ok()
            .and_then(|n| Settings::check_day_start_hour(n).ok())
            .ok_or_else(|| bad(DAY_START, v))?;
    }
    Ok(settings)
}

/// A scheduler with the stored desired retention. Read inside each
/// transaction that schedules, so every connection uses the committed value.
pub(crate) fn scheduler_in(conn: &Connection) -> Result<Scheduler> {
    Scheduler::new(settings_in(conn)?.desired_retention)
}

fn table_exists(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
        [],
        |r| r.get(0),
    )?)
}

fn write(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
