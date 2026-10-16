//! Property.

use super::*;

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
