//! A deck's `archived_at` in the file (ADR 0068): always written, optional
//! when read, kept by a new deck, and never changed on a deck the database
//! has.

use super::*;

/// "Maths" (archived, one card) and "Physics" (not), with the stamp.
fn env_with_decks() -> (Env, String, String, i64) {
    let mut env = Env::new();
    let maths = env.store.create_deck("Maths").unwrap().id;
    let physics = env.store.create_deck("Physics").unwrap().id;
    env.store.add_card(&maths, "2 + 2", "4").unwrap();
    env.store.add_card(&physics, "c", "light").unwrap();
    let stamp = env.store.archive_deck(&maths).unwrap().archived_at.unwrap();
    (env, maths, physics, stamp)
}

fn archived_at(store: &Store, id: &str) -> Option<i64> {
    store.deck(id).unwrap().archived_at
}

/// Each deck record's `archived_at`, by deck id, as the file has it.
fn flags_in(json: &Value) -> Vec<(String, Value)> {
    json["decks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["id"].as_str().unwrap().to_owned(),
                d["archived_at"].clone(),
            )
        })
        .collect()
}

#[test]
fn the_flag_is_written_on_every_deck_and_round_trips() {
    let (source, maths, physics, stamp) = env_with_decks();
    let bytes = source.export();
    let json = json_of(&bytes);
    let mut flags = flags_in(&json);
    flags.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = vec![
        (maths.clone(), json!(stamp)),
        (physics.clone(), json!(null)),
    ];
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(flags, expected);
    // Written after `updated_at`, before `deleted_at`.
    let text = String::from_utf8(read_cards_json(&bytes, &Limits::DEFAULT).unwrap()).unwrap();
    let at = |key: &str| text.find(key).unwrap();
    assert!(at("\"updated_at\"") < at("\"archived_at\""));
    assert!(at("\"archived_at\"") < at("\"deleted_at\""));

    let mut target = Env::new();
    target.import(&bytes).unwrap();
    assert_eq!(archived_at(&target.store, &maths), Some(stamp));
    assert_eq!(archived_at(&target.store, &physics), None);
    assert_eq!(target.export(), bytes);
    assert_eq!(target.store.archived_decks().unwrap().len(), 1);
}

#[test]
fn a_file_without_the_field_or_with_null_reads_as_not_archived() {
    let (source, maths, physics, _) = env_with_decks();
    let base = json_of(&source.export());
    for remove in [true, false] {
        let mut value = base.clone();
        for deck in value["decks"].as_array_mut().unwrap() {
            let deck = deck.as_object_mut().unwrap();
            if remove {
                deck.remove("archived_at").unwrap();
            } else {
                deck.insert("archived_at".to_owned(), json!(null));
            }
        }
        let mut target = Env::new();
        target.import(&archive_of(&value)).unwrap();
        assert_eq!(archived_at(&target.store, &maths), None, "{remove}");
        assert_eq!(archived_at(&target.store, &physics), None);
        assert_eq!(
            target.store.list_decks(T_EXPORT, T_EXPORT).unwrap().len(),
            2
        );
    }
}

#[test]
fn a_new_deck_keeps_the_files_value() {
    let (source, maths, physics, _) = env_with_decks();
    let mut value = json_of(&source.export());
    for deck in value["decks"].as_array_mut().unwrap() {
        deck["archived_at"] = json!(5);
    }
    let mut target = Env::new();
    target.import(&archive_of(&value)).unwrap();
    assert_eq!(archived_at(&target.store, &maths), Some(5));
    assert_eq!(archived_at(&target.store, &physics), Some(5));
    assert_eq!(target.store.list_decks(T_EXPORT, T_EXPORT).unwrap(), []);
}

