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
/// build and never regenerated: it guards the archive layout against drift,
/// and its version is refused (ADR 0017).
const PRE_RENAME_EXPORT: &[u8] = include_bytes!("../../../tests/fixtures/export-v1.cards");

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

/// A `.cards` file is still offered by the file chooser, and refused by
/// its version, whatever its name, with nothing written.
#[test]
fn a_pre_rename_export_is_refused_by_its_version_whatever_its_extension() {
    let env = Env::new();
    for name in ["old.cards", "renamed.rappel"] {
        let path = env.tmp.path().join(name);
        fs::write(&path, PRE_RENAME_EXPORT).unwrap();
        let err = env.store.plan_import(&path).unwrap_err();
        assert!(
            matches!(err, Error::ImportRefused(ImportProblem::EarlyTestVersion)),
            "{name}: {err:?}"
        );
    }
    assert_eq!(env.store.decks().unwrap(), []);
    assert_eq!(env.backups(), 0);
}
