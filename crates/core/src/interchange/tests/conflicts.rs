//! Conflicts.

use super::*;

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