/// Anything but `null` or a whole number of milliseconds from 1970 to
/// 9999 is refused as the field of its row (ADR 0068), and nothing is
/// written.
#[test]
fn every_other_value_is_refused_as_the_field() {
    let (source, ..) = env_with_decks();
    let base = json_of(&source.export());
    let cases = [
        (json!("1792067696789"), FieldProblem::NotATimestamp),
        (json!(""), FieldProblem::NotATimestamp),
        (json!(1.5), FieldProblem::NotATimestamp),
        (json!(1.0), FieldProblem::NotATimestamp),
        (json!(true), FieldProblem::NotATimestamp),
        (json!(false), FieldProblem::NotATimestamp),
        (json!({}), FieldProblem::NotATimestamp),
        (json!({"at": 5}), FieldProblem::NotATimestamp),
        (json!([]), FieldProblem::NotATimestamp),
        (json!([5]), FieldProblem::NotATimestamp),
        (json!(-1), FieldProblem::TimestampOutOfRange),
        (json!(i64::MIN), FieldProblem::TimestampOutOfRange),
        (
            json!(253_402_300_800_000_i64),
            FieldProblem::TimestampOutOfRange,
        ),
        (json!(u64::MAX), FieldProblem::TimestampOutOfRange),
    ];
    for (bad, reason) in cases {
        for index in [0, 1] {
            let mut value = base.clone();
            value["decks"][index]["archived_at"] = bad.clone();
            let mut target = Env::new();
            let problem = refused(target.plan(&archive_of(&value)));
            assert_eq!(
                problem,
                ImportProblem::InvalidField {
                    table: "decks",
                    index,
                    field: "archived_at",
                    reason,
                },
                "{bad}"
            );
            assert_eq!(
                problem.to_string(),
                format!("decks[{index}].archived_at: {reason}")
            );
            assert!(target.import(&archive_of(&value)).is_err());
            assert_eq!(target.store.decks_with_archived().unwrap(), []);
        }
    }
    // The edges of the range are accepted.
    for good in [json!(0), json!(253_402_300_799_999_i64)] {
        let mut value = base.clone();
        value["decks"][0]["archived_at"] = good.clone();
        assert!(Env::new().plan(&archive_of(&value)).is_ok(), "{good}");
    }
}

/// A deck the database has that differs only in `archived_at` is already
/// present, whichever way the file has it: the database keeps its own
/// flag, as an import never changes how a live deck is studied.
#[test]
fn a_deck_that_differs_only_in_the_flag_is_already_present() {
    let (mut env, maths, physics, stamp) = env_with_decks();
    let archived = env.export();
    env.store.unarchive_deck(&maths).unwrap();
    let physics_stamp = env.store.archive_deck(&physics).unwrap().archived_at;
    let swapped = env.export();
    assert_ne!(archived, swapped);
    let mut later = json_of(&archived);
    for deck in later["decks"].as_array_mut().unwrap() {
        deck["archived_at"] = json!(stamp + 7);
    }
    for file in [&archived, &swapped, &archive_of(&later)] {
        let plan = env.plan(file).unwrap();
        let s = plan.summary();
        assert_eq!((s.decks.new, s.decks.present, s.decks.revived), (0, 2, 0));
        assert_eq!(s.conflicts, []);
        assert!(s.is_noop(), "{s:?}");
        env.import(file).unwrap();
        assert_eq!(archived_at(&env.store, &maths), None);
        assert_eq!(archived_at(&env.store, &physics), physics_stamp);
    }
    assert_eq!(env.export(), swapped);
}

/// A deck deleted here and live in the file is revived with the
/// database's own flag.
#[test]
fn a_revived_deck_keeps_the_databases_flag() {
    let (mut env, maths, physics, stamp) = env_with_decks();
    let mut value = json_of(&env.export());
    for deck in value["decks"].as_array_mut().unwrap() {
        deck["archived_at"] = if deck["id"] == json!(maths) {
            json!(null)
        } else {
            json!(stamp)
        };
    }
    env.store.delete_deck(&maths).unwrap();
    env.store.delete_deck(&physics).unwrap();
    let plan = env.plan(&archive_of(&value)).unwrap();
    assert_eq!(plan.summary().decks.revived, 2);
    env.import(&archive_of(&value)).unwrap();
    assert_eq!(archived_at(&env.store, &maths), Some(stamp));
    assert_eq!(archived_at(&env.store, &physics), None);
    assert_eq!(env.store.card_count(&maths).unwrap(), 1);
}

