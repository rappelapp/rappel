//! Tests for `Store::open`: fresh database, re-open, migrations, backups,
//! integrity failures and the append-only rule.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::*;
use crate::domain::{CardState, Phase, Schedule};
use crate::scheduling::Scheduler;

/// 2026-10-15T12:34:56.789Z
pub(super) const DAY0: i64 = 1_792_067_696_789;
pub(super) const DAY_MS: i64 = 86_400_000;

pub(super) const DECK: &str = "01920000-0000-7000-8000-000000000001";
pub(super) const CARD: &str = "01920000-0000-7000-8000-000000000002";
pub(super) const CARD2: &str = "01920000-0000-7000-8000-000000000003";
const EVENT: &str = "01920000-0000-7000-8000-000000000004";

struct Env {
    _tmp: tempfile::TempDir,
    db: PathBuf,
    backups: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("data").join(DB_FILE_NAME);
    let backups = tmp.path().join("data").join(BACKUP_DIR_NAME);
    Env {
        _tmp: tmp,
        db,
        backups,
    }
}

impl Env {
    fn open(&self, now: i64) -> Result<Store> {
        Store::open_with(&self.db, &self.backups, now, MIGRATIONS)
    }

    fn open_with(&self, now: i64, migrations: &[Migration]) -> Result<Store> {
        Store::open_with(&self.db, &self.backups, now, migrations)
    }

    fn backups(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.backups) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

pub(super) fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub(super) fn insert_sample(conn: &Connection) {
    conn.execute_batch(&format!(
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('{DECK}', 'Maths', 'maths', 1, 1);
         INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
             VALUES ('{CARD}', '{DECK}', 'front', 'back', 1, 1);
         INSERT INTO review_events VALUES ('{EVENT}', '{CARD}', 2, 'pass', 1500);
         INSERT INTO card_schedule VALUES ('{CARD}', 'learning', 3, 1.5, 5.0, 2, 1, 0);"
    ))
    .unwrap();
}

pub(super) fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

// --- Fresh database -------------------------------------------------------

#[test]
fn fresh_database_is_created_private_migrated_and_not_backed_up() {
    let e = env();
    let store = e.open(DAY0).unwrap();

    assert_eq!(store.schema_version().unwrap(), latest(MIGRATIONS));
    assert_eq!(
        tables(&store.conn),
        ["card_schedule", "cards", "decks", "review_events"]
    );
    assert_eq!(mode(&e.db), 0o600);
    assert_eq!(mode(e.db.parent().unwrap()), 0o700);

    let journal: String = store
        .conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    let fk: bool = store
        .conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .unwrap();
    assert!(fk);
    let sync: i64 = store
        .conn
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 2, "synchronous=FULL");

    assert!(
        e.backups().is_empty(),
        "nothing to back up in a new database"
    );
}

#[test]
fn wal_and_shm_files_are_private() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    for suffix in ["-wal", "-shm"] {
        assert_eq!(mode(&sidecar(&e.db, suffix)), 0o600, "{suffix}");
    }
}

#[test]
fn existing_loose_permissions_are_tightened() {
    let e = env();
    drop(e.open(DAY0).unwrap());
    fs::set_permissions(&e.db, fs::Permissions::from_mode(0o644)).unwrap();
    drop(e.open(DAY0).unwrap());
    assert_eq!(mode(&e.db), 0o600);
}

// --- Re-open --------------------------------------------------------------

#[test]
fn reopen_keeps_data_and_backs_up_once_per_utc_day() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    drop(store);

    // Same day: the first open of an existing database makes today's backup.
    let store = e.open(DAY0 + 1000).unwrap();
    assert_eq!(count(&store.conn, "review_events"), 1);
    assert_eq!(store.schema_version().unwrap(), latest(MIGRATIONS));
    drop(store);
    assert_eq!(e.backups(), ["cards-20261015-123457.sqlite3"]);

    // Same day again: no new backup.
    drop(e.open(DAY0 + 60_000).unwrap());
    assert_eq!(e.backups().len(), 1);

    // Next day: one more.
    drop(e.open(DAY0 + DAY_MS).unwrap());
    assert_eq!(
        e.backups(),
        [
            "cards-20261015-123457.sqlite3",
            "cards-20261016-123456.sqlite3"
        ]
    );
}

