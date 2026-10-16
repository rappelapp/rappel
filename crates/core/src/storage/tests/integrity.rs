//! Integrity.

use super::*;

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
