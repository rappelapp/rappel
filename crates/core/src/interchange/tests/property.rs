//! Property.

use super::*;

#[derive(Debug, Clone)]
pub(super) enum Op {
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
    /// Undoes the card's latest surviving review or reset.
    Undo(usize),
    Reset(usize),
    AddTag(usize, usize),
    RemoveTag(usize, usize),
    SetMaxNew(u32),
    /// Move to Deck… of one card.
    MoveCards(usize, usize),
    /// Merge Into…: the first deck into the second.
    MergeDeck(usize, usize),
    /// Archive (ADR 0068).
    Archive(usize),
    /// Unarchive.
    Unarchive(usize),
}

/// Names that collide by key, so renames and creates sometimes fail.
const NAMES: [&str; 5] = ["Maths", "maths", "Physique", "PHYSIQUE", "Ελληνικά"];
const TEXTS: [&str; 4] = ["q", "two\nlines", "\"quoted\" & <b>", "√2 ≈ 1.414"];
/// Tag names that collide by key, so some adds reuse a tag.
const TAGS: [&str; 4] = ["algebra", "Algebra", "géométrie", "Unused"];

pub(super) fn op() -> impl Strategy<Value = Op> {
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
        2 => i().prop_map(Op::Undo),
        1 => i().prop_map(Op::Reset),
        2 => (i(), 0..TAGS.len()).prop_map(|(c, t)| Op::AddTag(c, t)),
        1 => (i(), i()).prop_map(|(c, t)| Op::RemoveTag(c, t)),
        1 => (0u32..30).prop_map(Op::SetMaxNew),
        2 => (i(), i()).prop_map(|(c, d)| Op::MoveCards(c, d)),
        1 => (i(), i()).prop_map(|(a, b)| Op::MergeDeck(a, b)),
        1 => i().prop_map(Op::Archive),
        1 => i().prop_map(Op::Unarchive),
    ]
}

/// The ids `sql` reads from `store`.
fn ids(store: &Store, sql: &str) -> Vec<String> {
    store
        .conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Runs `ops` on `store` through its public API, ignoring the refusals
/// (duplicate names, deleted targets): they are part of a random history.
/// The decks and cards picked from are those the store has, in the order
/// they were made, then those `ops` make, so a second run goes on with the
/// history of the first (an export from before it is then older).
pub(super) fn run(store: &mut Store, ops: &[Op]) {
    let mut decks = ids(store, "SELECT id FROM decks ORDER BY rowid");
    let mut cards = ids(store, "SELECT id FROM cards ORDER BY seq");
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
            Op::Undo(c) => {
                if let Some(c) = pick(&cards, c) {
                    let log = store.review_events(&c).unwrap();
                    let latest = crate::scheduling::surviving(&log)
                        .unwrap()
                        .last()
                        .map(|e| e.id.clone());
                    if let Some(id) = latest {
                        let _ = store.undo(&c, &id, clock);
                    }
                }
            }
            Op::Reset(c) => {
                if let Some(c) = pick(&cards, c) {
                    let _ = store.reset_card(&c, clock);
                }
            }
            Op::AddTag(c, t) => {
                if let Some(c) = pick(&cards, c) {
                    let _ = store.add_tag(&c, TAGS[t]);
                }
            }
            Op::RemoveTag(c, t) => {
                let tags: Vec<String> = store.tags().unwrap().into_iter().map(|t| t.id).collect();
                if let (Some(c), Some(t)) = (pick(&cards, c), pick(&tags, t)) {
                    let _ = store.remove_tag(&c, &t);
                }
            }
            Op::SetMaxNew(n) => store.set_max_new_per_day(n).unwrap(),
            Op::MoveCards(..) | Op::MergeDeck(..) | Op::Archive(_) | Op::Unarchive(_) => {
                reorganise(store, &decks, &cards, op);
            }
        }
    }
}