#[test]
fn backups_are_pruned_to_fourteen() {
    let e = env();
    drop(e.open(DAY0).unwrap());
    for day in 1..=20 {
        drop(e.open(DAY0 + day * DAY_MS).unwrap());
    }
    let backups = e.backups();
    assert_eq!(backups.len(), KEEP_BACKUPS);
    assert_eq!(backups.last().unwrap(), "cards-20261104-123456.sqlite3");
}

// --- Migrations -----------------------------------------------------------

/// For every real migration N: a database at N-1 (built by the real code
/// with only the first N-1 migrations) opens and reaches N. This covers
/// "migrate a DB created by the previous version" for every future
/// migration too; migrations that transform data add their own tests.
#[test]
fn every_migration_upgrades_the_previous_version() {
    for n in 1..=MIGRATIONS.len() {
        let e = env();
        let prev = e.open_with(DAY0, &MIGRATIONS[..n - 1]).unwrap();
        assert_eq!(prev.schema_version().unwrap() as usize, n - 1);
        drop(prev);

        let store = e.open_with(DAY0, &MIGRATIONS[..n]).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, n);
        assert_eq!(check_rows(&store.conn, "integrity_check").unwrap(), ["ok"]);
    }
}

#[test]
fn migration_0001_applies_to_an_existing_empty_file() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    drop(Connection::open(&e.db).unwrap());
    assert_eq!(fs::metadata(&e.db).unwrap().len(), 0);

    let store = e.open(DAY0).unwrap();
    assert_eq!(store.schema_version().unwrap(), latest(MIGRATIONS));
    insert_sample(&store.conn);
}

pub(super) const FAKE_V1: Migration = Migration {
    version: 1,
    name: "fake v1",
    sql: "CREATE TABLE t(x INTEGER NOT NULL) STRICT;",
    rebuild_schedule: false,
    backfill: None,
};
pub(super) const FAKE_V2: Migration = Migration {
    version: 2,
    name: "fake v2",
    sql: "ALTER TABLE t ADD COLUMN y INTEGER NOT NULL DEFAULT 7;",
    rebuild_schedule: false,
    backfill: None,
};
pub(super) const FAILING_V2: Migration = Migration {
    version: 2,
    name: "failing v2",
    sql: "CREATE TABLE u(x INTEGER) STRICT; INSERT INTO missing VALUES (1);",
    rebuild_schedule: false,
    backfill: None,
};
const FK_VIOLATING_V2: Migration = Migration {
    version: 2,
    name: "fk-violating v2",
    sql: "PRAGMA defer_foreign_keys = ON;
          CREATE TABLE p(id INTEGER PRIMARY KEY) STRICT;
          CREATE TABLE c(p INTEGER REFERENCES p(id)) STRICT;
          INSERT INTO c VALUES (99);",
    rebuild_schedule: false,
    backfill: None,
};

fn db_at_fake_v1(e: &Env) {
    let store = e.open_with(DAY0, &[FAKE_V1]).unwrap();
    store
        .conn
        .execute_batch("INSERT INTO t VALUES (1), (2);")
        .unwrap();
}

