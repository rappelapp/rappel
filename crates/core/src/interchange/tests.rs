use std::fs;
use std::io::{Cursor, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proptest::prelude::*;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::archive::{JSON_ENTRY, read_cards_json, write_archive};
use super::*;
use crate::Error;
use crate::domain::Grade;
use crate::scheduling::Scheduler;
use crate::storage::{BACKUP_DIR_NAME, Store};

/// 2026-10-15T12:34:56.789Z
const T0: i64 = 1_792_067_696_789;
const DAY_MS: i64 = 86_400_000;
/// The export time used by every test, so exports compare byte for byte.
const T_EXPORT: i64 = T0 + 100 * DAY_MS;

/// A store in a temporary data directory, so it can be backed up.
struct Env {
    tmp: tempfile::TempDir,
    store: Store,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("data")).unwrap();
        Self { tmp, store }
    }

    fn backups(&self) -> usize {
        fs::read_dir(self.tmp.path().join("data").join(BACKUP_DIR_NAME)).map_or(0, Iterator::count)
    }

    fn export(&self) -> Vec<u8> {
        export(&self.store)
    }

    fn import(&mut self, bytes: &[u8]) -> crate::Result<ImportReport> {
        let plan = self.store.plan_import_bytes(bytes, &Limits::DEFAULT)?;
        self.store.apply_import_at(&plan, T_EXPORT)
    }

    fn plan(&self, bytes: &[u8]) -> crate::Result<ImportPlan> {
        self.store.plan_import_bytes(bytes, &Limits::DEFAULT)
    }
}

fn export(store: &Store) -> Vec<u8> {
    store.export_bytes_at(T_EXPORT).unwrap().0
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(&read_cards_json(bytes, &Limits::DEFAULT).unwrap()).unwrap()
}

fn archive_of(value: &Value) -> Vec<u8> {
    write_archive(&serde_json::to_vec(value).unwrap()).unwrap()
}

/// A `card_schedule` row with the floats as bits, so equality is exact.
type ScheduleRow = (String, String, i64, u64, u64, i64, i64, i64);

fn schedule_table(store: &Store) -> Vec<ScheduleRow> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT card_id, phase, due_at, stability, difficulty, last_review_at, reps, lapses
             FROM card_schedule ORDER BY card_id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, f64>(3)?.to_bits(),
            r.get::<_, f64>(4)?.to_bits(),
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

/// A small but varied database: a renamed deck, a deleted deck, edited,
/// deleted and undeleted cards, and reviews on live and deleted cards.
/// Returns the store and the ids of the live deck and its first card.
fn sample(store: &mut Store) -> (String, String) {
    let maths = store.create_deck("Maths").unwrap().id;
    store.rename_deck(&maths, "Mathématiques").unwrap();
    let physics = store.create_deck("Physics").unwrap().id;
    let q1 = store.add_card(&maths, "2 + 2 ?", "4").unwrap().id;
    let q2 = store
        .add_card(&maths, "√2 ?", "≈ 1.414 \"quoted\"\n")
        .unwrap()
        .id;
    let q3 = store.add_card(&maths, "π ?", "3.14").unwrap().id;
    let p1 = store
        .add_card(&physics, "c ?", "299 792 458 m/s")
        .unwrap()
        .id;
    store.edit_card(&q2, "√2 ≈ ?", "1.414").unwrap();
    let mut t = T0;
    for (card, grade) in [
        (&q1, Grade::Fail),
        (&q1, Grade::Pass),
        (&q2, Grade::Pass),
        (&p1, Grade::Pass),
        (&q3, Grade::Pass),
    ] {
        store
            .record_review(card, grade, Duration::from_millis(1_500), t)
            .unwrap();
        t += DAY_MS;
    }
    store.delete_card(&q3).unwrap();
    store.undelete_card(&q3).unwrap();
    store.delete_card(&q2).unwrap();
    store.delete_deck(&physics).unwrap();
    (maths, q1)
}

fn sample_env() -> (Env, String, String) {
    let mut env = Env::new();
    let (deck, card) = sample(&mut env.store);
    (env, deck, card)
}

fn refused(result: crate::Result<ImportPlan>) -> ImportProblem {
    match result {
        Err(Error::ImportRefused(problem)) => problem,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------- format

/// A hand-written file with fixed ids pins the format: importing it into
/// an empty database and exporting again gives exactly this text.
const FIXTURE: &str = r#"{
  "format_version": 1,
  "exported_at": 1800707696789,
  "decks": [
    {
      "id": "01920000-0000-7000-8000-000000000001",
      "name": "Maths",
      "created_at": 1792067696789,
      "updated_at": 1792067696789,
      "deleted_at": null
    },
    {
      "id": "01920000-0000-7000-8000-000000000002",
      "name": "Old",
      "created_at": 1792067696789,
      "updated_at": 1792067696789,
      "deleted_at": 1792067700000
    }
  ],
  "cards": [
    {
      "id": "01920000-0000-7000-8000-000000000003",
      "deck_id": "01920000-0000-7000-8000-000000000001",
      "front_md": "2 + 2 ?",
      "back_md": "4",
      "created_at": 1792067696789,
      "updated_at": 1792067696789,
      "deleted_at": null
    },
    {
      "id": "01920000-0000-7000-8000-000000000004",
      "deck_id": "01920000-0000-7000-8000-000000000002",
      "front_md": "gone",
      "back_md": "",
      "created_at": 1792067696789,
      "updated_at": 1792067696789,
      "deleted_at": 1792067700000
    }
  ],
  "review_events": [
    {
      "id": "01920000-0000-7000-8000-000000000005",
      "card_id": "01920000-0000-7000-8000-000000000003",
      "reviewed_at": 1792067696789,
      "grade": "pass",
      "duration_ms": 1200
    },
    {
      "id": "01920000-0000-7000-8000-000000000006",
      "card_id": "01920000-0000-7000-8000-000000000004",
      "reviewed_at": 1792067696789,
      "grade": "fail",
      "duration_ms": 0
    }
  ],
  "settings": {},
  "media": []
}
"#;

#[test]
fn the_fixture_round_trips_to_the_same_text() {
    let mut env = Env::new();
    let report = env
        .import(&write_archive(FIXTURE.as_bytes()).unwrap())
        .unwrap();
    assert_eq!(
        (report.decks, report.cards, report.review_events),
        (2, 2, 2)
    );
    let json = read_cards_json(&env.export(), &Limits::DEFAULT).unwrap();
    assert_eq!(String::from_utf8(json).unwrap(), FIXTURE);
    // The live card got its schedule, the deleted one too.
    assert_eq!(schedule_table(&env.store).len(), 2);
    // Deleted rows stay hidden.
    let decks = env.store.list_decks(T_EXPORT, T_EXPORT).unwrap();
    assert_eq!(decks.len(), 1);
    assert_eq!(decks[0].deck.name, "Maths");
}

#[test]
fn an_export_is_a_zip_with_exactly_cards_json_and_media() {
    let (env, _, _) = sample_env();
    let bytes = env.export();
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
    assert_eq!(names, ["cards.json", "media/"]);
    assert!(zip.by_index(1).unwrap().is_dir());
    assert_eq!(
        zip.by_index(0).unwrap().compression(),
        CompressionMethod::Deflated
    );
    let doc = json_of(&bytes);
    assert_eq!(doc["format_version"], 1);
    assert_eq!(doc["exported_at"], T_EXPORT);
    assert_eq!(doc["decks"].as_array().unwrap().len(), 2);
    assert_eq!(doc["cards"].as_array().unwrap().len(), 4);
    assert_eq!(doc["review_events"].as_array().unwrap().len(), 5);
    assert_eq!(doc["settings"], json!({}));
    assert_eq!(doc["media"], json!([]));
    // Lists are sorted by id.
    for table in ["decks", "cards", "review_events"] {
        let ids: Vec<&str> = doc[table]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert!(ids.is_sorted(), "{table}");
    }
}

#[test]
fn the_same_database_gives_the_same_bytes() {
    let (env, _, _) = sample_env();
    assert_eq!(env.export(), env.export());
    // Only the export time differs between two exports.
    let later = env.store.export_bytes_at(T_EXPORT + 1).unwrap().0;
    let (mut a, b) = (json_of(&env.export()), json_of(&later));
    a["exported_at"] = b["exported_at"].clone();
    assert_eq!(a, b);
}

#[test]
fn export_to_writes_a_private_file_that_plan_import_reads() {
    let (env, _, _) = sample_env();
    let path = env.tmp.path().join("all.cards");
    fs::write(&path, b"old").unwrap();
    let summary = env.store.export_to(&path).unwrap();
    assert_eq!(
        summary,
        ExportSummary {
            decks: 2,
            cards: 4,
            review_events: 5
        }
    );
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!partial(&path).exists());
    let plan = Env::new().store.plan_import(&path).unwrap();
    assert_eq!(plan.summary().cards.new, 4);
}

