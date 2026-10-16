//! Export and import of the whole database as a `.cards` file (SPEC §4.1,
//! ADR 0017).
//!
//! A `.cards` file is a zip archive with exactly two entries: `cards.json`
//! (decks, cards and the review log, deleted rows included) and an empty
//! `media/` directory. The same database exported at the same time always
//! gives the same bytes.
//!
//! Import is two steps. [`Store::plan_import`](crate::storage::Store::plan_import)
//! reads and checks the file and compares it with the database without
//! writing anything; [`Store::apply_import`](crate::storage::Store::apply_import)
//! backs the database up, then adds the new rows and brings back deleted
//! rows that are live in the file, in one transaction. The
//! file is untrusted: anything unexpected refuses the whole import.

mod archive;
mod export;
mod format;
mod import;
mod validate;

use std::fmt;
use std::path::PathBuf;

pub use format::FORMAT_VERSION;

/// File name extension of an export, without the dot.
pub const FILE_EXTENSION: &str = "cards";

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    /// Decks written, deleted ones included.
    pub decks: usize,
    /// Cards written, deleted ones included.
    pub cards: usize,
    /// Review events written.
    pub review_events: usize,
}

/// How the rows of one table in a file compare with the database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableCounts {
    /// Rows whose id is not in the database: they would be added.
    pub new: usize,
    /// Rows already in the database with the same content: identical, or
    /// deleted in the file but live in the database (an import never
    /// deletes).
    pub present: usize,
    /// Deleted rows of the database that are live in the file with
    /// otherwise the same content: they would be brought back. For cards,
    /// this includes the cards deleted with a revived deck that the file
    /// does not have. Always 0 for review events.
    pub revived: usize,
}

/// One reason an import cannot be applied. Ids are the file's own, already
/// checked to be `UUIDv7`; no card text is included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// The database has a deck with this id but different content (apart
    /// from one side being deleted and the other live).
    DeckDiffers(String),
    /// The database has a card with this id but different content (apart
    /// from one side being deleted and the other live).
    CardDiffers(String),
    /// The database has a review event with this id but different content.
    EventDiffers(String),
    /// A live deck in the file has the name (by
    /// [`deck_name_key`](crate::domain::deck_name_key)) of another live
    /// deck in the database.
    DeckNameTaken {
        /// The deck in the file.
        deck_id: String,
        /// The database's deck with that name.
        existing_id: String,
    },
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeckDiffers(id) => write!(f, "deck {id} differs from the one in the database"),
            Self::CardDiffers(id) => write!(f, "card {id} differs from the one in the database"),
            Self::EventDiffers(id) => {
                write!(f, "review {id} differs from the one in the database")
            }
            Self::DeckNameTaken {
                deck_id,
                existing_id,
            } => write!(
                f,
                "deck {deck_id} has the same name as deck {existing_id} in the database"
            ),
        }
    }
}

/// What an import would do, from [`ImportPlan::summary`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Decks, deleted ones included.
    pub decks: TableCounts,
    /// Cards, deleted ones included.
    pub cards: TableCounts,
    /// Review events.
    pub review_events: TableCounts,
    /// Every conflict, in file order (decks, cards, events, then names).
    /// Any conflict refuses the whole import.
    pub conflicts: Vec<Conflict>,
}

impl ImportSummary {
    /// Whether applying would change nothing (every row is already
    /// present: nothing to add, nothing to bring back).
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.decks.new == 0
            && self.cards.new == 0
            && self.review_events.new == 0
            && self.decks.revived == 0
            && self.cards.revived == 0
    }
}

/// A checked file and how it compares with the database, from
/// [`Store::plan_import`](crate::storage::Store::plan_import). Pass it to
/// [`Store::apply_import`](crate::storage::Store::apply_import) to write.
#[derive(Clone)]
pub struct ImportPlan {
    document: format::Document,
    /// The summary, and exactly which rows apply would write.
    classified: import::Classified,
}

/// Only the summary: the document holds card text, which must not reach
/// the logs (SPEC §2).
impl fmt::Debug for ImportPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportPlan")
            .field("summary", &self.classified.summary)
            .finish_non_exhaustive()
    }
}

impl ImportPlan {
    /// What the import would add, what it would bring back, what is
    /// already there, and the conflicts.
    #[must_use]
    pub fn summary(&self) -> &ImportSummary {
        &self.classified.summary
    }
}

/// What [`Store::apply_import`](crate::storage::Store::apply_import) did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Decks added.
    pub decks: usize,
    /// Cards added.
    pub cards: usize,
    /// Review events added.
    pub review_events: usize,
    /// Deleted decks brought back.
    pub revived_decks: usize,
    /// Deleted cards brought back.
    pub revived_cards: usize,
    /// The backup made just before writing; `None` if nothing was added or
    /// brought back (then nothing was written and no backup was needed).
    pub backup: Option<PathBuf>,
}

