use super::*;
use crate::test_support::T0;

const MISSING: &str = "01920000-0000-7000-8000-00000000dead";

fn store_with_cards() -> (Store, String, String) {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Maths", T0).unwrap().id;
    let a = store.add_card_at(&deck, "a", "", T0).unwrap().id;
    let b = store.add_card_at(&deck, "b", "", T0).unwrap().id;
    (store, a, b)
}

fn names(tags: &[Tag]) -> Vec<&str> {
    tags.iter().map(|t| t.name.as_str()).collect()
}

#[test]
fn a_tag_is_found_by_its_name_key() {
    let (mut store, a, b) = store_with_cards();
    let maths = store.add_tag_at(&a, "  Maths ", T0).unwrap();
    assert_eq!(maths.name, "Maths");
    // Another spelling of the same key reuses the tag and its spelling.
    let again = store.add_tag_at(&b, "maths", T0).unwrap();
    assert_eq!(again, maths);
    let upper = store.add_tag_at(&b, "MATHS", T0).unwrap();
    assert_eq!(upper, maths);
    let e1 = store.add_tag_at(&a, "\u{C9}conomie", T0).unwrap();
    let e2 = store.add_tag_at(&b, "E\u{301}conomie", T0).unwrap();
    assert_eq!(e1, e2);
    assert_eq!(names(&store.tags().unwrap()), ["Maths", "\u{C9}conomie"]);
    assert_eq!(
        names(&store.card_tags(&a).unwrap()),
        ["Maths", "\u{C9}conomie"]
    );
    assert_eq!(
        names(&store.card_tags(&b).unwrap()),
        ["Maths", "\u{C9}conomie"]
    );
}

#[test]
fn adding_a_tag_twice_changes_nothing_and_leaves_the_card_alone() {
    let (mut store, a, _) = store_with_cards();
    let before = store.card(&a).unwrap();
    store.add_tag_at(&a, "x", T0 + 5).unwrap();
    store.add_tag_at(&a, "x", T0 + 6).unwrap();
    assert_eq!(store.card_tags(&a).unwrap().len(), 1);
    assert_eq!(store.card(&a).unwrap(), before);
}

#[test]
fn removing_a_tag_keeps_the_tag_and_its_id() {
    let (mut store, a, b) = store_with_cards();
    let x = store.add_tag_at(&a, "x", T0).unwrap();
    store.add_tag_at(&b, "x", T0).unwrap();
    assert!(store.remove_tag(&a, &x.id).unwrap());
    assert!(!store.remove_tag(&a, &x.id).unwrap());
    assert_eq!(store.card_tags(&a).unwrap(), []);
    assert_eq!(store.card_tags(&b).unwrap(), std::slice::from_ref(&x));
    store.remove_tag(&b, &x.id).unwrap();
    // Unused, but still there: the same name gets the same id back.
    assert_eq!(store.tags().unwrap(), std::slice::from_ref(&x));
    assert_eq!(store.add_tag_at(&a, "X", T0).unwrap(), x);
}

#[test]
fn invalid_names_and_missing_rows_are_refused_without_writing() {
    let (mut store, a, _) = store_with_cards();
    assert!(matches!(
        store.add_tag_at(&a, " ", T0),
        Err(Error::EmptyTagName)
    ));
    assert!(matches!(
        store.add_tag_at(&a, "a,b", T0),
        Err(Error::TagNameHasComma)
    ));
    assert!(matches!(
        store.add_tag_at(&a, "a\0", T0),
        Err(Error::ContainsNul)
    ));
    assert!(matches!(
        store.add_tag_at(&a, &"x".repeat(257), T0),
        Err(Error::TagNameTooLong { .. })
    ));
    assert!(matches!(
        store.add_tag_at(MISSING, "x", T0),
        Err(Error::CardNotFound(_))
    ));
    assert_eq!(store.tags().unwrap(), []);
    assert!(matches!(
        store.remove_tag(&a, MISSING),
        Err(Error::TagNotFound(_))
    ));
    assert!(matches!(
        store.card_tags(MISSING),
        Err(Error::CardNotFound(_))
    ));
}

#[test]
fn a_deleted_card_takes_no_tags_and_keeps_its_own() {
    let (mut store, a, _) = store_with_cards();
    let x = store.add_tag_at(&a, "x", T0).unwrap();
    store.delete_card_at(&a, T0 + 1).unwrap();
    assert!(matches!(
        store.add_tag_at(&a, "y", T0),
        Err(Error::CardNotFound(_))
    ));
    assert!(matches!(
        store.remove_tag(&a, &x.id),
        Err(Error::CardNotFound(_))
    ));
    store.undelete_card(&a).unwrap();
    assert_eq!(store.card_tags(&a).unwrap(), [x]);
}

#[test]
fn ids_of_tags_increase_and_differ_from_other_ids() {
    let (mut store, a, _) = store_with_cards();
    let x = store.add_tag_at(&a, "x", T0).unwrap();
    let y = store.add_tag_at(&a, "y", T0).unwrap();
    assert!(crate::id::is_uuid_v7(&x.id));
    assert!(y.id > x.id);
    assert_ne!(x.id, a);
}