fn partial(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

#[test]
fn a_leftover_partial_file_does_not_weaken_the_export() {
    let (env, _, _) = sample_env();
    let path = env.tmp.path().join("all.cards");
    let elsewhere = env.tmp.path().join("elsewhere");
    fs::write(&elsewhere, b"keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, partial(&path)).unwrap();
    env.store.export_to(&path).unwrap();
    assert_eq!(fs::read(&elsewhere).unwrap(), b"keep");
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn a_failed_export_leaves_the_target_alone() {
    let (env, _, _) = sample_env();
    let path = env.tmp.path().join("missing-dir").join("all.cards");
    assert!(matches!(env.store.export_to(&path), Err(Error::Io { .. })));
    assert!(!path.exists());
}

// ------------------------------------------------------------ round trip

#[test]
fn export_import_export_gives_identical_bytes_and_schedules() {
    let (source, _, _) = sample_env();
    let bytes = source.export();

    let mut target = Env::new();
    let plan = target.plan(&bytes).unwrap();
    assert_eq!(
        *plan.summary(),
        ImportSummary {
            decks: TableCounts {
                new: 2,
                present: 0,
                revived: 0
            },
            cards: TableCounts {
                new: 4,
                present: 0,
                revived: 0
            },
            review_events: TableCounts {
                new: 5,
                present: 0,
                revived: 0
            },
            conflicts: vec![],
        }
    );
    let report = target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(
        (report.decks, report.cards, report.review_events),
        (2, 4, 5)
    );
    assert!(report.backup.as_ref().unwrap().exists());

    assert_eq!(target.export(), bytes);
    assert_eq!(schedule_table(&target.store), schedule_table(&source.store));
}

#[test]
fn the_longest_names_and_sides_the_app_accepts_can_be_imported() {
    use crate::domain::{MAX_CARD_SIDE_BYTES, MAX_DECK_NAME_BYTES};
    let mut source = Env::new();
    // Two-byte letters, so the limits are met in bytes, not characters.
    let name = "é".repeat(MAX_DECK_NAME_BYTES / 2);
    let side = "ü".repeat(MAX_CARD_SIDE_BYTES / 2);
    let deck = source.store.create_deck(&name).unwrap();
    source.store.add_card(&deck.id, &side, &side).unwrap();
    let bytes = export(&source.store);

    let mut target = Env::new();
    let report = target.import(&bytes).unwrap();
    assert_eq!((report.decks, report.cards), (1, 1));
    assert_eq!(export(&target.store), bytes);
}

#[test]
fn importing_the_same_file_twice_is_a_noop() {
    let (source, _, _) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    target.import(&bytes).unwrap();
    let after_first = target.export();
    let backups = target.backups();

    let plan = target.plan(&bytes).unwrap();
    assert_eq!(
        *plan.summary(),
        ImportSummary {
            decks: TableCounts {
                new: 0,
                present: 2,
                revived: 0
            },
            cards: TableCounts {
                new: 0,
                present: 4,
                revived: 0
            },
            review_events: TableCounts {
                new: 0,
                present: 5,
                revived: 0
            },
            conflicts: vec![],
        }
    );
    assert!(plan.summary().is_noop());
    let report = target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(report.backup, None);
    assert_eq!(target.export(), after_first);
    assert_eq!(target.backups(), backups);

    // Importing a database's own export into itself is a no-op too.
    let plan = source.plan(&bytes).unwrap();
    assert!(plan.summary().is_noop());
}

#[test]
fn a_later_export_adds_only_what_is_new_and_rebuilds_those_schedules() {
    let (mut source, deck, card) = sample_env();
    let older = source.export();
    let mut target = Env::new();
    target.import(&older).unwrap();

    // More history on an existing card, a new card with a review, and
    // a new deck.
    source
        .store
        .record_review(&card, Grade::Pass, Duration::from_secs(2), T0 + 30 * DAY_MS)
        .unwrap();
    let new_card = source.store.add_card(&deck, "e ?", "2.718").unwrap().id;
    source
        .store
        .record_review(
            &new_card,
            Grade::Fail,
            Duration::from_secs(3),
            T0 + 31 * DAY_MS,
        )
        .unwrap();
    source.store.create_deck("Chemistry").unwrap();
    let newer = source.export();

    let plan = target.plan(&newer).unwrap();
    assert_eq!(
        plan.summary().decks,
        TableCounts {
            new: 1,
            present: 2,
            revived: 0
        }
    );
    assert_eq!(
        plan.summary().cards,
        TableCounts {
            new: 1,
            present: 4,
            revived: 0
        }
    );
    assert_eq!(
        plan.summary().review_events,
        TableCounts {
            new: 2,
            present: 5,
            revived: 0
        }
    );
    target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(target.export(), newer);
    assert_eq!(schedule_table(&target.store), schedule_table(&source.store));
}

#[test]
fn diverged_copies_merge_with_reviews_interleaved_by_time() {
    // Two copies of one history, each reviewing the same card later at
    // different times; then each imports the other's export.
    let (mut a, _, card) = sample_env();
    let mut b = Env::new();
    b.import(&a.export()).unwrap();
    let late = T0 + 40 * DAY_MS;
    let review = |env: &mut Env, t: i64| {
        env.store
            .record_review(&card, Grade::Pass, Duration::from_secs(1), t)
            .unwrap();
    };
    review(&mut a, late);
    review(&mut b, late + DAY_MS);
    review(&mut a, late + 2 * DAY_MS);
    let (from_a, from_b) = (a.export(), b.export());
    let plan = b.plan(&from_a).unwrap();
    assert_eq!(
        plan.summary().review_events,
        TableCounts {
            new: 2,
            present: 5,
            revived: 0
        }
    );
    b.store.apply_import_at(&plan, T_EXPORT).unwrap();
    a.import(&from_b).unwrap();
    // Both now hold the same log and the same cache, equal to a rebuild.
    assert_eq!(a.export(), b.export());
    assert_eq!(schedule_table(&a.store), schedule_table(&b.store));
    let events = a.store.review_events(&card).unwrap();
    assert_eq!(events.len(), 5);
    let rebuilt = Scheduler::default().rebuild_state(&card, &events).unwrap();
    assert_eq!(a.store.card_state(&card).unwrap(), rebuilt);
}

#[test]
fn files_from_two_databases_merge() {
    let (a, _, _) = sample_env();
    let mut b = Env::new();
    let deck = b.store.create_deck("Languages").unwrap().id;
    let card = b.store.add_card(&deck, "chat", "cat").unwrap().id;
    b.store
        .record_review(&card, Grade::Pass, Duration::from_secs(1), T0)
        .unwrap();

    let mut target = Env::new();
    target.import(&a.export()).unwrap();
    target.import(&b.export()).unwrap();
    let doc = json_of(&target.export());
    assert_eq!(doc["decks"].as_array().unwrap().len(), 3);
    assert_eq!(doc["cards"].as_array().unwrap().len(), 5);
    assert_eq!(doc["review_events"].as_array().unwrap().len(), 6);
    let mut expected = schedule_table(&a.store);
    expected.extend(schedule_table(&b.store));
    expected.sort();
    assert_eq!(schedule_table(&target.store), expected);
}

#[test]
fn an_in_memory_store_refuses_to_import_without_a_backup() {
    let (source, _, _) = sample_env();
    let mut store = Store::in_memory();
    let plan = store
        .plan_import_bytes(&source.export(), &Limits::DEFAULT)
        .unwrap();
    assert!(matches!(
        store.apply_import_at(&plan, T_EXPORT),
        Err(Error::NoBackupDir)
    ));
    assert_eq!(json_of(&export(&store))["decks"], json!([]));
}

// ---------------------------------------------------------------- revive

fn live_card_ids(store: &Store, deck: &str) -> Vec<String> {
    store
        .list_cards(deck)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect()
}

#[test]
fn only_a_difference_in_deleted_at_can_be_merged() {
    use super::import::{Outcome, compare};
    assert_eq!(compare(true, None, None), Outcome::Present);
    assert_eq!(compare(true, Some(5), Some(5)), Outcome::Present);
    assert_eq!(compare(true, None, Some(5)), Outcome::Present);
    assert_eq!(compare(true, Some(5), None), Outcome::Revive(5));
    assert_eq!(compare(true, Some(5), Some(6)), Outcome::Conflict);
    for (stored, file) in [
        (None, None),
        (Some(5), None),
        (None, Some(5)),
        (Some(5), Some(5)),
    ] {
        assert_eq!(compare(false, stored, file), Outcome::Conflict);
    }
}

#[test]
fn export_delete_all_import_revives_everything() {
    let (mut env, deck, _) = sample_env();
    let bytes = env.export();
    let schedules = schedule_table(&env.store);
    let cards = live_card_ids(&env.store, &deck);
    assert_eq!(cards.len(), 2);
    // One card on its own first, so it has its own stamp, then every
    // live deck ("Physics" is already deleted in the file).
    env.store.delete_card(&cards[1]).unwrap();
    env.store.delete_deck(&deck).unwrap();
    assert_eq!(env.store.list_decks(T_EXPORT, T_EXPORT).unwrap(), []);

    let plan = env.plan(&bytes).unwrap();
    assert_eq!(
        *plan.summary(),
        ImportSummary {
            decks: TableCounts {
                new: 0,
                present: 1,
                revived: 1
            },
            cards: TableCounts {
                new: 0,
                present: 2,
                revived: 2
            },
            review_events: TableCounts {
                new: 0,
                present: 5,
                revived: 0
            },
            conflicts: vec![],
        }
    );
    let report = env.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(
        (report.decks, report.cards, report.review_events),
        (0, 0, 0)
    );
    assert_eq!((report.revived_decks, report.revived_cards), (1, 2));
    assert!(report.backup.as_ref().unwrap().exists());

    // Every field, deleted rows and the review log as before the deletes;
    // schedules and card order (seq) kept.
    assert_eq!(env.export(), bytes);
    assert_eq!(schedule_table(&env.store), schedules);
    assert_eq!(live_card_ids(&env.store, &deck), cards);

    // The same file again changes nothing and makes no backup.
    let backups = env.backups();
    let plan = env.plan(&bytes).unwrap();
    assert!(plan.summary().is_noop(), "{:?}", plan.summary());
    let report = env.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(report.backup, None);
    assert_eq!(env.export(), bytes);
    assert_eq!(env.backups(), backups);
}

#[test]
fn a_mixed_import_revives_adds_and_skips() {
    let (mut source, deck, _) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    let old_cards = live_card_ids(&target.store, &deck);

    // The source gets a new card and a new deck after the first export.
    let added = source.store.add_card(&deck, "e ?", "2.718").unwrap().id;
    source.store.create_deck("Chemistry").unwrap();
    // The target gets a card the file never had, then deletes the deck:
    // that card is in the deck's delete batch and comes back with it.
    let only_here = target.store.add_card(&deck, "φ ?", "1.618").unwrap().id;
    target
        .store
        .record_review(&only_here, Grade::Pass, Duration::from_secs(1), T0)
        .unwrap();
    let schedules = schedule_table(&target.store);
    target.store.delete_deck(&deck).unwrap();

    let plan = target.plan(&source.export()).unwrap();
    assert_eq!(
        *plan.summary(),
        ImportSummary {
            decks: TableCounts {
                new: 1,
                present: 1,
                revived: 1
            },
            cards: TableCounts {
                new: 1,
                present: 2,
                revived: 3
            },
            review_events: TableCounts {
                new: 0,
                present: 5,
                revived: 0
            },
            conflicts: vec![],
        }
    );
    let report = target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(
        (report.decks, report.cards, report.review_events),
        (1, 1, 0)
    );
    assert_eq!((report.revived_decks, report.revived_cards), (1, 3));

    let mut expected = old_cards;
    expected.extend([only_here, added]);
    assert_eq!(live_card_ids(&target.store, &deck), expected);
    let names: Vec<String> = target
        .store
        .list_decks(T_EXPORT, T_EXPORT)
        .unwrap()
        .into_iter()
        .map(|d| d.deck.name)
        .collect();
    assert_eq!(names, ["Chemistry", "Mathématiques"]);
    assert_eq!(schedule_table(&target.store), schedules);
}

#[test]
fn a_card_deleted_in_the_file_stays_deleted_when_its_deck_comes_back() {
    let (source, deck, card) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    let cards = live_card_ids(&target.store, &deck);
    target.store.delete_deck(&deck).unwrap();
    let stamp = json_of(&target.export())["decks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == deck.as_str())
        .unwrap()["deleted_at"]
        .clone();
    assert!(stamp.is_i64());

    // A file where the deck is live but one of its cards was deleted at
    // exactly the deck's delete stamp here: the card's own row decides.
    let mut doc = json_of(&source.export());
    for c in doc["cards"].as_array_mut().unwrap() {
        if c["id"] == cards[1].as_str() {
            c["deleted_at"] = stamp.clone();
        }
    }
    let plan = target.plan(&archive_of(&doc)).unwrap();
    assert_eq!(
        plan.summary().cards,
        TableCounts {
            new: 0,
            present: 3,
            revived: 1
        }
    );
    target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(live_card_ids(&target.store, &deck), [card]);
}

#[test]
fn a_row_deleted_in_the_file_but_live_here_is_present() {
    let (mut source, deck, _) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    let before = target.export();
    let cards = live_card_ids(&target.store, &deck);
    source.store.delete_card(&cards[0]).unwrap();
    source.store.delete_deck(&deck).unwrap();
    let bytes = source.export();

    let plan = target.plan(&bytes).unwrap();
    assert_eq!(
        *plan.summary(),
        ImportSummary {
            decks: TableCounts {
                new: 0,
                present: 2,
                revived: 0
            },
            cards: TableCounts {
                new: 0,
                present: 4,
                revived: 0
            },
            review_events: TableCounts {
                new: 0,
                present: 5,
                revived: 0
            },
            conflicts: vec![],
        }
    );
    // An import never deletes.
    target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(target.export(), before);

    // With something new in the file, the import is applied, and still
    // deletes nothing.
    source.store.create_deck("Chemistry").unwrap();
    let report = target.import(&source.export()).unwrap();
    assert_eq!((report.decks, report.revived_decks), (1, 0));
    assert_eq!(live_card_ids(&target.store, &deck), cards);
}

#[test]
fn a_plan_whose_deck_was_brought_back_meanwhile_is_stale() {
    let (source, deck, _) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    target.import(&bytes).unwrap();
    target.store.delete_deck(&deck).unwrap();
    let plan = target.plan(&bytes).unwrap();
    assert_eq!(plan.summary().decks.revived, 1);
    target.store.undelete_deck(&deck).unwrap();
    let before = target.export();
    assert!(matches!(
        target.store.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn only_the_cards_deleted_with_the_deck_come_back() {
    let (source, deck, _) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    let cards = live_card_ids(&target.store, &deck);
    // Added and deleted on its own here, before the deck: its own stamp.
    let alone = target.store.add_card(&deck, "τ ?", "6.283").unwrap().id;
    target.store.delete_card(&alone).unwrap();
    target.store.delete_deck(&deck).unwrap();

    let plan = target.plan(&source.export()).unwrap();
    assert_eq!(plan.summary().cards.revived, 2);
    target.store.apply_import_at(&plan, T_EXPORT).unwrap();
    assert_eq!(live_card_ids(&target.store, &deck), cards);
}

#[test]
fn a_plan_whose_deck_was_deleted_meanwhile_is_stale() {
    let (source, mut target, deck, _) = diverged();
    let plan = target.plan(&source.export()).unwrap();
    assert_eq!(plan.summary().decks.revived, 0);
    target.store.delete_deck(&deck).unwrap();
    let before = target.export();
    assert!(matches!(
        target.store.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn a_plan_is_stale_even_if_the_changes_cancel_out_in_the_counts() {
    let mut source = Env::new();
    let a = source.store.create_deck("A").unwrap().id;
    let b = source.store.create_deck("B").unwrap().id;
    source.store.add_card(&a, "a", "").unwrap();
    source.store.add_card(&b, "b", "").unwrap();
    let bytes = source.export();
    let mut target = Env::new();
    target.import(&bytes).unwrap();
    target.store.delete_deck(&a).unwrap();
    let plan = target.plan(&bytes).unwrap();

    // A comes back here and B goes: the same counts, other rows.
    target.store.undelete_deck(&a).unwrap();
    target.store.delete_deck(&b).unwrap();
    assert_eq!(target.plan(&bytes).unwrap().summary(), plan.summary());
    let before = target.export();
    assert!(matches!(
        target.store.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn a_row_deleted_on_both_sides_at_different_times_is_a_conflict() {
    let (source, deck, _) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    let card = live_card_ids(&target.store, &deck)[1].clone();
    target.store.delete_card(&card).unwrap();
    let mut doc = json_of(&source.export());
    for c in doc["cards"].as_array_mut().unwrap() {
        if c["id"] == card.as_str() {
            c["deleted_at"] = json!(T0);
        }
    }
    assert_conflicts(
        &mut target,
        &archive_of(&doc),
        &[Conflict::CardDiffers(card)],
    );
}

// ------------------------------------------------------------- conflicts

/// Applies `bytes` to `target` and checks the whole import is refused
/// with exactly `expected`, writing nothing and making no backup.
fn assert_conflicts(target: &mut Env, bytes: &[u8], expected: &[Conflict]) {
    let before = target.export();
    let backups = target.backups();
    let plan = target.plan(bytes).unwrap();
    assert_eq!(plan.summary().conflicts, expected);
    let result = target.store.apply_import_at(&plan, T_EXPORT);
    assert!(
        matches!(result, Err(Error::ImportConflicts(n)) if n == expected.len()),
        "{result:?}"
    );
    assert_eq!(target.export(), before);
    assert_eq!(target.backups(), backups);
}

/// A target holding the sample's export, and the source with one more
/// card, so a refused import would have had something to add.
fn diverged() -> (Env, Env, String, String) {
    let (mut source, deck, card) = sample_env();
    let mut target = Env::new();
    target.import(&source.export()).unwrap();
    source.store.add_card(&deck, "new", "card").unwrap();
    (source, target, deck, card)
}

#[test]
fn a_deck_with_different_content_refuses_the_import() {
    let (mut source, mut target, deck, _) = diverged();
    source.store.rename_deck(&deck, "Algebra").unwrap();
    assert_conflicts(
        &mut target,
        &source.export(),
        &[Conflict::DeckDiffers(deck)],
    );
}

#[test]
fn a_card_with_different_content_refuses_the_import() {
    let (mut source, mut target, _, card) = diverged();
    source.store.edit_card(&card, "2 + 2 = ?", "4").unwrap();
    assert_conflicts(
        &mut target,
        &source.export(),
        &[Conflict::CardDiffers(card)],
    );
}

#[test]
fn an_edit_after_export_is_still_a_conflict() {
    let (source, deck, card) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    target.import(&bytes).unwrap();
    // Changed, then deleted: only being deleted could be merged.
    target.store.edit_card(&card, "changed", "here").unwrap();
    target.store.delete_card(&card).unwrap();
    target.store.rename_deck(&deck, "Algebra").unwrap();
    target.store.delete_deck(&deck).unwrap();
    assert_conflicts(
        &mut target,
        &bytes,
        &[Conflict::DeckDiffers(deck), Conflict::CardDiffers(card)],
    );
}

#[test]
fn a_revive_whose_name_is_taken_is_refused() {
    let (source, deck, _) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    target.import(&bytes).unwrap();
    target.store.delete_deck(&deck).unwrap();
    // Same name by key (case and Unicode composition), another id.
    let existing = target.store.create_deck("MATHE\u{301}MATIQUES").unwrap().id;
    assert_conflicts(
        &mut target,
        &bytes,
        &[Conflict::DeckNameTaken {
            deck_id: deck,
            existing_id: existing,
        }],
    );
}

#[test]
fn a_review_event_with_different_content_refuses_the_import() {
    let (source, mut target, _, _) = diverged();
    let mut doc = json_of(&source.export());
    doc["review_events"][0]["duration_ms"] = json!(9);
    let id = doc["review_events"][0]["id"].as_str().unwrap().to_owned();
    assert_conflicts(
        &mut target,
        &archive_of(&doc),
        &[Conflict::EventDiffers(id)],
    );
}

#[test]
fn a_live_deck_name_already_in_use_refuses_the_import() {
    let (source, deck, _) = sample_env();
    let mut target = Env::new();
    // Same name by key (case and Unicode composition), another id.
    let existing = target.store.create_deck("MATHE\u{301}MATIQUES").unwrap().id;
    assert_conflicts(
        &mut target,
        &source.export(),
        &[Conflict::DeckNameTaken {
            deck_id: deck,
            existing_id: existing,
        }],
    );
}

#[test]
fn a_deleted_deck_may_share_a_name() {
    let (source, _, _) = sample_env();
    let mut target = Env::new();
    // "Physics" is deleted in the file; a live "Physics" here is fine,
    // and a deleted "Mathématiques" here does not block the live one.
    target.store.create_deck("Physics").unwrap();
    let old = target.store.create_deck("Mathématiques").unwrap().id;
    target.store.delete_deck(&old).unwrap();
    target.import(&source.export()).unwrap();
    let names: Vec<String> = target
        .store
        .list_decks(T_EXPORT, T_EXPORT)
        .unwrap()
        .into_iter()
        .map(|d| d.deck.name)
        .collect();
    assert_eq!(names, ["Mathématiques", "Physics"]);
}

#[test]
fn a_plan_whose_new_rows_arrived_meanwhile_is_stale() {
    let (source, mut target, _, _) = diverged();
    let bytes = source.export();
    let plan = target.plan(&bytes).unwrap();
    target.import(&bytes).unwrap();
    let before = target.export();
    assert!(matches!(
        target.store.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn a_plan_made_before_the_database_changed_is_refused() {
    let (source, mut target, _, card) = diverged();
    let plan = target.plan(&source.export()).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    target
        .store
        .edit_card(&card, "changed", "meanwhile")
        .unwrap();
    let before = target.export();
    assert!(matches!(
        target.store.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

// ------------------------------------------------- a second connection

impl Env {
    /// A second store on the same database, as a worker thread opens it.
    fn concurrent(&self) -> Store {
        Store::open_concurrent(&self.tmp.path().join("data")).unwrap()
    }
}

#[test]
fn an_export_on_another_thread_sees_only_committed_data() {
    let (env, deck, _) = sample_env();
    let committed = env.export();
    // An unfinished write on the main connection...
    let tx = env.store.conn.unchecked_transaction().unwrap();
    tx.execute("UPDATE decks SET name = 'Changed' WHERE id = ?1", [&deck])
        .unwrap();
    // ...is not in an export made meanwhile on another thread.
    let worker = env.concurrent();
    let bytes = std::thread::spawn(move || export(&worker)).join().unwrap();
    assert_eq!(bytes, committed);
    drop(tx); // rolls back
    assert_eq!(env.export(), committed);
}

#[test]
fn an_import_on_another_thread_while_the_main_store_reads() {
    let (source, _, _) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    let mut worker = target.concurrent();
    // A read transaction on the main connection (its first read takes the
    // snapshot) does not block the write, and keeps seeing its snapshot.
    let reading = target.store.conn.unchecked_transaction().unwrap();
    let cards_in = |tx: &rusqlite::Transaction<'_>| -> i64 {
        tx.query_row("SELECT count(*) FROM cards", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(cards_in(&reading), 0);
    let plan = worker.plan_import_bytes(&bytes, &Limits::DEFAULT).unwrap();
    let report = std::thread::spawn(move || worker.apply_import_at(&plan, T_EXPORT))
        .join()
        .unwrap()
        .unwrap();
    assert!(report.cards > 0);
    assert_eq!(cards_in(&reading), 0);
    reading.finish().unwrap();
    assert_eq!(target.export(), bytes);

    // Ids issued by the main store afterwards stay above the imported ones.
    let largest: String = target
        .store
        .conn
        .query_row("SELECT max(id) FROM decks", [], |r| r.get(0))
        .unwrap();
    let deck = target.store.create_deck("After the import").unwrap();
    assert!(deck.id > largest);
}

#[test]
fn a_change_on_the_main_store_makes_a_worker_plan_stale() {
    let (source, target, _, card) = diverged();
    let mut worker = target.concurrent();
    let plan = worker
        .plan_import_bytes(&source.export(), &Limits::DEFAULT)
        .unwrap();
    assert_eq!(plan.summary().conflicts, []);
    let mut target = target;
    target
        .store
        .edit_card(&card, "changed", "meanwhile")
        .unwrap();
    let before = target.export();
    assert!(matches!(
        worker.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn what_a_worker_returns_can_cross_threads() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<ImportPlan>();
    assert_send::<ImportReport>();
    assert_send::<ExportSummary>();
    assert_send::<crate::Error>();
}

// ----------------------------------------------------- malformed / tampered

/// The sample's export as JSON, to tamper with.
fn sample_doc() -> Value {
    let (env, _, _) = sample_env();
    json_of(&env.export())
}

/// A change made to a valid document.
type Mutation = fn(&mut Value);

fn plan_doc(doc: &Value) -> ImportProblem {
    refused(Env::new().plan(&archive_of(doc)))
}

fn field(
    table: &'static str,
    index: usize,
    name: &'static str,
    reason: FieldProblem,
) -> ImportProblem {
    ImportProblem::InvalidField {
        table,
        index,
        field: name,
        reason,
    }
}

#[test]
fn files_that_are_not_cards_archives_are_refused() {
    let env = Env::new();
    let good = sample_env().0.export();
    for bytes in [
        Vec::new(),
        b"hello".to_vec(),
        FIXTURE.as_bytes().to_vec(),
        good[..good.len() - 1].to_vec(),
        good[good.len() / 2..].to_vec(),
        [good.as_slice(), b"trailing"].concat(),
    ] {
        assert!(
            matches!(refused(env.plan(&bytes)), ImportProblem::InvalidArchive(_)),
            "{} bytes",
            bytes.len()
        );
    }
}

/// A zip with the given entries: `Some(data)` is a stored file, `None` a
/// directory.
fn zip_with(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        match data {
            Some(data) => {
                let options =
                    SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Stored);
                zip.start_file(*name, options).unwrap();
                zip.write_all(data).unwrap();
            }
            None => zip
                .add_directory(*name, SimpleFileOptions::DEFAULT)
                .unwrap(),
        }
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn archive_entries_must_be_exactly_cards_json_and_media() {
    let json = FIXTURE.as_bytes();
    let env = Env::new();
    let check = |entries: &[(&str, Option<&[u8]>)], expected: ImportProblem| {
        assert_eq!(
            refused(env.plan(&zip_with(entries))),
            expected,
            "{entries:?}"
        );
    };
    let unexpected = |s: &str| ImportProblem::UnexpectedEntry(format!("\"{s}\""));

    check(
        &[(JSON_ENTRY, Some(json))],
        ImportProblem::MissingEntry("media/"),
    );
    check(
        &[("media/", None)],
        ImportProblem::MissingEntry("cards.json"),
    );
    check(
        &[("../cards.json", Some(json)), ("media/", None)],
        unexpected("../cards.json"),
    );
    check(
        &[("/cards.json", Some(json)), ("media/", None)],
        unexpected("/cards.json"),
    );
    check(
        &[("Cards.json", Some(json)), ("media/", None)],
        unexpected("Cards.json"),
    );
    check(
        &[
            (JSON_ENTRY, Some(json)),
            ("media/", None),
            ("media/x.png", Some(b"x")),
        ],
        unexpected("media/x.png"),
    );
    check(
        &[(JSON_ENTRY, Some(json)), ("media/", None), ("extra/", None)],
        unexpected("extra/"),
    );
    check(
        &[(JSON_ENTRY, None), ("media/", None)],
        // A directory named "cards.json/" is not the entry.
        unexpected("cards.json/"),
    );
    check(
        &[(JSON_ENTRY, Some(json)), ("media", Some(b""))],
        unexpected("media"),
    );
    // The right entries are accepted, stored or deflated, in any order.
    assert!(
        env.plan(&zip_with(&[("media/", None), (JSON_ENTRY, Some(json))]))
            .is_ok()
    );
    assert!(env.plan(&write_archive(json).unwrap()).is_ok());
}

#[test]
fn a_repeated_entry_name_is_refused() {
    // The writer refuses duplicate names, so rename the second entry in
    // place (local header and central directory).
    let json = FIXTURE.as_bytes();
    let bytes = zip_with(&[
        (JSON_ENTRY, Some(json)),
        ("cards.jsoX", Some(json)),
        ("media/", None),
    ]);
    let patched = replace_all(&bytes, b"cards.jsoX", b"cards.json");
    assert!(matches!(
        refused(Env::new().plan(&patched)),
        ImportProblem::UnexpectedEntry(_)
    ));
}

fn replace_all(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut out = bytes.to_vec();
    let mut i = 0;
    let mut n = 0;
    while i + from.len() <= out.len() {
        if &out[i..i + from.len()] == from {
            out[i..i + from.len()].copy_from_slice(to);
            n += 1;
        }
        i += 1;
    }
    assert!(n > 0);
    out
}

#[test]
fn a_corrupted_entry_fails_its_crc() {
    let bytes = zip_with(&[(JSON_ENTRY, Some(FIXTURE.as_bytes())), ("media/", None)]);
    // Change one character of the stored JSON; the CRC no longer matches.
    let tampered = replace_all(&bytes, b"\"Maths\"", b"\"Mathz\"");
    assert!(matches!(
        refused(Env::new().plan(&tampered)),
        ImportProblem::InvalidArchive(_)
    ));
}

#[test]
fn size_limits_are_enforced_whatever_the_entry_claims() {
    let env = Env::new();
    let bytes = sample_env().0.export();
    let json_len = read_cards_json(&bytes, &Limits::DEFAULT).unwrap().len() as u64;

    let tight = Limits {
        json: json_len - 1,
        ..Limits::DEFAULT
    };
    assert_eq!(
        refused(env.store.plan_import_bytes(&bytes, &tight)),
        ImportProblem::EntryTooLarge {
            entry: "cards.json",
            limit: json_len - 1
        }
    );
    let exact = Limits {
        json: json_len,
        ..Limits::DEFAULT
    };
    assert!(env.store.plan_import_bytes(&bytes, &exact).is_ok());

    // A highly compressible entry (a zip bomb in miniature).
    let bomb = write_archive(&vec![b' '; 1 << 20]).unwrap();
    assert!(bomb.len() < 10_000);
    let small = Limits {
        json: 1 << 16,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        refused(env.store.plan_import_bytes(&bomb, &small)),
        ImportProblem::EntryTooLarge { .. }
    ));

    // The file itself.
    let path = env.tmp.path().join("big.cards");
    fs::write(&path, &bytes).unwrap();
    let tiny = Limits {
        archive: bytes.len() as u64 - 1,
        ..Limits::DEFAULT
    };
    assert_eq!(
        refused(env.store.plan_import_with(&path, &tiny)),
        ImportProblem::FileTooLarge {
            limit: bytes.len() as u64 - 1
        }
    );
}

#[test]
fn text_limits_are_enforced() {
    let mut doc = sample_doc();
    let limits = Limits {
        card_text: 8,
        deck_name: 4,
        ..Limits::DEFAULT
    };
    let env = Env::new();
    let plan = |doc: &Value| refused(env.store.plan_import_bytes(&archive_of(doc), &limits));
    assert_eq!(plan(&doc), field("decks", 0, "name", FieldProblem::TooLong));
    doc["decks"][0]["name"] = json!("Abc");
    doc["decks"][1]["name"] = json!("Def");
    doc["cards"][0]["front_md"] = json!("123456789");
    assert_eq!(
        plan(&doc),
        field("cards", 0, "front_md", FieldProblem::TooLong)
    );
    doc["cards"][0]["front_md"] = json!("q");
    doc["cards"][0]["back_md"] = json!("123456789");
    assert_eq!(
        plan(&doc),
        field("cards", 0, "back_md", FieldProblem::TooLong)
    );
}

#[test]
fn unsupported_format_versions_are_refused() {
    for version in [json!(0), json!(2), json!(-1), json!(1_000_000)] {
        let mut doc = sample_doc();
        doc["format_version"] = version.clone();
        assert_eq!(
            plan_doc(&doc),
            ImportProblem::UnsupportedFormatVersion {
                found: version.as_i64().unwrap()
            }
        );
    }
    // Checked before the rest: a newer file with new fields still says
    // "unsupported version".
    let mut doc = sample_doc();
    doc["format_version"] = json!(2);
    doc["tags"] = json!([]);
    assert!(matches!(
        plan_doc(&doc),
        ImportProblem::UnsupportedFormatVersion { found: 2 }
    ));
}

#[test]
fn json_that_does_not_match_the_format_is_refused() {
    let env = Env::new();
    let json_problem = |bytes: &[u8]| {
        matches!(
            refused(env.plan(&write_archive(bytes).unwrap())),
            ImportProblem::Json(_)
        )
    };
    assert!(json_problem(b"not json"));
    assert!(json_problem(b"{}"));
    assert!(json_problem(br#"{"format_version": "1"}"#));
    assert!(json_problem(br#"{"format_version": 1}"#));

    let mutations: [Mutation; 9] = [
        |d| d["unknown"] = json!(1),
        |d| d["decks"][0]["extra"] = json!(true),
        |d| d["cards"][0]["seq"] = json!(1),
        |d| d["review_events"][0]["duration_ms"] = json!(1.5),
        |d| d["review_events"][0]["reviewed_at"] = json!("yesterday"),
        |d| d["cards"][0]["deck_id"] = json!(null),
        |d| _ = d.as_object_mut().unwrap().remove("cards"),
        // Missing, not null: must not read as "not deleted".
        |d| _ = d["decks"][1].as_object_mut().unwrap().remove("deleted_at"),
        |d| _ = d["cards"][1].as_object_mut().unwrap().remove("deleted_at"),
    ];
    for (i, mutate) in mutations.into_iter().enumerate() {
        let mut doc = sample_doc();
        mutate(&mut doc);
        assert!(
            matches!(plan_doc(&doc), ImportProblem::Json(_)),
            "mutation {i}"
        );
    }
}

#[test]
fn ids_must_be_lowercase_uuidv7() {
    let bad_ids = [
        "01920000-0000-4000-8000-000000000001", // v4
        "01920000-0000-8000-8000-000000000001", // v8
        "01920000-0000-7000-c000-000000000001", // variant 110
        "01920000-0000-7000-8000-00000000000A", // uppercase
        "01920000000070008000000000000001",     // no hyphens
        "x",
        "",
    ];
    for bad in bad_ids {
        for (table, index) in [("decks", 1), ("cards", 2), ("review_events", 3)] {
            let mut doc = sample_doc();
            doc[table][index]["id"] = json!(bad);
            assert_eq!(
                plan_doc(&doc),
                field(table, index, "id", FieldProblem::NotUuidV7),
                "{table} {bad}"
            );
        }
    }
}

#[test]
fn ids_dated_after_the_year_9999_are_refused() {
    // The largest id would leave the id generator no room (#12 review):
    // every later deck, card or review would fail with IdsExhausted.
    let past_9999 = format!("{:012x}", 253_402_300_800_000_i64);
    let just_past = format!(
        "{}-{}-7000-8000-000000000001",
        &past_9999[..8],
        &past_9999[8..]
    );
    for bad in ["ffffffff-ffff-7fff-bfff-ffffffffffff", just_past.as_str()] {
        for (table, index) in [("decks", 0), ("cards", 1), ("review_events", 2)] {
            let mut doc = sample_doc();
            doc[table][index]["id"] = json!(bad);
            assert_eq!(
                plan_doc(&doc),
                field(table, index, "id", FieldProblem::TimestampOutOfRange),
                "{table} {bad}"
            );
        }
    }
    // The last millisecond of 9999 is fine.
    let last = format!("{:012x}", 253_402_300_799_999_i64);
    let mut doc = sample_doc();
    doc["decks"][1]["id"] = json!(format!(
        "{}-{}-7fff-bfff-ffffffffffff",
        &last[..8],
        &last[8..]
    ));
    let deck = doc["decks"][1]["id"].clone();
    doc["cards"][3]["deck_id"] = deck;
    assert!(Env::new().plan(&archive_of(&doc)).is_ok());
}

#[test]
fn ids_must_be_unique_within_a_table() {
    for table in ["decks", "cards", "review_events"] {
        let mut doc = sample_doc();
        doc[table][1]["id"] = doc[table][0]["id"].clone();
        assert_eq!(
            plan_doc(&doc),
            field(table, 1, "id", FieldProblem::DuplicateId),
            "{table}"
        );
    }
}

#[test]
fn references_must_resolve_within_the_file() {
    let missing = json!("01920000-0000-7000-8000-0000000000ff");
    let mut doc = sample_doc();
    doc["cards"][1]["deck_id"] = missing.clone();
    assert_eq!(
        plan_doc(&doc),
        field("cards", 1, "deck_id", FieldProblem::UnknownReference)
    );

    let mut doc = sample_doc();
    doc["review_events"][4]["card_id"] = missing;
    assert_eq!(
        plan_doc(&doc),
        field(
            "review_events",
            4,
            "card_id",
            FieldProblem::UnknownReference
        )
    );

    // A card id is not a deck id.
    let mut doc = sample_doc();
    doc["cards"][0]["deck_id"] = doc["cards"][1]["id"].clone();
    assert_eq!(
        plan_doc(&doc),
        field("cards", 0, "deck_id", FieldProblem::UnknownReference)
    );
}

#[test]
fn values_are_checked() {
    let cases: [(Mutation, ImportProblem); 12] = [
        (
            |d| d["review_events"][0]["grade"] = json!("easy"),
            field("review_events", 0, "grade", FieldProblem::InvalidGrade),
        ),
        (
            |d| d["review_events"][0]["grade"] = json!("Pass"),
            field("review_events", 0, "grade", FieldProblem::InvalidGrade),
        ),
        (
            |d| d["review_events"][1]["duration_ms"] = json!(-1),
            field("review_events", 1, "duration_ms", FieldProblem::Negative),
        ),
        (
            |d| d["review_events"][2]["reviewed_at"] = json!(-1),
            field(
                "review_events",
                2,
                "reviewed_at",
                FieldProblem::TimestampOutOfRange,
            ),
        ),
        (
            |d| d["cards"][0]["updated_at"] = json!(253_402_300_800_000_i64),
            field("cards", 0, "updated_at", FieldProblem::TimestampOutOfRange),
        ),
        (
            |d| d["decks"][1]["deleted_at"] = json!(i64::MIN),
            field("decks", 1, "deleted_at", FieldProblem::TimestampOutOfRange),
        ),
        (
            |d| d["exported_at"] = json!(-5),
            ImportProblem::InvalidTopLevel {
                field: "exported_at",
                reason: FieldProblem::TimestampOutOfRange,
            },
        ),
        (
            |d| d["decks"][0]["name"] = json!("  "),
            field("decks", 0, "name", FieldProblem::Empty),
        ),
        (
            |d| d["decks"][0]["name"] = json!(" Maths"),
            field("decks", 0, "name", FieldProblem::NotTrimmed),
        ),
        (
            |d| d["cards"][3]["front_md"] = json!("\n"),
            field("cards", 3, "front_md", FieldProblem::Empty),
        ),
        (
            |d| d["settings"] = json!({"desired_retention": "0.9"}),
            ImportProblem::UnsupportedSettings,
        ),
        (
            |d| d["media"] = json!([{"hash": "x"}]),
            ImportProblem::UnsupportedMedia,
        ),
    ];
    for (i, (mutate, expected)) in cases.into_iter().enumerate() {
        let mut doc = sample_doc();
        mutate(&mut doc);
        assert_eq!(plan_doc(&doc), expected, "case {i}");
    }
}

#[test]
fn states_the_app_never_produces_are_refused() {
    // The sample's decks: [0] live, [1] deleted. Cards [0..3) are in [0].
    let mut doc = sample_doc();
    let deleted_deck = doc["decks"][1]["id"].clone();
    doc["cards"][0]["deck_id"] = deleted_deck;
    assert_eq!(
        plan_doc(&doc),
        field(
            "cards",
            0,
            "deleted_at",
            FieldProblem::LiveCardInDeletedDeck
        )
    );

    let mut doc = sample_doc();
    doc["decks"][1]["deleted_at"] = json!(null);
    doc["decks"][1]["name"] = json!("MATHÉMATIQUES");
    assert_eq!(
        plan_doc(&doc),
        field("decks", 1, "name", FieldProblem::DuplicateDeckName)
    );
}

#[test]
fn a_refused_file_writes_nothing() {
    let (mut env, _, _) = sample_env();
    let before = env.export();
    let backups = env.backups();
    let mut doc = json_of(&before);
    doc["review_events"][0]["grade"] = json!("again");
    assert!(env.import(&archive_of(&doc)).is_err());
    assert_eq!(env.export(), before);
    assert_eq!(env.backups(), backups);
}

#[test]
fn json_errors_give_a_place_not_the_text() {
    let mut doc = sample_doc();
    doc["review_events"][0]["duration_ms"] = json!(format!("SECRET{}", "x".repeat(100_000)));
    let problem = plan_doc(&doc);
    let ImportProblem::Json(message) = &problem else {
        panic!("{problem:?}");
    };
    assert!(!message.contains("SECRET"), "{message}");
    assert!(message.len() < 100, "{message}");
    assert!(message.contains("line "), "{message}");

    let mut doc = sample_doc();
    doc["cards"][0]["SECRET field"] = json!(1);
    assert!(!plan_doc(&doc).to_string().contains("SECRET"));
}

#[test]
fn an_archive_comment_is_refused() {
    let mut bytes = write_archive(FIXTURE.as_bytes()).unwrap();
    // Set the end record's comment length and append the comment.
    let n = bytes.len();
    bytes[n - 2..].copy_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(b"abc");
    assert!(matches!(
        refused(Env::new().plan(&bytes)),
        ImportProblem::InvalidArchive(_)
    ));
}

#[test]
fn problems_never_quote_card_text() {
    let mut doc = sample_doc();
    doc["review_events"][0]["grade"] = json!("SECRET");
    doc["cards"][0]["front_md"] = json!("SECRET");
    let message = Error::ImportRefused(plan_doc(&doc)).to_string();
    assert!(!message.contains("SECRET"), "{message}");
    assert_eq!(
        message,
        "cannot import this file: review_events[0].grade: not a grade"
    );
}

#[test]
fn a_plan_debug_prints_only_the_summary() {
    let (env, _, _) = sample_env();
    let plan = Env::new().plan(&env.export()).unwrap();
    let shown = format!("{plan:?}");
    assert!(shown.contains("summary"), "{shown}");
    assert!(!shown.contains("2 + 2"), "{shown}");
}

// --------------------------------------------------------------- property

#[derive(Debug, Clone)]
enum Op {
    CreateDeck(usize),
    RenameDeck(usize, usize),
    DeleteDeck(usize),
    UndeleteDeck(usize),
    AddCard(usize, usize),
    EditCard(usize, usize),
    DeleteCard(usize),
    UndeleteCard(usize),
    Review {
        card: usize,
        pass: bool,
        clock_delta: i64,
    },
}

/// Names that collide by key, so renames and creates sometimes fail.
const NAMES: [&str; 5] = ["Maths", "maths", "Physique", "PHYSIQUE", "Ελληνικά"];
const TEXTS: [&str; 4] = ["q", "two\nlines", "\"quoted\" & <b>", "√2 ≈ 1.414"];

fn op() -> impl Strategy<Value = Op> {
    let i = || 0usize..16;
    prop_oneof![
        2 => (0..NAMES.len()).prop_map(Op::CreateDeck),
        1 => (i(), 0..NAMES.len()).prop_map(|(d, n)| Op::RenameDeck(d, n)),
        1 => i().prop_map(Op::DeleteDeck),
        1 => i().prop_map(Op::UndeleteDeck),
        4 => (i(), 0..TEXTS.len()).prop_map(|(d, t)| Op::AddCard(d, t)),
        1 => (i(), 0..TEXTS.len()).prop_map(|(c, t)| Op::EditCard(c, t)),
        1 => i().prop_map(Op::DeleteCard),
        1 => i().prop_map(Op::UndeleteCard),
        8 => (i(), any::<bool>(), 0..20 * DAY_MS).prop_map(|(card, pass, clock_delta)| {
            Op::Review { card, pass, clock_delta }
        }),
    ]
}

/// Runs `ops` on `store` through its public API, ignoring the refusals
/// (duplicate names, deleted targets): they are part of a random history.
fn run(store: &mut Store, ops: &[Op]) {
    let mut decks: Vec<String> = Vec::new();
    let mut cards: Vec<String> = Vec::new();
    let mut clock = T0;
    let pick = |v: &[String], i: usize| (!v.is_empty()).then(|| v[i % v.len()].clone());
    for op in ops {
        match *op {
            Op::CreateDeck(n) => {
                if let Ok(d) = store.create_deck(NAMES[n]) {
                    decks.push(d.id);
                }
            }
            Op::RenameDeck(d, n) => {
                if let Some(d) = pick(&decks, d) {
                    let _ = store.rename_deck(&d, NAMES[n]);
                }
            }
            Op::DeleteDeck(d) => {
                if let Some(d) = pick(&decks, d) {
                    let _ = store.delete_deck(&d);
                }
            }
            Op::UndeleteDeck(d) => {
                if let Some(d) = pick(&decks, d) {
                    let _ = store.undelete_deck(&d);
                }
            }
            Op::AddCard(d, t) => {
                if let Some(d) = pick(&decks, d)
                    && let Ok(c) = store.add_card(&d, TEXTS[t], TEXTS[(t + 1) % TEXTS.len()])
                {
                    cards.push(c.id);
                }
            }
            Op::EditCard(c, t) => {
                if let Some(c) = pick(&cards, c) {
                    let _ = store.edit_card(&c, TEXTS[t], "edited");
                }
            }
            Op::DeleteCard(c) => {
                if let Some(c) = pick(&cards, c) {
                    let _ = store.delete_card(&c);
                }
            }
            Op::UndeleteCard(c) => {
                if let Some(c) = pick(&cards, c) {
                    let _ = store.undelete_card(&c);
                }
            }
            Op::Review {
                card,
                pass,
                clock_delta,
            } => {
                clock += clock_delta;
                if let Some(c) = pick(&cards, card) {
                    let grade = if pass { Grade::Pass } else { Grade::Fail };
                    let _ = store.record_review(&c, grade, Duration::from_millis(700), clock);
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// For any database: export, import into an empty database, export
    /// again gives the same bytes and the same schedule cache; importing
    /// the file again, into either database, changes nothing.
    #[test]
    fn random_databases_round_trip(ops in prop::collection::vec(op(), 0..120)) {
        let mut source = Env::new();
        run(&mut source.store, &ops);
        let bytes = source.export();

        let mut target = Env::new();
        let plan = target.plan(&bytes).unwrap();
        prop_assert!(plan.summary().conflicts.is_empty());
        prop_assert_eq!(plan.summary().decks.present, 0);
        target.store.apply_import_at(&plan, T_EXPORT).unwrap();
        prop_assert_eq!(&target.export(), &bytes);
        prop_assert_eq!(schedule_table(&target.store), schedule_table(&source.store));

        for env in [&mut source, &mut target] {
            let backups = env.backups();
            let plan = env.plan(&bytes).unwrap();
            prop_assert!(plan.summary().is_noop());
            prop_assert!(plan.summary().conflicts.is_empty());
            prop_assert_eq!(env.store.apply_import_at(&plan, T_EXPORT).unwrap().backup, None);
            prop_assert_eq!(&env.export(), &bytes);
            prop_assert_eq!(env.backups(), backups);
        }
    }
}

// ----------------------------------------------------------- larger data

#[test]
fn five_thousand_cards_with_reviews_round_trip() {
    let mut source = Env::new();
    let deck = source.store.create_deck("Big").unwrap().id;
    for i in 0..5_000 {
        let card = source
            .store
            .add_card(&deck, &format!("question {i}"), &format!("answer {i}"))
            .unwrap()
            .id;
        if i % 5 == 0 {
            for k in 0..3 {
                source
                    .store
                    .record_review(&card, Grade::Pass, Duration::from_secs(2), T0 + k * DAY_MS)
                    .unwrap();
            }
        }
    }
    let bytes = source.export();
    let mut target = Env::new();
    let report = target.import(&bytes).unwrap();
    assert_eq!((report.cards, report.review_events), (5_000, 3_000));
    assert_eq!(target.export(), bytes);
    assert_eq!(schedule_table(&target.store), schedule_table(&source.store));
}