/// Why a file was refused by
/// [`Store::plan_import`](crate::storage::Store::plan_import). The
/// messages never quote the file's text, only places in it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportProblem {
    /// The file is larger than an import may be.
    #[error("the file is larger than {limit} bytes")]
    FileTooLarge {
        /// The limit, in bytes.
        limit: u64,
    },
    /// The file is not a zip archive, or a damaged one (`0` says what the
    /// zip reader found).
    #[error("not a valid .cards archive: {0}")]
    InvalidArchive(String),
    /// The archive has an entry other than `cards.json` and `media/`, or
    /// one of them twice. The name is shown escaped and shortened.
    #[error("unexpected entry {0} in the archive")]
    UnexpectedEntry(String),
    /// A required entry is missing.
    #[error("the archive has no {0} entry")]
    MissingEntry(&'static str),
    /// An entry is encrypted, compressed with a method other than Stored
    /// or Deflate, or of the wrong kind (a file where a directory belongs).
    #[error("archive entry {0} is not stored in a supported way")]
    UnsupportedEntry(&'static str),
    /// `cards.json` is larger than allowed once decompressed.
    #[error("{entry} is larger than {limit} bytes")]
    EntryTooLarge {
        /// The entry.
        entry: &'static str,
        /// The limit, in bytes.
        limit: u64,
    },
    /// `cards.json` is not valid JSON or does not have the expected shape
    /// (`0` is the parser's message, with line and column).
    #[error("cards.json is malformed: {0}")]
    Json(String),
    /// The file was written by a newer version of Cards, or has a version
    /// that never existed.
    #[error(
        "format version {found} is not supported (this version of Cards reads up to {FORMAT_VERSION})"
    )]
    UnsupportedFormatVersion {
        /// The file's `format_version`.
        found: i64,
    },
    /// A field of one row is invalid.
    #[error("{table}[{index}].{field}: {reason}")]
    InvalidField {
        /// `decks`, `cards` or `review_events`.
        table: &'static str,
        /// The row's position in its list, from 0.
        index: usize,
        /// The field.
        field: &'static str,
        /// What is wrong with it.
        reason: FieldProblem,
    },
    /// A top-level field is invalid (`exported_at`).
    #[error("{field}: {reason}")]
    InvalidTopLevel {
        /// The field.
        field: &'static str,
        /// What is wrong with it.
        reason: FieldProblem,
    },
    /// `settings` is not empty; this version knows no settings.
    #[error("the file has settings, which this version of Cards does not know")]
    UnsupportedSettings,
    /// `media` is not empty; this version cannot import media.
    #[error("the file has media, which this version of Cards cannot import")]
    UnsupportedMedia,
}

/// What is wrong with one field, for [`ImportProblem::InvalidField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FieldProblem {
    /// Not a lowercase hyphenated `UUIDv7` with the RFC variant (#12).
    #[error("not a valid id")]
    NotUuidV7,
    /// Another row of the same list has this id.
    #[error("the same id appears twice")]
    DuplicateId,
    /// No row of the referenced list has this id.
    #[error("refers to an id that is not in the file")]
    UnknownReference,
    /// Not `fail` or `pass`.
    #[error("not a grade")]
    InvalidGrade,
    /// A duration below zero.
    #[error("negative")]
    Negative,
    /// A timestamp before 1970 or after the year 9999.
    #[error("timestamp out of range")]
    TimestampOutOfRange,
    /// Empty, or only whitespace.
    #[error("empty")]
    Empty,
    /// Leading or trailing whitespace (deck names are stored trimmed).
    #[error("has leading or trailing whitespace")]
    NotTrimmed,
    /// Longer than allowed.
    #[error("too long")]
    TooLong,
    /// Has a NUL character (U+0000), which GTK cannot take.
    #[error("contains a NUL character")]
    ContainsNul,
    /// A live card whose deck is deleted, which the app never produces.
    #[error("a live card in a deleted deck")]
    LiveCardInDeletedDeck,
    /// Another live deck in the file has the same name key.
    #[error("another live deck in the file has the same name")]
    DuplicateDeckName,
}

/// Size limits for reading a file. The defaults are generous; tests lower
/// them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// The whole `.cards` file.
    pub archive: u64,
    /// `cards.json`, decompressed.
    pub json: u64,
    /// One card side.
    pub card_text: usize,
    /// One deck name.
    pub deck_name: usize,
}

impl Limits {
    /// The limits used by [`Store::plan_import`](crate::storage::Store::plan_import).
    pub(crate) const DEFAULT: Self = Self {
        archive: 256 << 20,
        json: 256 << 20,
        card_text: crate::domain::MAX_CARD_SIDE_BYTES,
        deck_name: crate::domain::MAX_DECK_NAME_BYTES,
    };
}

#[cfg(test)]
mod tests;