/// The ops of [`run`] that reorganise decks: Move to Deck…, Merge Into…,
/// Archive and Unarchive, on the decks and cards picked from.
fn reorganise(store: &mut Store, decks: &[String], cards: &[String], op: &Op) {
    let pick = |v: &[String], i: usize| (!v.is_empty()).then(|| v[i % v.len()].clone());
    match *op {
        Op::MoveCards(c, d) => {
            if let (Some(c), Some(d)) = (pick(cards, c), pick(decks, d)) {
                let _ = store.move_cards(&[c], &d);
            }
        }
        Op::MergeDeck(a, b) => {
            if let (Some(a), Some(b)) = (pick(decks, a), pick(decks, b)) {
                let _ = store.merge_deck(&a, &b);
            }
        }
        Op::Archive(d) => {
            if let Some(d) = pick(decks, d) {
                let _ = store.archive_deck(&d);
            }
        }
        Op::Unarchive(d) => {
            if let Some(d) = pick(decks, d) {
                let _ = store.unarchive_deck(&d);
            }
        }
        _ => {}
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

/// The writes that only reorganise: new decks, moves, merges, deck and
/// card deletes and their undos, archiving and unarchiving. An export made
/// before any of them conflicts only on a deck name taken since. Also the
/// later writes of `streaming.rs`'s comparison with the old reader, so it
/// sees cards moved since and deleted with their new deck.
pub(super) fn reorganisation() -> impl Strategy<Value = Op> {
    let i = || 0usize..16;
    prop_oneof![
        2 => i().prop_map(Op::Archive),
        1 => i().prop_map(Op::Unarchive),
        2 => (0..NAMES.len()).prop_map(Op::CreateDeck),
        4 => (i(), i()).prop_map(|(c, d)| Op::MoveCards(c, d)),
        2 => (i(), i()).prop_map(|(a, b)| Op::MergeDeck(a, b)),
        3 => i().prop_map(Op::DeleteDeck),
        1 => i().prop_map(Op::UndeleteDeck),
        1 => i().prop_map(Op::DeleteCard),
        1 => i().prop_map(Op::UndeleteCard),
    ]
}

/// Each live deck's id and `archived_at`.
fn archive_flags(store: &Store) -> Vec<(String, Option<i64>)> {
    store
        .conn
        .prepare("SELECT id, archived_at FROM decks WHERE deleted_at IS NULL ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The ids of the live cards and the deck each is in.
fn live_cards(store: &Store) -> Vec<(String, String)> {
    store
        .conn
        .prepare("SELECT id, deck_id FROM cards WHERE deleted_at IS NULL ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// An export made before cards were moved, decks merged, deleted or
    /// brought back, archived or unarchived imports into that database
    /// without a conflict (ADR 0067, 0068): every card live before stays in
    /// its deck, every live deck keeps its `archived_at`, no live card is
    /// left in a deleted deck, the summary counts the cards that stay in
    /// another deck than the file's, and importing again changes nothing.
    #[test]
    fn an_older_export_imports_after_a_reorganisation(
        ops in prop::collection::vec(op(), 0..80),
        later in prop::collection::vec(reorganisation(), 1..30),
    ) {
        let mut env = Env::new();
        run(&mut env.store, &ops);
        let old = env.export();
        run(&mut env.store, &later);
        let live = live_cards(&env.store);
        let flags = archive_flags(&env.store);
        let decks_of_file: std::collections::HashMap<String, String> = json_of(&old)["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap().to_owned(),
                    c["deck_id"].as_str().unwrap().to_owned(),
                )
            })
            .collect();

        let plan = env.plan(&old).unwrap();
        // The only conflict left is a deck the file has live whose name a
        // live deck took since (a delete, a new deck, an undo): the
        // existing rule of SPEC §4.1, not about where cards are.
        let conflicts = &plan.summary().conflicts;
        prop_assert!(
            conflicts.iter().all(|c| matches!(c, Conflict::DeckNameTaken { .. })),
            "{:?}",
            conflicts
        );
        prop_assume!(conflicts.is_empty());
        env.store.apply_import_at(&plan, T_EXPORT).unwrap();
        let after = live_cards(&env.store);
        // Every card live before is still live, in the same deck.
        for card in &live {
            prop_assert!(after.contains(card), "{:?}", card);
        }
        // Every deck live before keeps its own flag; a deck the import
        // revives keeps the database's too.
        let flags_after = archive_flags(&env.store);
        for flag in &flags {
            prop_assert!(flags_after.contains(flag), "{:?}", flag);
        }
        // The cards that stay live in another deck than the file's.
        let kept = after
            .iter()
            .filter(|(id, deck)| decks_of_file.get(id).is_some_and(|d| d != deck))
            .count();
        prop_assert_eq!(plan.summary().cards_kept_in_their_deck, kept);
        // No live card in a deleted deck.
        let orphans: i64 = env.store.conn.query_row(
            "SELECT count(*) FROM cards c JOIN decks d ON d.id = c.deck_id
             WHERE c.deleted_at IS NULL AND d.deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        ).unwrap();
        prop_assert_eq!(orphans, 0);
        let again = env.plan(&old).unwrap();
        prop_assert!(again.summary().is_noop(), "{:?}", again.summary());
    }
}
