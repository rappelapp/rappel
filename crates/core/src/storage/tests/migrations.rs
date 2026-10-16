//! Migrations.

use super::*;

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
    assert!(!with_suffix(&e.db, "-wal").exists(), "no WAL file created");
    assert!(e.backups().is_empty(), "no backup of a database we refuse");
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

    assert!(matches!(e.open(DAY0), Err(Error::NotARappelDatabase)));
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
    let stuck = e.backups.join("rappel-20261001-000000.sqlite3.partial");
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
                .join(format!("rappel-203001{day:02}-120000.sqlite3")),
            b"",
        )
        .unwrap();
    }

    let store = e.open_with(DAY0, &[FAKE_V1, FAKE_V2]).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let fresh = e.backups.join("rappel-20261015-123456.sqlite3");
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
