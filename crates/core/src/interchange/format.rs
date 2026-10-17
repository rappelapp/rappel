//! The `cards.json` document (format version 1), as serde types.
//!
//! Field order here is the order in the file, and every list is written
//! sorted by id, so a database always gives the same bytes (ADR 0017).
//! Reading denies unknown fields: a file that does not match exactly is
//! refused rather than half understood.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The format version this build writes, and the newest it reads.
pub const FORMAT_VERSION: u32 = 1;

/// Only the version, read before the rest so that a file from a newer
/// version is refused as such, not for its unknown fields.
#[derive(Debug, Deserialize)]
pub(super) struct Header {
    pub format_version: i64,
}

/// The whole `cards.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    pub format_version: u32,
    /// When the export was made, UTC milliseconds. The only field that
    /// differs between two exports of the same database.
    pub exported_at: i64,
    pub decks: Vec<DeckRecord>,
    pub cards: Vec<CardRecord>,
    pub review_events: Vec<EventRecord>,
    /// Settings are not exported in version 1 (ADR 0027); must be empty.
    pub settings: BTreeMap<String, serde_json::Value>,
    /// No media exist yet; must be empty in version 1.
    pub media: Vec<serde_json::Value>,
}

/// A `decks` row, deleted ones included. `name_key` is not exported: it
/// is derived from `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeckRecord {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Required, though it may be `null`: serde would otherwise read a
    /// missing field as `null`, which would make a deleted row live.
    #[serde(deserialize_with = "Option::deserialize")]
    pub deleted_at: Option<i64>,
}

/// A `cards` row, deleted ones included. `seq` is internal and not
/// exported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CardRecord {
    pub id: String,
    pub deck_id: String,
    pub front_md: String,
    pub back_md: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Required, though it may be `null`: serde would otherwise read a
    /// missing field as `null`, which would make a deleted row live.
    #[serde(deserialize_with = "Option::deserialize")]
    pub deleted_at: Option<i64>,
}

/// A `review_events` row. The grade stays a string here and is checked by
/// validation, so a bad grade is reported with its place in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventRecord {
    pub id: String,
    pub card_id: String,
    pub reviewed_at: i64,
    pub grade: String,
    pub duration_ms: i64,
}
