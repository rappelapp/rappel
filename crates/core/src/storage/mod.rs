//! SQLite storage: opening the database safely, migrations and backups.
//!
//! See ADR 0005 for the decisions behind this module.

mod backup;
mod migrations;
mod schedule;

use std::fs::{self, OpenOptions, Permissions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::data_dir::ensure_data_dir;
use crate::error::{Error, Result};
use crate::time::now_ms;
use backup::{KEEP_BACKUPS, check_rows, create_backup, has_backup_for_day, prune_backups};
use migrations::{MIGRATIONS, Migration, latest, migrate};

/// File name of the database inside the data directory.
pub const DB_FILE_NAME: &str = "cards.sqlite3";

/// Directory name for backups inside the data directory.
pub const BACKUP_DIR_NAME: &str = "backups";

const FILE_MODE: u32 = 0o600;

/// An open Cards database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (or creates) the database in `data_dir`, checks it, backs it up
    /// when needed and migrates it to the current schema.
    ///
    /// In order:
    /// 1. The directory is created with mode `0700` and the database file
    ///    with `0600` (an existing one is tightened, with its `-wal`/`-shm`).
    /// 2. `PRAGMA integrity_check`, then the version check. A damaged file,
    ///    a database newer than this build, or a non-Cards database is
    ///    refused before anything is written.
    /// 3. WAL mode, `synchronous=FULL`, foreign keys on.
    /// 4. If migrations are pending on an existing database, a backup is
    ///    made first; if it fails, nothing is migrated. Otherwise one is
    ///    made if none exists for today (UTC); that one failing only logs a
    ///    warning. The 14 newest backups are kept, never deleting the one
    ///    just made.
    /// 5. Pending migrations run, one transaction each.
    ///
    /// # Errors
    /// [`Error::IntegrityCheckFailed`], [`Error::UnsupportedVersion`],
    /// [`Error::NotACardsDatabase`],
    /// [`Error::Migration`], [`Error::ForeignKeyViolation`], backup errors,
    /// and IO or SQLite errors.
    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with(
            &data_dir.join(DB_FILE_NAME),
            &data_dir.join(BACKUP_DIR_NAME),
            now_ms(),
            MIGRATIONS,
        )
    }

    /// [`Store::open`] with every input explicit, for tests.
    fn open_with(
        db_path: &Path,
        backup_dir: &Path,
        now_ms: i64,
        migrations: &[Migration],
    ) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            ensure_data_dir(parent)?;
        }
        prepare_private_file(db_path)?;

        let mut conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Connection-only settings; none of them write to the file.
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "trusted_schema", false)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        check_integrity(&conn)?;

        // Read-only checks, before anything that writes to the file (even
        // switching to WAL rewrites header bytes).
        let supported = latest(migrations);
        let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let current = u32::try_from(found)
            .ok()
            .filter(|&v| v <= supported)
            .ok_or(Error::UnsupportedVersion { found, supported })?;
        let is_new = current == 0 && is_empty(&conn)?;
        if current == 0 && !is_new {
            // Migration 0001 sets user_version in the same transaction that
            // creates the tables, so this is some other application's file.
            return Err(Error::NotACardsDatabase);
        }

        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::PragmaRejected {
                pragma: "journal_mode",
                value: mode,
            });
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
        let foreign_keys: bool = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
        if !foreign_keys {
            return Err(Error::PragmaRejected {
                pragma: "foreign_keys",
                value: "0".into(),
            });
        }

        if !is_new {
            let fresh_backup = if current < supported {
                // No migration without a backup: failure aborts the open.
                let path = create_backup(&conn, backup_dir, now_ms)?;
                tracing::info!(path = %path.display(), "database backed up before migrating");
                Some(path)
            } else {
                // A failed daily backup must not lock the user out of a
                // healthy database.
                daily_backup(&conn, backup_dir, now_ms).unwrap_or_else(|err| {
                    tracing::warn!("daily backup failed: {err}");
                    None
                })
            };
            if let Err(err) = prune_backups(backup_dir, KEEP_BACKUPS, fresh_backup.as_deref()) {
                tracing::warn!("pruning old backups failed: {err}");
            }
        }

        migrate(&mut conn, migrations, current)?;
        Ok(Self { conn })
    }

    /// The schema version of the open database.
    ///
    /// # Errors
    /// If SQLite fails to read it.
    pub fn schema_version(&self) -> Result<u32> {
        let v: u32 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        Ok(v)
    }
}

/// Creates `path` with mode `0600` if missing (never truncates), and sets
/// `0600` on it and on any existing `-wal`, `-shm` or `-journal` file.
/// SQLite gives the files it creates next to the database the database
/// file's mode, so they stay private too.
fn prepare_private_file(path: &Path) -> Result<()> {
    let io_err = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .open(path)
        .map_err(io_err(path))?;

    let mut paths = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    for p in paths {
        match fs::metadata(&p) {
            Ok(meta) if meta.permissions().mode() & 0o7777 != FILE_MODE => {
                fs::set_permissions(&p, Permissions::from_mode(FILE_MODE)).map_err(io_err(&p))?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_err(&p)(err)),
        }
    }
    Ok(())
}

/// Runs `PRAGMA integrity_check`. A file SQLite cannot even read as a
/// database counts as a failed check.
fn check_integrity(conn: &Connection) -> Result<()> {
    let messages = match check_rows(conn, "integrity_check") {
        Ok(rows) => rows,
        Err(err)
            if matches!(
                err.sqlite_error_code(),
                Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
            ) =>
        {
            vec![err.to_string()]
        }
        Err(err) => return Err(err.into()),
    };
    if messages == ["ok"] {
        Ok(())
    } else {
        tracing::error!(problems = messages.len(), "database integrity check failed");
        Err(Error::IntegrityCheckFailed { messages })
    }
}

/// Makes a backup unless one exists from today (UTC). Returns its path if
/// one was made.
fn daily_backup(conn: &Connection, backup_dir: &Path, now_ms: i64) -> Result<Option<PathBuf>> {
    if has_backup_for_day(backup_dir, now_ms)? {
        return Ok(None);
    }
    let path = create_backup(conn, backup_dir, now_ms)?;
    tracing::info!(path = %path.display(), "daily database backup");
    Ok(Some(path))
}

/// Whether the database has no schema at all (a file SQLite just created).
fn is_empty(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
    Ok(n == 0)
}

#[cfg(test)]
mod tests;