/// An archived deck keeps its name taken against an import too.
#[test]
fn a_name_an_archived_deck_has_is_a_conflict() {
    let (source, ..) = env_with_decks();
    let mut target = Env::new();
    let own = target.store.create_deck("maths").unwrap().id;
    target.store.archive_deck(&own).unwrap();
    let plan = target.plan(&source.export()).unwrap();
    assert!(matches!(
        plan.summary().conflicts.as_slice(),
        [Conflict::DeckNameTaken { existing_id, .. }] if *existing_id == own
    ));
}

/// What a case of `odd_archived_at_texts_are_refused_and_write_nothing`
/// expects of the refusal.
type Expected = Box<dyn Fn(&ImportProblem) -> bool>;

/// `bytes` with its `cards.json` text changed by `f`, archived again.
fn with_text(bytes: &[u8], f: impl Fn(&str) -> String) -> Vec<u8> {
    let text = String::from_utf8(read_cards_json(bytes, &Limits::DEFAULT).unwrap()).unwrap();
    write_archive(f(&text).as_bytes()).unwrap()
}

/// Hardening round 4 (ADR 0073): `archived_at` written in every odd way
/// the JSON text allows, beyond the values of
/// `every_other_value_is_refused_as_the_field`. Each is refused as the
/// field, as malformed JSON for what is not one value, or read as the
/// timestamp it is; a refusal writes nothing into a database that has
/// data (no row, no backup).
#[test]
fn odd_archived_at_texts_are_refused_and_write_nothing() {
    let (source, ..) = env_with_decks();
    let file = source.export();
    let null = "\"archived_at\": null,";
    // The second deck in the file is "Physics", not archived.
    let at = |value: &str| {
        let value = value.to_owned();
        with_text(&file, move |t| {
            t.replacen(null, &format!("\"archived_at\": {value},"), 1)
        })
    };
    let not_a_time = |index| ImportProblem::InvalidField {
        table: "decks",
        index,
        field: "archived_at",
        reason: FieldProblem::NotATimestamp,
    };
    let out_of_range = |index| ImportProblem::InvalidField {
        table: "decks",
        index,
        field: "archived_at",
        reason: FieldProblem::TimestampOutOfRange,
    };
    let index = json_of(&file)["decks"]
        .as_array()
        .unwrap()
        .iter()
        .position(|d| d["archived_at"].is_null())
        .unwrap();
    let mut target = Env::new();
    let (deck, _) = sample(&mut target.store);
    let before = crate::test_support::dump(&target.store);
    let backups = target.backups();
    let cases: Vec<(Vec<u8>, Expected)> = vec![
        (
            at(&i64::MAX.to_string()),
            Box::new(move |p| *p == out_of_range(index)),
        ),
        (at("1e3"), Box::new(move |p| *p == not_a_time(index))),
        (at("-0.0"), Box::new(move |p| *p == not_a_time(index))),
        // Past u64: serde_json reads it as a float, not a whole number.
        (
            at("123456789012345678901234567890"),
            Box::new(move |p| *p == not_a_time(index)),
        ),
        (
            at("1e400"),
            Box::new(|p| matches!(p, ImportProblem::Json(_))),
        ),
        (
            at("\"\\u0000\""),
            Box::new(move |p| *p == not_a_time(index)),
        ),
        // A long string is read (within the reader's string bound) and
        // refused as the field; one past the bound is refused where it is.
        (
            at(&format!("\"{}\"", "9".repeat(1 << 20))),
            Box::new(move |p| *p == not_a_time(index)),
        ),
        (
            at(&format!("\"{}\"", "9".repeat(7 << 20))),
            Box::new(|p| matches!(p, ImportProblem::Json(m) if m.contains("a string longer than"))),
        ),
        (
            at(&format!("{}{}", "[".repeat(5_000), "]".repeat(5_000))),
            Box::new(|p| matches!(p, ImportProblem::Json(_))),
        ),
        (
            at(&format!("[{}0]", "0,".repeat(200_000))),
            Box::new(move |p| *p == not_a_time(index)),
        ),
        (
            at("{\"archived_at\": 5, \"deleted_at\": null}"),
            Box::new(move |p| *p == not_a_time(index)),
        ),
        // A key twice in one record: refused as malformed at its place,
        // either order, never one of the two values picked (the reader
        // words every shape error the same way, never quoting the file).
        (
            with_text(&file, |t| {
                t.replacen(null, &format!("{null} \"archived_at\": 5,"), 1)
            }),
            Box::new(
                |p| matches!(p, ImportProblem::Json(m) if m.starts_with("unexpected field, value or type at line")),
            ),
        ),
        (
            with_text(&file, |t| {
                t.replacen(null, &format!("\"archived_at\": 5, {null}"), 1)
            }),
            Box::new(
                |p| matches!(p, ImportProblem::Json(m) if m.starts_with("unexpected field, value or type at line")),
            ),
        ),
    ];
    for (n, (bytes, expected)) in cases.into_iter().enumerate() {
        let problem = refused(target.plan(&bytes));
        assert!(expected(&problem), "case {n}: {problem:?}");
        assert!(target.import(&bytes).is_err(), "case {n}");
        assert_eq!(crate::test_support::dump(&target.store), before, "case {n}");
        assert_eq!(target.backups(), backups, "case {n}");
    }
    // `-0`: serde_json reads it as the float −0.0 (it keeps the sign), so
    // it is not a whole number either; Rappel never writes it.
    assert_eq!(refused(target.plan(&at("-0"))), not_a_time(index));
    // `0` is 1970, a stamp like any other (`every_other_value_…`), and
    // the database still has its deck.
    assert!(target.store.deck(&deck).is_ok());
}

