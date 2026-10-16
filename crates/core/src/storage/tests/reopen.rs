//! Re-open.

use super::*;

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
    assert_eq!(e.backups(), ["rappel-20261015-123457.sqlite3"]);

    // Same day again: no new backup.
    drop(e.open(DAY0 + 60_000).unwrap());
    assert_eq!(e.backups().len(), 1);

    // Next day: one more.
    drop(e.open(DAY0 + DAY_MS).unwrap());
    assert_eq!(
        e.backups(),
        [
            "rappel-20261015-123457.sqlite3",
            "rappel-20261016-123456.sqlite3"
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
    assert_eq!(backups.last().unwrap(), "rappel-20261104-123456.sqlite3");
}

#[test]
fn a_concurrent_store_needs_an_existing_migrated_database() {
    let e = env();
    // Missing: refused, and nothing is created.
    assert!(matches!(
        Store::open_concurrent_with(&e.db, &e.backups, MIGRATIONS),
        Err(Error::Sqlite(_))
    ));
    assert!(!e.db.exists());

    // Older schema: only `Store::open` may migrate it.
    let older = e
        .open_with(DAY0, &MIGRATIONS[..MIGRATIONS.len() - 1])
        .unwrap();
    let found = older.schema_version().unwrap();
    assert!(matches!(
        Store::open_concurrent_with(&e.db, &e.backups, MIGRATIONS),
        Err(Error::NotMigrated { found: f, supported }) if f == found && supported == found + 1
    ));
    drop(older);

    let main = e.open(DAY0).unwrap();
    let backups = e.backups();
    let second = Store::open_concurrent_with(&e.db, &e.backups, MIGRATIONS).unwrap();
    assert_eq!(
        second.schema_version().unwrap(),
        main.schema_version().unwrap()
    );
    let fk: bool = second
        .conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .unwrap();
    assert!(fk);
    // Opening it makes no backup.
    assert_eq!(e.backups(), backups);
}