#[test]
fn upgrade_keeps_data_and_backs_up_the_old_version_first() {
    let e = env();
    db_at_fake_v1(&e);

    let store = e.open_with(DAY0, &[FAKE_V1, FAKE_V2]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let sum: i64 = store
        .conn
        .query_row("SELECT sum(x + y) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sum, 1 + 7 + 2 + 7);

    // The backup holds the database as it was before the migration.
    let backups = e.backups();
    assert_eq!(backups.len(), 1);
    let old = Connection::open(e.backups.join(&backups[0])).unwrap();
    let v: u32 = old
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(count(&old, "t"), 2);
    let has_y: bool = old
        .query_row(
            "SELECT count(*) FROM pragma_table_info('t') WHERE name = 'y'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!has_y);
}

#[test]
fn failed_migration_rolls_back_and_keeps_the_backup() {
    let e = env();
    db_at_fake_v1(&e);

    let err = e.open_with(DAY0, &[FAKE_V1, FAILING_V2]).unwrap_err();
    assert!(matches!(err, Error::Migration { version: 2, .. }), "{err}");

    let conn = Connection::open(&e.db).unwrap();
    let v: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(tables(&conn), ["t"], "table u was rolled back");
    assert_eq!(count(&conn, "t"), 2);
    assert_eq!(e.backups().len(), 1);
}

#[test]
fn foreign_key_violation_rolls_back_the_migration() {
    let e = env();
    db_at_fake_v1(&e);

    let err = e.open_with(DAY0, &[FAKE_V1, FK_VIOLATING_V2]).unwrap_err();
    assert!(
        matches!(err, Error::ForeignKeyViolation { version: 2 }),
        "{err}"
    );
    let conn = Connection::open(&e.db).unwrap();
    assert_eq!(tables(&conn), ["t"]);
}

#[test]
fn newer_database_is_refused_untouched() {
    let e = env();
    drop(e.open(DAY0).unwrap());
    {
        // Not in WAL mode, like a backup copied back by hand: switching it
        // to WAL would rewrite the header, so this catches any write that
        // happens before the version check.
        let conn = Connection::open(&e.db).unwrap();
        conn.pragma_update_and_check(None, "journal_mode", "DELETE", |r| r.get::<_, String>(0))
            .unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
    }
    let before = fs::read(&e.db).unwrap();

    let err = e.open(DAY0 + DAY_MS).unwrap_err();
    assert!(
        matches!(
            err,
            Error::UnsupportedVersion {
                found: 99,
                supported
            } if supported == latest(MIGRATIONS)
        ),
        "{err}"
    );
    assert_eq!(fs::read(&e.db).unwrap(), before);
    assert!(!sidecar(&e.db, "-wal").exists(), "no WAL file created");
    assert!(e.backups().is_empty(), "no backup of a database we refuse");
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut p = db.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

#[test]
fn foreign_database_is_refused_untouched() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    Connection::open(&e.db)
        .unwrap()
        .execute_batch("CREATE TABLE other_app(x); INSERT INTO other_app VALUES (1);")
        .unwrap();
    let before = fs::read(&e.db).unwrap();

    assert!(matches!(e.open(DAY0), Err(Error::NotACardsDatabase)));
    assert_eq!(fs::read(&e.db).unwrap(), before);
    assert_eq!(
        e.backups(),
        Vec::<String>::new(),
        "no backup of a foreign file"
    );
}

#[test]
fn failed_pre_migration_backup_stops_the_migration() {
    let e = env();
    db_at_fake_v1(&e);
    // A regular file where the backup directory should be.
    fs::write(&e.backups, b"").unwrap();

    let err = e.open_with(DAY0, &[FAKE_V1, FAKE_V2]).unwrap_err();
    assert!(matches!(err, Error::NotADirectory(_)), "{err}");

    let conn = Connection::open(&e.db).unwrap();
    let v: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1, "not migrated without a backup");
    let leftovers: Vec<_> = fs::read_dir(e.db.parent().unwrap())
        .unwrap()
        .map(|d| d.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("cards-"))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());
}

#[test]
fn failed_daily_backup_still_opens_a_healthy_database() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    drop(store);
    fs::write(&e.backups, b"").unwrap();

    let mut store = e.open(DAY0 + DAY_MS).unwrap();
    assert_eq!(count(&store.conn, "review_events"), 1);
    assert_eq!(store.schema_version().unwrap(), latest(MIGRATIONS));
    let warnings = store.take_open_warnings();
    assert!(
        matches!(warnings.as_slice(), [OpenWarning::DailyBackupFailed(_)]),
        "one warning for the failed backup, not a second for the prune: {warnings:?}"
    );
    assert!(store.take_open_warnings().is_empty(), "taken once");
}

#[test]
fn healthy_opens_have_no_warnings() {
    let e = env();
    let mut store = e.open(DAY0).unwrap();
    assert!(store.take_open_warnings().is_empty(), "new database");
    drop(store);
    let mut store = e.open(DAY0 + DAY_MS).unwrap();
    assert!(store.take_open_warnings().is_empty(), "daily backup made");
    assert_eq!(e.backups().len(), 1);
}

