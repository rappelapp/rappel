//! Larger data.

use super::*;

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

/// An export made by the last version before the rename to Rappel (named
/// `.cards`, format version 1): 3 decks (one deleted), 5 cards (two deleted,
/// one of them with its deck) and 4 review events. Written once with that
/// build and never regenerated: it guards the archive layout against drift.
const PRE_RENAME_EXPORT: &[u8] = include_bytes!("../../../tests/fixtures/export-v1.cards");

#[test]
fn imports_an_export_made_before_the_rename() {
    let mut env = Env::new();
    let plan = env.plan(PRE_RENAME_EXPORT).unwrap();
    assert_eq!(
        (
            plan.summary().decks.new,
            plan.summary().cards.new,
            plan.summary().review_events.new
        ),
        (3, 5, 4)
    );

    let report = env.import(PRE_RENAME_EXPORT).unwrap();
    assert_eq!(
        (report.decks, report.cards, report.review_events),
        (3, 5, 4)
    );

    // Every row came across unchanged: ids, texts, deleted stamps, reviews.
    let old = json_of(PRE_RENAME_EXPORT);
    let now = json_of(&env.export());
    assert_eq!(old["format_version"], 1);
    for table in ["decks", "cards", "review_events"] {
        assert_eq!(now[table], old[table], "{table}");
    }
    assert_eq!(old["cards"].as_array().unwrap().len(), 5);
}

#[test]
fn importing_a_pre_rename_export_twice_changes_nothing() {
    let mut env = Env::new();
    env.import(PRE_RENAME_EXPORT).unwrap();
    let after_first = env.export();
    assert!(env.plan(PRE_RENAME_EXPORT).unwrap().summary().is_noop());
    env.import(PRE_RENAME_EXPORT).unwrap();
    assert_eq!(env.export(), after_first);
}

#[test]
fn the_pre_rename_export_has_the_same_layout_as_a_new_one() {
    // Same two entries, so one reader serves both extensions.
    let env = Env::new();
    let new = env.export();
    for bytes in [PRE_RENAME_EXPORT, new.as_slice()] {
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<_> = archive
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["cards.json", "media/"]);
    }
}

#[test]
fn imports_accept_the_old_and_the_new_extension() {
    assert_eq!(FILE_EXTENSION, "rappel");
    assert!(IMPORT_EXTENSIONS.contains(&FILE_EXTENSION));
    assert!(IMPORT_EXTENSIONS.contains(&"cards"));
}

#[test]
fn imports_a_pre_rename_export_from_disk_whatever_its_extension() {
    let mut env = Env::new();
    for name in ["old.cards", "renamed.rappel"] {
        let path = env.tmp.path().join(name);
        fs::write(&path, PRE_RENAME_EXPORT).unwrap();
        let plan = env.store.plan_import(&path).unwrap();
        if name == "old.cards" {
            let report = env.store.apply_import(&plan).unwrap();
            assert_eq!(
                (report.decks, report.cards, report.review_events),
                (3, 5, 4)
            );
        } else {
            assert!(plan.summary().is_noop(), "{name}");
        }
    }
}
