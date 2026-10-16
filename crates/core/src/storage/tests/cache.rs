//! Schedule cache.

use super::*;

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
    let decks = store
        .list_decks(DAY0 + 400 * DAY_MS, DAY0 + 400 * DAY_MS)
        .unwrap();
    assert_eq!(decks.len(), 1);
    assert_eq!((decks[0].new_count, decks[0].due_count), (1, 1));
    assert!(matches!(
        store.create_deck("Maths"),
        Err(Error::DuplicateDeckName(_))
    ));
    // New events sort after the ones written before the upgrade.
    let e = store
        .record_review(
            CARD,
            crate::domain::Grade::Pass,
            std::time::Duration::ZERO,
            crate::time::now_ms(),
        )
        .unwrap();
    assert_eq!(store.review_events(CARD).unwrap().last(), Some(&e));
}

/// Two live decks with the same name cannot exist in a v2 database written
/// by Rappel (it had no way to create decks), but if they do, migration
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

/// The log of a hard card: graduated, then eight lapses each relearned on
/// time (difficulty up, stability down), then a Pass `late_days` after the
/// last review. Offsets from `DAY0`, oldest first.
fn hard_card_log(late_days: i64) -> Vec<(i64, &'static str)> {
    let mut log = vec![(0, "pass"), (STEP, "pass")];
    let mut t = STEP;
    for _ in 0..8 {
        t += DAY_MS;
        log.push((t, "fail"));
        t += STEP;
        log.push((t, "pass"));
    }
    log.push((t + late_days * DAY_MS, "pass"));
    log
}

/// Migration 0005 moves a due date that the pre-0005 scheduler set below
/// the floor of ADR 0009: the cache row is what that scheduler wrote (FSRS
/// interval, fuzzed, no floor), and after the migration the card is due at
/// least `elapsed + 1` days after the late Pass.
#[test]
fn migration_0005_moves_the_due_date_of_a_late_pass() {
    use crate::scheduling::fuzz::{floored_fuzzed_days, fuzz_factor};

    const LATE_DAYS: i64 = 30;
    let e = env();
    let log = hard_card_log(LATE_DAYS);
    let scheduler = Scheduler::default();
    {
        let v4 = e.open_with(DAY0, &MIGRATIONS[..4]).unwrap();
        v4.conn
            .execute_batch(&format!(
                "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('{DECK}', 'Maths', 'maths', 1, 1);
                 INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                     VALUES ('{CARD}', '{DECK}', 'a', 'b', 1, 1);"
            ))
            .unwrap();
        insert_events(&v4.conn, CARD, 0, &log);
        let events = v4.review_events(CARD).unwrap();
        let (last, before_last) = events.split_last().unwrap();
        let prev = match scheduler.rebuild_state(CARD, before_last).unwrap() {
            CardState::Scheduled(s) => s,
            CardState::New => panic!("expected a schedule"),
        };
        assert_eq!(prev.phase, Phase::Review);
        let now = match scheduler.rebuild_state(CARD, &events).unwrap() {
            CardState::Scheduled(s) => s,
            CardState::New => panic!("expected a schedule"),
        };
        // What the scheduler wrote before ADR 0009: the same memory, and
        // FSRS's interval fuzzed with no floor.
        let old_days = floored_fuzzed_days(
            f64::from(scheduler.interval(now.stability)),
            1,
            fuzz_factor(CARD, prev.reps),
        );
        let floor_days = LATE_DAYS + 1;
        assert!(
            i64::from(old_days) < floor_days,
            "the fixture must be below the floor: {old_days} days"
        );
        let old_due = last.reviewed_at + i64::from(old_days) * DAY_MS;
        v4.conn
            .execute(
                "INSERT INTO card_schedule VALUES (?1, 'review', ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    CARD,
                    old_due,
                    f64::from(now.stability),
                    f64::from(now.difficulty),
                    now.last_review_at,
                    now.reps,
                    now.lapses
                ],
            )
            .unwrap();
        assert_eq!(scheduled_due(&v4), old_due);
    }

    let store = e.open_with(DAY0, &MIGRATIONS[..5]).unwrap();

    let events = store.review_events(CARD).unwrap();
    let last_review = events.last().unwrap().reviewed_at;
    let due = scheduled_due(&store);
    assert!(
        due >= last_review + (LATE_DAYS + 1) * DAY_MS,
        "moved to at least the floor"
    );
    assert_eq!(
        store.card_state(CARD).unwrap(),
        scheduler.rebuild_state(CARD, &events).unwrap()
    );
}

/// A real file-backed store, closed and opened again (a restart), keeps
/// issuing increasing ids and keeps its cache equal to the rebuild.
#[test]
fn a_reopened_file_store_keeps_ids_increasing_and_the_cache_exact() {
    use std::time::Duration;

    use crate::domain::Grade;

    let tmp = tempfile::tempdir().unwrap();
    let (first, card_a, card_b) = {
        let mut store = Store::open(tmp.path()).unwrap();
        let deck = store.create_deck("Maths").unwrap().id;
        let a = store.add_card(&deck, "a", "1").unwrap().id;
        let b = store.add_card(&deck, "b", "2").unwrap().id;
        let first = store
            .record_review(&a, Grade::Fail, Duration::from_secs(2), DAY0)
            .unwrap();
        (first, a, b)
    };

    let mut store = Store::open(tmp.path()).unwrap();
    // The clock went back across the restart: a fresh generator that did
    // not observe the log would stamp this id before `first`'s.
    let second = store
        .record_review(&card_b, Grade::Pass, Duration::from_secs(2), DAY0 - 1_000)
        .unwrap();
    assert_eq!(
        second.reviewed_at,
        DAY0 - 1_000,
        "another card: not floored"
    );
    let third = store
        .record_review(&card_a, Grade::Pass, Duration::from_secs(2), DAY0 + STEP)
        .unwrap();

    assert!(
        first.id < second.id && second.id < third.id,
        "ids increase across the reopen"
    );
    let scheduler = Scheduler::default();
    for card in [&card_a, &card_b] {
        let events = store.review_events(card).unwrap();
        assert_eq!(
            store.card_state(card).unwrap(),
            scheduler.rebuild_state(card, &events).unwrap()
        );
    }
    assert_eq!(store.review_events(&card_a).unwrap(), [first, third]);
    assert_eq!(store.review_events(&card_b).unwrap(), [second]);
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