#[test]
fn failed_prune_is_a_warning() {
    let e = env();
    drop(e.open(DAY0).unwrap());
    drop(e.open(DAY0 + DAY_MS).unwrap());
    // Today's backup exists, so none is attempted; a stale partial backup
    // that cannot be removed (a non-empty directory) makes the prune fail.
    let stuck = e.backups.join("cards-20261001-000000.sqlite3.partial");
    fs::create_dir(&stuck).unwrap();
    fs::write(stuck.join("x"), b"").unwrap();

    let mut store = e.open(DAY0 + DAY_MS + 1000).unwrap();
    let warnings = store.take_open_warnings();
    assert!(
        matches!(warnings.as_slice(), [OpenWarning::PruneFailed(_)]),
        "{warnings:?}"
    );
}

/// The app restores on a worker thread and sends the new store back.
#[test]
fn store_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Store>();
}

#[test]
fn pre_migration_backup_survives_future_dated_backups() {
    let e = env();
    db_at_fake_v1(&e);
    fs::remove_dir_all(&e.backups).ok();
    fs::create_dir(&e.backups).unwrap();
    for day in 1..=KEEP_BACKUPS {
        fs::write(
            e.backups
                .join(format!("cards-203001{day:02}-120000.sqlite3")),
            b"",
        )
        .unwrap();
    }

    let store = e.open_with(DAY0, &[FAKE_V1, FAKE_V2]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let fresh = e.backups.join("cards-20261015-123456.sqlite3");
    let v: u32 = Connection::open(&fresh)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1, "the pre-migration backup is still there");
    assert_eq!(e.backups().len(), KEEP_BACKUPS);
}

#[test]
fn migration_that_commits_itself_is_not_recorded() {
    const COMMITTING_V2: Migration = Migration {
        version: 2,
        name: "committing v2",
        sql: "CREATE TABLE u(x INTEGER) STRICT; COMMIT;",
        rebuild_schedule: false,
        backfill: None,
    };
    let e = env();
    db_at_fake_v1(&e);
    let err = e.open_with(DAY0, &[FAKE_V1, COMMITTING_V2]).unwrap_err();
    assert!(
        matches!(err, Error::MigrationEndedTransaction { version: 2 }),
        "{err}"
    );
    let v: u32 = Connection::open(&e.db)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
}

#[test]
fn negative_user_version_is_refused() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    Connection::open(&e.db)
        .unwrap()
        .pragma_update(None, "user_version", -1)
        .unwrap();
    assert!(matches!(
        e.open(DAY0),
        Err(Error::UnsupportedVersion { found: -1, .. })
    ));
}

// --- Integrity ------------------------------------------------------------

/// Overwrites the root page of a btree with garbage, the way a bad disk
/// sector or a torn write would.
pub(super) fn corrupt_btree(db: &Path, name: &str) {
    let (root, page_size): (i64, i64) = {
        let conn = Connection::open(db).unwrap();
        let root = conn
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        let page_size = conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .unwrap();
        (root, page_size)
    };
    let mut bytes = fs::read(db).unwrap();
    let start = usize::try_from((root - 1) * page_size).unwrap();
    let end = start + usize::try_from(page_size).unwrap();
    bytes[start..end].fill(0xA5);
    fs::write(db, bytes).unwrap();
}

#[test]
fn integrity_failure_is_reported_and_nothing_is_written() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    // Closing the last connection checkpoints the WAL into the main file.
    drop(store);
    corrupt_btree(&e.db, "review_events_card_reviewed");
    let before = fs::read(&e.db).unwrap();

    match e.open(DAY0 + DAY_MS) {
        Err(Error::IntegrityCheckFailed { messages }) => {
            // `any` needs at least one message; `all` rules out the healthy "ok".
            assert!(
                messages.iter().any(|m| m != "ok") && messages.iter().all(|m| m != "ok"),
                "{messages:?}"
            );
        }
        other => panic!("expected IntegrityCheckFailed, got {other:?}"),
    }
    assert_eq!(fs::read(&e.db).unwrap(), before, "damaged file unchanged");
    assert!(e.backups().is_empty(), "no backup made of a damaged file");
}

