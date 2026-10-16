//! Format.

use super::*;

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
    let path = env.tmp.path().join("all.rappel");
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
    let path = env.tmp.path().join("all.rappel");
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
    let path = env.tmp.path().join("missing-dir").join("all.rappel");
    assert!(matches!(env.store.export_to(&path), Err(Error::Io { .. })));
    assert!(!path.exists());
}