/// Archived decks meeting a merge: a file made before the database
/// archived a deck and merged it away, or merged a deck into one archived
/// since. The merged deck comes back empty with the database's flag, its
/// cards stay where the merge put them (ADR 0066, 0067, 0068), and a
/// second import changes nothing.
#[test]
fn an_archived_deck_meets_a_merge_in_the_file() {
    // Archived, then merged away: the database's row is deleted with its
    // stamp kept; the file (from before both) has it live and not archived.
    let (mut env, maths, physics, stamp) = env_with_decks();
    env.store.unarchive_deck(&maths).unwrap();
    let before_both = env.export();
    let stamp2 = env.store.archive_deck(&maths).unwrap().archived_at;
    assert!(stamp2 >= Some(stamp));
    env.store.merge_deck(&maths, &physics).unwrap();
    let plan = env.plan(&before_both).unwrap();
    let s = plan.summary();
    assert_eq!(s.conflicts, []);
    assert_eq!((s.decks.revived, s.cards_kept_in_their_deck), (1, 1));
    env.import(&before_both).unwrap();
    assert_eq!(archived_at(&env.store, &maths), stamp2);
    assert_eq!(env.store.card_count(&maths).unwrap(), 0);
    assert_eq!(env.store.card_count(&physics).unwrap(), 2);
    assert!(env.plan(&before_both).unwrap().summary().is_noop());

    // Merged into a deck archived since: the file has the target live and
    // not archived, and the merged deck live with its card.
    let (mut env, maths, physics, _) = env_with_decks();
    env.store.unarchive_deck(&maths).unwrap();
    let before = env.export();
    env.store.merge_deck(&maths, &physics).unwrap();
    let archived = env.store.archive_deck(&physics).unwrap().archived_at;
    let plan = env.plan(&before).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    env.import(&before).unwrap();
    assert_eq!(archived_at(&env.store, &physics), archived);
    assert_eq!(archived_at(&env.store, &maths), None);
    assert_eq!(env.store.card_count(&physics).unwrap(), 2);
    assert_eq!(env.store.card_count(&maths).unwrap(), 0);
    // The archived deck is out of the deck list; the revived one is in it.
    let listed: Vec<String> = env
        .store
        .list_decks(T_EXPORT, T_EXPORT)
        .unwrap()
        .into_iter()
        .map(|d| d.deck.id)
        .collect();
    assert_eq!(listed, [maths]);
    // A file where the archived target holds the merged cards (made after
    // the merge and archive) into an empty database: new decks keep the
    // file's flags.
    let after = env.export();
    let mut fresh = Env::new();
    fresh.import(&after).unwrap();
    assert_eq!(archived_at(&fresh.store, &physics), archived);
    assert_eq!(fresh.store.card_count(&physics).unwrap(), 2);
    assert_eq!(fresh.export(), after);
}