#[test]
fn file_that_is_not_a_database_fails_the_integrity_check() {
    let e = env();
    fs::create_dir_all(e.db.parent().unwrap()).unwrap();
    fs::write(&e.db, vec![0x42; 8192]).unwrap();
    assert!(matches!(
        e.open(DAY0),
        Err(Error::IntegrityCheckFailed { .. })
    ));
    assert_eq!(fs::read(&e.db).unwrap(), vec![0x42; 8192]);
}

// --- Schema rules ---------------------------------------------------------

#[test]
fn review_events_are_append_only() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);

    for sql in [
        "UPDATE review_events SET grade = 'fail'",
        "DELETE FROM review_events",
    ] {
        let err = store.conn.execute(sql, []).unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
    assert_eq!(count(&store.conn, "review_events"), 1);
}

#[test]
fn card_with_reviews_cannot_be_deleted() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    assert!(
        store
            .conn
            .execute("DELETE FROM cards WHERE id = ?1", [CARD])
            .is_err()
    );
    assert!(
        store
            .conn
            .execute("DELETE FROM decks WHERE id = ?1", [DECK])
            .is_err(),
        "deck with cards"
    );
}

#[test]
fn deleting_a_card_without_reviews_drops_its_cached_schedule() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    store
        .conn
        .execute_batch(&format!(
            "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                 VALUES ('{CARD2}', '{DECK}', 'f', 'b', 1, 1);
             INSERT INTO card_schedule VALUES ('{CARD2}', 'learning', 3, 1.0, 5.0, 2, 1, 0);
             DELETE FROM cards WHERE id = '{CARD2}';"
        ))
        .unwrap();
    assert_eq!(count(&store.conn, "card_schedule"), 1);
}

#[test]
fn schema_rejects_bad_values() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    for sql in [
        // Grade outside Fail/Pass.
        format!(
            "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '{CARD}', 5, 'good', 0)"
        ),
        // Negative duration.
        format!(
            "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '{CARD}', 5, 'pass', -1)"
        ),
        // Id not a lowercase 36-character UUID.
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('short', 'x', 'x', 1, 1)".to_owned(),
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('01920000-0000-7000-8000-00000000000A', 'x', 'x', 1, 1)".to_owned(),
        // STRICT: text in an integer column.
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('01920000-0000-7000-8000-00000000000b', 'x', 'x', 'now', 1)"
            .to_owned(),
        // Unknown card.
        "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '01920000-0000-7000-8000-0000000000ff', 5, 'pass', 0)".to_owned(),
    ] {
        assert!(store.conn.execute(&sql, []).is_err(), "{sql}");
    }
}

// --- Schedule cache ---------------------------------------------------------

/// Inserts review events for `card` at `DAY0 + offsets`, ids from `first_id`.
pub(super) fn insert_events(conn: &Connection, card: &str, first_id: u32, reviews: &[(i64, &str)]) {
    for (i, (offset, grade)) in (first_id..).zip(reviews) {
        conn.execute(
            "INSERT INTO review_events VALUES (?1, ?2, ?3, ?4, 1000)",
            rusqlite::params![
                format!("01920000-0000-7000-8000-1{i:011}"),
                card,
                DAY0 + offset,
                grade
            ],
        )
        .unwrap();
    }
}

const STEP: i64 = crate::scheduling::STEP_MS;
pub(super) const REVIEWS: [(i64, &str); 4] = [
    (0, "fail"),
    (STEP, "pass"),
    (3 * DAY_MS, "fail"),
    (3 * DAY_MS + STEP, "pass"),
];

