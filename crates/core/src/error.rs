//! Error type shared by all of `cards-core`.

use std::io;
use std::path::PathBuf;

/// Everything that can go wrong in `cards-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No usable base directory: `CARDS_DATA_DIR` and `XDG_DATA_HOME` are
    /// unset (or unusable) and `HOME` is unset, empty or relative.
    #[error("cannot find the data directory: set HOME, XDG_DATA_HOME or CARDS_DATA_DIR")]
    NoHomeDir,

    /// `CARDS_DATA_DIR` is a relative path. It is rejected instead of being
    /// resolved against whatever the current directory happens to be.
    #[error("CARDS_DATA_DIR must be an absolute path, got {}", .0.display())]
    RelativeDataDir(PathBuf),

    /// A debug build resolved to the release data directory (or a directory
    /// inside it), which would let development touch real data.
    #[error(
        "debug builds must not use the release data directory: {} is {} or inside it",
        path.display(),
        release.display()
    )]
    DebugUsesReleaseDir {
        /// The directory the debug build would have used.
        path: PathBuf,
        /// The release data directory it collides with.
        release: PathBuf,
    },

    /// The data directory path exists but is not a directory.
    #[error("{} exists but is not a directory", .0.display())]
    NotADirectory(PathBuf),

    /// Creating a directory or setting its permissions failed.
    #[error("cannot create directory {}: {source}", path.display())]
    CreateDir {
        /// The directory that could not be created.
        path: PathBuf,
        /// The underlying IO error.
        source: io::Error,
    },

    /// A file operation on the database or a backup failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file the operation was on.
        path: PathBuf,
        /// The underlying IO error.
        source: io::Error,
    },

    /// An SQLite call failed.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// SQLite did not accept a required setting.
    #[error("database setting {pragma} could not be applied (got {value})")]
    PragmaRejected {
        /// The pragma, e.g. `journal_mode`.
        pragma: &'static str,
        /// The value SQLite reported back.
        value: String,
    },

    /// `PRAGMA integrity_check` found damage. The store refuses to open so
    /// nothing is written to a damaged file.
    #[error("database integrity check failed: {}", messages.join("; "))]
    IntegrityCheckFailed {
        /// What SQLite reported, one entry per problem.
        messages: Vec<String>,
    },

    /// The database was written by a newer version of Cards (or is not a
    /// Cards database). It is left untouched.
    #[error(
        "database version {found} is not supported (this version of Cards supports up to {supported})"
    )]
    UnsupportedVersion {
        /// `PRAGMA user_version` of the file.
        found: i64,
        /// The latest migration this build knows.
        supported: u32,
    },

    /// The file is an SQLite database with tables but no Cards schema
    /// version, so it belongs to some other application. It is left
    /// untouched.
    #[error("the file is a database, but not a Cards database")]
    NotACardsDatabase,

    /// A migration failed and was rolled back.
    #[error("migration {version} failed: {source}")]
    Migration {
        /// The migration that failed.
        version: u32,
        /// The SQLite error.
        source: rusqlite::Error,
    },

    /// A migration's SQL ended the transaction itself. Its version was not
    /// recorded; this is a bug in the migration.
    #[error("migration {version} ended its own transaction")]
    MigrationEndedTransaction {
        /// The faulty migration.
        version: u32,
    },

    /// A migration left rows that violate a foreign key. It was rolled back.
    #[error("migration {version} left foreign key violations")]
    ForeignKeyViolation {
        /// The migration that was rolled back.
        version: u32,
    },

    /// Writing a backup failed. No partial backup is left behind.
    #[error("backup to {} failed: {source}", path.display())]
    Backup {
        /// The backup file that could not be written.
        path: PathBuf,
        /// The SQLite error.
        source: rusqlite::Error,
    },

    /// The finished backup did not pass `PRAGMA quick_check`. It was deleted.
    #[error("backup {} failed its check: {}", path.display(), messages.join("; "))]
    BackupCheckFailed {
        /// The backup file that failed.
        path: PathBuf,
        /// What SQLite reported.
        messages: Vec<String>,
    },
}

/// `Result` with [`Error`] as the error type.
pub type Result<T> = std::result::Result<T, Error>;