/// A database written by v1 of the schema (cards, review events and a
/// cache row in the old shape) upgrades to v2: the log is kept and the
/// cache is rebuilt from it in the new shape.
#[test]
fn migration_0002_rebuilds_the_schedule_cache_from_the_log() {
    let e = env();
    {
        let v1 = e.open_with(DAY0, &MIGRATIONS[..1]).unwrap();
        v1.conn
            .execute_batch(&format!(
                "INSERT INTO decks (id, name, created_at, updated_at) VALUES ('{DECK}', 'Maths', 1, 1);
                 INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                     VALUES ('{CARD}', '{DECK}', 'front', 'back', 1, 1),
                            ('{CARD2}', '{DECK}', 'new card', 'back', 1, 1);
                 INSERT INTO card_schedule VALUES ('{CARD}', 3, 1.5, 5.0, 2, 1, 0);"
            ))
            .unwrap();
        insert_events(&v1.conn, CARD, 0, &REVIEWS);
    }

    let store = e.open_with(DAY0 + 4 * DAY_MS, &MIGRATIONS[..2]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    assert_eq!(count(&store.conn, "cards"), 2);
    assert_eq!(count(&store.conn, "review_events"), 4);

    let events = store.review_events(CARD).unwrap();
    let expected = Scheduler::default().rebuild_state(CARD, &events).unwrap();
    assert!(matches!(
        expected,
        CardState::Scheduled(Schedule {
            phase: Phase::Review,
            reps: 4,
            lapses: 1,
            ..
        })
    ));
    assert_eq!(store.card_state(CARD).unwrap(), expected);
    let CardState::Scheduled(cached) = expected else {
        unreachable!("matched above")
    };
    let last = DAY0 + 3 * DAY_MS + STEP;
    assert_eq!(cached.last_review_at, last);
    assert_eq!((cached.due_at - last) % DAY_MS, 0);
    assert!(cached.due_at >= last + DAY_MS);
    assert_eq!(store.card_state(CARD2).unwrap(), CardState::New);
    assert_eq!(count(&store.conn, "card_schedule"), 1);
}

/// A migration whose cache rebuild fails is rolled back as a whole.
#[test]
fn failed_schedule_rebuild_rolls_back_the_migration() {
    // Fake v1 has no review_events table, so the rebuild cannot run.
    const REBUILDING_V2: Migration = Migration {
        version: 2,
        name: "rebuilding v2",
        sql: "CREATE TABLE u(x INTEGER) STRICT;",
        rebuild_schedule: true,
        backfill: None,
    };
    let e = env();
    db_at_fake_v1(&e);
    let err = e.open_with(DAY0, &[FAKE_V1, REBUILDING_V2]).unwrap_err();
    assert!(
        matches!(err, Error::MigrationRebuild { version: 2, .. }),
        "{err}"
    );
    let conn = Connection::open(&e.db).unwrap();
    let v: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(tables(&conn), ["t"], "u was rolled back");
}

/// A database written by schema v2 (a deck, two cards, one reviewed)
/// upgrades to v3: everything is live, rows, log and cache are unchanged,
/// and the new operations work on it.
#[test]
fn migration_0003_keeps_everything_live_and_unchanged() {
    let e = env();
    let (events, state) = {
        let v2 = e.open_with(DAY0, &MIGRATIONS[..2]).unwrap();
        v2.conn
            .execute_batch(&format!(
                "INSERT INTO decks VALUES ('{DECK}', 'Maths', 1, 1);
                 INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                     VALUES ('{CARD}', '{DECK}', 'front', 'back', 1, 1),
                            ('{CARD2}', '{DECK}', 'new card', 'back', 1, 1);"
            ))
            .unwrap();
        insert_events(&v2.conn, CARD, 0, &REVIEWS);
        let mut v2 = v2;
        v2.rebuild_schedule_cache(&Scheduler::default()).unwrap();
        (
            v2.review_events(CARD).unwrap(),
            v2.card_state(CARD).unwrap(),
        )
    };

    let store = e.open_with(DAY0 + 4 * DAY_MS, &MIGRATIONS[..3]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
    let live: i64 = store
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM decks WHERE deleted_at IS NULL)
                  + (SELECT count(*) FROM cards WHERE deleted_at IS NULL)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, 3);
    assert_eq!(store.review_events(CARD).unwrap(), events);
    assert_eq!(store.card_state(CARD).unwrap(), state);
    assert_eq!(store.card_state(CARD2).unwrap(), CardState::New);

    // The operations target the current schema: upgrade the rest of the
    // way, then use them on the data written at v2.
    drop(store);
    let mut store = e.open(DAY0 + 4 * DAY_MS).unwrap();
    let decks = store.list_decks(DAY0 + 400 * DAY_MS).unwrap();
    assert_eq!(decks.len(), 1);
    assert_eq!((decks[0].new_count, decks[0].due_count), (1, 1));
    assert!(matches!(
        store.create_deck("Maths"),
        Err(Error::DuplicateDeckName(_))
    ));
    // New events sort after the ones written before the upgrade.
    let e = store
        .record_review(CARD, crate::domain::Grade::Pass, std::time::Duration::ZERO)
        .unwrap();
    assert_eq!(store.review_events(CARD).unwrap().last(), Some(&e));
}

/// Two live decks with the same name cannot exist in a v2 database written
/// by Cards (it had no way to create decks), but if they do, migration
/// 0003 fails and rolls back, leaving the database at v2.
#[test]
fn migration_0003_with_duplicate_deck_names_rolls_back() {
    let e = env();
    {
        let v2 = e.open_with(DAY0, &MIGRATIONS[..2]).unwrap();
        v2.conn
            .execute_batch(&format!(
                "INSERT INTO decks VALUES ('{DECK}', 'Maths', 1, 1);
                 INSERT INTO decks VALUES ('{CARD}', 'Maths', 1, 1);"
            ))
            .unwrap();
    }
    let err = e.open_with(DAY0, &MIGRATIONS[..3]).unwrap_err();
    assert!(matches!(err, Error::Migration { version: 3, .. }), "{err}");
    let store = e.open_with(DAY0, &MIGRATIONS[..2]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    assert_eq!(count(&store.conn, "decks"), 2);
}

/// A v3 database's decks, live and deleted, get their name keys from
/// migration 0004, which computes them in Rust (NFC + lowercase).
#[test]
fn migration_0004_backfills_name_keys_for_every_deck() {
    let e = env();
    {
        let v3 = e.open_with(DAY0, &MIGRATIONS[..3]).unwrap();
        v3.conn
            .execute_batch(&format!(
                "INSERT INTO decks (id, name, created_at, updated_at, deleted_at)
                     VALUES ('{DECK}', 'E\u{301}conomie', 1, 1, NULL),
                            ('{CARD}', '\u{C9}conomie', 1, 1, 5),
                            ('{CARD2}', 'MATHS', 1, 1, NULL);"
            ))
            .unwrap();
    }
    let store = e.open_with(DAY0, &MIGRATIONS[..4]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 4);
    let keys: Vec<(String, String)> = store
        .conn
        .prepare("SELECT id, name_key FROM decks ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        keys,
        [
            (DECK.to_owned(), "\u{E9}conomie".to_owned()),
            (CARD.to_owned(), "\u{E9}conomie".to_owned()),
            (CARD2.to_owned(), "maths".to_owned()),
        ]
    );
    // Names themselves are kept as typed.
    let name: String = store
        .conn
        .query_row("SELECT name FROM decks WHERE id = ?1", [DECK], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "E\u{301}conomie");
    // A key can no longer be cleared.
    let err = store
        .conn
        .execute("UPDATE decks SET name_key = NULL WHERE id = ?1", [DECK])
        .unwrap_err();
    assert!(err.to_string().contains("name_key is required"), "{err}");
}

/// Two live decks whose names only differ in case make migration 0004 fail
/// and roll back; the database stays at v3, and its backup is there.
#[test]
fn migration_0004_with_names_differing_only_in_case_rolls_back() {
    let e = env();
    {
        let v3 = e.open_with(DAY0, &MIGRATIONS[..3]).unwrap();
        v3.conn
            .execute_batch(&format!(
                "INSERT INTO decks (id, name, created_at, updated_at)
                     VALUES ('{DECK}', 'Maths', 1, 1), ('{CARD}', 'maths', 1, 1);"
            ))
            .unwrap();
    }
    let err = e.open_with(DAY0, &MIGRATIONS[..4]).unwrap_err();
    assert!(
        matches!(err, Error::MigrationBackfill { version: 4, .. }),
        "{err}"
    );
    assert_eq!(e.backups().len(), 1);
    let store = e.open_with(DAY0, &MIGRATIONS[..3]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
    assert_eq!(count(&store.conn, "decks"), 2);
    // The SQL half (index swap, new column, triggers) was undone too.
    let objects = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        objects("SELECT count(*) FROM pragma_table_info('decks') WHERE name = 'name_key'"),
        0
    );
    assert_eq!(
        objects("SELECT count(*) FROM sqlite_schema WHERE name = 'decks_live_name'"),
        1
    );
    assert_eq!(
        objects(
            "SELECT count(*) FROM sqlite_schema
             WHERE name IN ('decks_live_name_key', 'decks_name_key_required_on_insert',
                            'decks_name_key_required_on_update')"
        ),
        0
    );
}

/// Migration 0005 rebuilds cache rows written by the scheduler before the
/// interval floor (here: a stale row with a too-early due date).
#[test]
fn migration_0005_rebuilds_the_schedule_cache() {
    let e = env();
    {
        let v4 = e.open_with(DAY0, &MIGRATIONS[..4]).unwrap();
        insert_sample(&v4.conn);
    }
    let store = e.open_with(DAY0, &MIGRATIONS[..5]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 5);
    let events = store.review_events(CARD).unwrap();
    let expected = Scheduler::default().rebuild_state(CARD, &events).unwrap();
    assert_eq!(store.card_state(CARD).unwrap(), expected);
    // The sample's cache row (due at 3 ms) was replaced.
    assert_ne!(scheduled_due(&store), 3);
}

fn scheduled_due(store: &Store) -> i64 {
    match store.card_state(CARD).unwrap() {
        CardState::Scheduled(s) => s.due_at,
        CardState::New => panic!("expected a schedule"),
    }
}

#[test]
fn rebuild_schedule_cache_matches_rebuild_state_and_is_idempotent() {
    let e = env();
    let mut store = e.open(DAY0).unwrap();
    store
        .conn
        .execute_batch(&format!(
            "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('{DECK}', 'Maths', 'maths', 1, 1);
             INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                 VALUES ('{CARD}', '{DECK}', 'a', 'b', 1, 1),
                        ('{CARD2}', '{DECK}', 'c', 'd', 1, 1);"
        ))
        .unwrap();
    // Interleaved, inserted out of time order.
    insert_events(&store.conn, CARD, 0, &REVIEWS[2..]);
    insert_events(&store.conn, CARD2, 10, &[(DAY_MS, "pass"), (0, "pass")]);
    insert_events(&store.conn, CARD, 20, &REVIEWS[..2]);

    let events = store.review_events(CARD).unwrap();
    let times: Vec<i64> = events.iter().map(|e| e.reviewed_at - DAY0).collect();
    assert_eq!(times, REVIEWS.map(|r| r.0), "oldest first");

    let scheduler = Scheduler::new(0.85).unwrap();
    assert_eq!(store.rebuild_schedule_cache(&scheduler).unwrap(), 2);
    let first = [
        store.card_state(CARD).unwrap(),
        store.card_state(CARD2).unwrap(),
    ];
    for (card, cached) in [CARD, CARD2].iter().zip(&first) {
        let events = store.review_events(card).unwrap();
        assert_eq!(*cached, scheduler.rebuild_state(card, &events).unwrap());
    }

    assert_eq!(store.rebuild_schedule_cache(&scheduler).unwrap(), 2);
    let second = [
        store.card_state(CARD).unwrap(),
        store.card_state(CARD2).unwrap(),
    ];
    assert_eq!(first, second);
}

#[test]
fn schedule_cache_rejects_bad_values() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    store.conn.execute("DELETE FROM card_schedule", []).unwrap();
    for (phase, reps, lapses) in [("new", 1, 0), ("review", 0, 0), ("review", 1, -1)] {
        let sql = format!(
            "INSERT INTO card_schedule VALUES ('{CARD}', '{phase}', 3, 1.0, 5.0, 2, {reps}, {lapses})"
        );
        assert!(store.conn.execute(&sql, []).is_err(), "{sql}");
    }
}
