use super::*;
use crate::test_support::T0;

/// A store with the decks "Spanish" and "French".
fn two_decks() -> (Store, String, String) {
    let mut store = Store::in_memory();
    let spanish = store.create_deck_at("Spanish", T0).unwrap().id;
    let french = store.create_deck_at("French", T0).unwrap().id;
    (store, spanish, french)
}

fn add(store: &mut Store, deck: &str, front: &str) -> String {
    store.add_card_at(deck, front, "back", T0).unwrap().id
}

/// The ids of the cards found for `front`, in their order.
fn found(store: &Store, front: &str, except: Option<&str>) -> Vec<String> {
    store
        .cards_with_front(front, except)
        .unwrap()
        .into_iter()
        .map(|s| s.card_id)
        .collect()
}

#[test]
fn the_key_is_the_folded_plain_text_on_one_line() {
    assert_eq!(front_key("  Hola,\n\n  **MUNDO**  "), "hola, mundo");
    assert_eq!(front_key("Économie\t é"), "economie e");
    assert_eq!(front_key("$x^2$ and `code`"), "x^2 and code");
    assert_eq!(
        front_key("[link](https://example.org) ![alt](media:a.png)"),
        "link alt"
    );
    assert_eq!(front_key(""), "");
    assert_eq!(front_key(" \n\t "), "");
    // Punctuation stays.
    assert_ne!(front_key("Hola!"), front_key("Hola"));
}

#[test]
fn the_phrase_is_quoted_and_never_syntax() {
    assert_eq!(
        phrase_query("hola mundo").as_deref(),
        Some("front : ^ \"hola mundo\"")
    );
    assert_eq!(
        phrase_query("say \"hi\"").as_deref(),
        Some("front : ^ \"say \"\"hi\"\"\"")
    );
    // Leading punctuation holds no token and is left out.
    assert_eq!(phrase_query("¿qué?").as_deref(), Some("front : ^ \"qué?\""));
    assert_eq!(phrase_query("a\0b").as_deref(), Some("front : ^ \"a b\""));
    // No letter or digit: nothing to look up.
    assert_eq!(phrase_query(""), None);
    assert_eq!(phrase_query("+ - * ?"), None);
}

#[test]
fn a_long_key_is_cut_and_ends_in_a_prefix() {
    let key = "é".repeat(MAX_PHRASE_BYTES);
    let query = phrase_query(&key).unwrap();
    // Cut between two characters, at most the budget.
    let inner = query
        .strip_prefix("front : ^ \"")
        .and_then(|q| q.strip_suffix("\"*"))
        .unwrap();
    assert_eq!(inner, "é".repeat(MAX_PHRASE_BYTES / 2));
    // Padding before the first letter is free.
    let padded = format!("{}{}", "-".repeat(10_000), "word");
    assert_eq!(phrase_query(&padded).as_deref(), Some("front : ^ \"word\""));
}

#[test]
fn case_accents_and_whitespace_do_not_count() {
    let (mut store, spanish, _) = two_decks();
    let id = add(&mut store, &spanish, "El  Niño\n");
    for typed in [
        "el niño",
        "EL NINO",
        "  el\tnino  ",
        "el\n\nniño",
        "Él Ñiño",
    ] {
        assert_eq!(found(&store, typed, None), [id.as_str()], "{typed:?}");
    }
    for typed in ["el niños", "niño", "el", "el niño!", "elnino"] {
        assert_eq!(
            found(&store, typed, None),
            Vec::<String>::new(),
            "{typed:?}"
        );
    }
}

#[test]
fn markdown_and_math_are_compared_as_their_text() {
    let (mut store, spanish, _) = two_decks();
    let bold = add(&mut store, &spanish, "**el** _gato_");
    let math = add(&mut store, &spanish, "$x^2$");
    assert_eq!(found(&store, "el gato", None), [bold.as_str()]);
    assert_eq!(found(&store, "*el* **gato**", None), [bold]);
    assert_eq!(found(&store, "x^2", None), [math.as_str()]);
    assert_eq!(found(&store, "$$x^2$$", None), [math]);
}

#[test]
fn deleted_cards_and_decks_are_left_out() {
    let (mut store, spanish, french) = two_decks();
    let gone = add(&mut store, &spanish, "hola");
    let kept = add(&mut store, &french, "hola");
    store.delete_card_at(&gone, T0 + 1).unwrap();
    assert_eq!(found(&store, "hola", None), [kept]);
    store.delete_deck_at(&french, T0 + 2).unwrap();
    assert_eq!(found(&store, "hola", None), Vec::<String>::new());
}

#[test]
fn archived_decks_are_searched_flagged_and_listed_last() {
    let (mut store, spanish, french) = two_decks();
    let in_spanish = add(&mut store, &spanish, "hola");
    let in_french = add(&mut store, &french, "hola");
    store.archive_deck_at(&spanish, T0 + 1).unwrap();
    let same = store.cards_with_front("Hola", None).unwrap();
    assert_eq!(
        same,
        [
            SameFront {
                card_id: in_french,
                deck_id: french,
                deck_name: "French".to_owned(),
                archived: false,
            },
            SameFront {
                card_id: in_spanish,
                deck_id: spanish,
                deck_name: "Spanish".to_owned(),
                archived: true,
            },
        ]
    );
}

#[test]
fn the_newest_comes_first_and_the_edited_card_is_left_out() {
    let (mut store, spanish, _) = two_decks();
    let first = add(&mut store, &spanish, "hola");
    let second = add(&mut store, &spanish, "hola");
    assert_eq!(found(&store, "hola", None), [second.clone(), first.clone()]);
    assert_eq!(found(&store, "hola", Some(&second)), [first.as_str()]);
    assert_eq!(found(&store, "hola", Some(&first)), [second]);
    // An id that is no card leaves nothing out.
    assert_eq!(found(&store, "hola", Some("nope")).len(), 2);
}

#[test]
fn an_edit_moves_the_card_out_of_and_into_the_matches() {
    let (mut store, spanish, _) = two_decks();
    let id = add(&mut store, &spanish, "hola");
    store.edit_card_at(&id, "adiós", "back", T0 + 1).unwrap();
    assert_eq!(found(&store, "hola", None), Vec::<String>::new());
    assert_eq!(found(&store, "adios", None), [id]);
}

#[test]
fn a_phrase_inside_a_longer_front_is_not_the_same_front() {
    let (mut store, spanish, _) = two_decks();
    add(&mut store, &spanish, "el gato negro");
    add(&mut store, &spanish, "un gato");
    add(&mut store, &spanish, "gatos");
    let exact = add(&mut store, &spanish, "gato");
    assert_eq!(found(&store, "gato", None), [exact]);
    assert_eq!(found(&store, "el gato", None), Vec::<String>::new());
}

#[test]
fn fts_syntax_in_a_front_is_only_text() {
    let (mut store, spanish, _) = two_decks();
    let fronts = [
        "say \"hi\"",
        "a* b",
        "x - y",
        "NEAR(a b)",
        "a OR b",
        "front: back",
        "(a) AND NOT (b)",
        "^start",
        "\"",
        "\"\"\"",
        "col:umn \"un closed",
    ];
    let ids: Vec<String> = fronts
        .iter()
        .map(|f| add(&mut store, &spanish, f))
        .collect();
    for (front, id) in fronts.iter().zip(&ids) {
        let same = found(&store, front, None);
        if front.contains(char::is_alphanumeric) {
            assert_eq!(same, [id.as_str()], "{front:?}");
        } else {
            // No token to look up: no note, never an error.
            assert_eq!(same, Vec::<String>::new(), "{front:?}");
        }
    }
    // The operators are words, not syntax: "a OR b" finds no "a".
    assert_eq!(found(&store, "a", None), Vec::<String>::new());
}

#[test]
fn a_front_without_a_token_finds_nothing() {
    let (mut store, spanish, _) = two_decks();
    for front in ["?", "+ - *", "$+$", "…", "\u{301}"] {
        add(&mut store, &spanish, front);
        assert_eq!(
            found(&store, front, None),
            Vec::<String>::new(),
            "{front:?}"
        );
    }
    assert_eq!(found(&store, "", None), Vec::<String>::new());
    assert_eq!(found(&store, "  \n ", None), Vec::<String>::new());
}

#[test]
fn odd_characters_never_fail_the_query() {
    let (mut store, spanish, _) = two_decks();
    // Characters Rust calls alphanumeric that FTS5 may read as no token,
    // private use joined to a word, a lone combining mark.
    for front in ["ⓐ", "\u{10ffff}", "card\u{10ffff}", "Ⅻ", "x\u{301}y"] {
        let id = add(&mut store, &spanish, front);
        let same = store.cards_with_front(front, None).unwrap();
        assert!(
            same.is_empty() || same.iter().map(|s| &s.card_id).eq([&id]),
            "{front:?}: {same:?}"
        );
    }
    // A card never holds NUL, but the editor's text may: it is a
    // character no front has, never an error.
    add(&mut store, &spanish, "a b");
    for typed in ["a\0b", "a \0", "\0a b\0", "\0"] {
        assert_eq!(
            found(&store, typed, None),
            Vec::<String>::new(),
            "{typed:?}"
        );
    }
}

#[test]
fn long_fronts_are_compared_in_full() {
    let (mut store, spanish, _) = two_decks();
    let words = "palabra ".repeat(200);
    let long = add(&mut store, &spanish, &words);
    let longer = add(&mut store, &spanish, &format!("{words} más"));
    let cut_word = format!("{}{}", "x".repeat(MAX_PHRASE_BYTES - 2), "yyyy");
    let cut = add(&mut store, &spanish, &cut_word);
    assert_eq!(found(&store, &words.to_uppercase(), None), [long]);
    assert_eq!(found(&store, &format!("{words}   MÁS"), None), [longer]);
    // The phrase cuts this word in two; its prefix still finds it.
    assert_eq!(found(&store, &cut_word, None), [cut]);
    assert_eq!(
        found(&store, &"x".repeat(MAX_PHRASE_BYTES + 2), None),
        Vec::<String>::new()
    );
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// Any text typed is a valid lookup.
    #[test]
    fn any_input_is_a_valid_lookup(text in proptest::prelude::any::<String>()) {
        let (mut store, spanish, _) = two_decks();
        add(&mut store, &spanish, "alpha beta");
        store.cards_with_front(&text, None).unwrap();
    }
}

#[test]
fn a_front_over_the_cap_is_not_looked_up() {
    let (mut store, spanish, _) = two_decks();
    let longest = format!("{} end", "x".repeat(MAX_SAME_FRONT_BYTES - 4));
    let over = format!("{longest}s");
    assert_eq!(longest.len(), MAX_SAME_FRONT_BYTES);
    let id = add(&mut store, &spanish, &longest);
    add(&mut store, &spanish, &over);
    assert_eq!(found(&store, &longest, None), [id.as_str()]);
    assert_eq!(found(&store, &over, None), Vec::<String>::new());
}

#[test]
fn a_private_use_character_starts_a_token() {
    let (mut store, spanish, _) = two_decks();
    // unicode61 reads private use (U+E000) as a token character, joined to
    // the letters after it; Rust does not call it alphanumeric.
    let joined = add(&mut store, &spanish, "\u{E000}abc");
    let alone = add(&mut store, &spanish, "\u{F0000}");
    let plain = add(&mut store, &spanish, "abc");
    assert_eq!(found(&store, "\u{E000}ABC", None), [joined.as_str()]);
    assert_eq!(found(&store, "  \u{F0000} ", None), [alone.as_str()]);
    assert_eq!(found(&store, "abc", None), [plain.as_str()]);
    assert_eq!(
        phrase_query("-\u{E000}abc").as_deref(),
        Some("front : ^ \"\u{E000}abc\"")
    );
    assert_eq!(
        phrase_query("\u{10FFFD}").as_deref(),
        Some("front : ^ \"\u{10FFFD}\"")
    );
}

#[test]
fn the_phrase_is_anchored_to_the_start_of_the_front() {
    let (mut store, spanish, _) = two_decks();
    add(&mut store, &spanish, "un gato");
    add(&mut store, &spanish, "gato negro");
    let exact = add(&mut store, &spanish, "¿gato");
    let fronts = add(&mut store, &spanish, "gato");
    // Only "gato" itself, never a front holding it further on.
    assert_eq!(found(&store, "gato", None), [fronts.as_str()]);
    // Leading punctuation is no token: anchored, a front starting with it
    // is still found.
    assert_eq!(found(&store, "¿Gato", None), [exact.as_str()]);
}

/// The key the property test checks the store against, written here from
/// its definition rather than by calling [`front_key`]: the search
/// index's text with every run of whitespace as one space.
fn oracle_key(front: &str) -> String {
    crate::content::search_text(front)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What `typed` is made from in the property test: one of the fronts as
/// it is, or changed in a way that keeps its key (upper case, accents
/// decomposed, more whitespace around and inside, bold), or unrelated text.
#[derive(Debug, Clone)]
enum Typed {
    Front {
        index: usize,
        upper: bool,
        nfd: bool,
        spaced: bool,
        bold: bool,
    },
    Other(String),
}

/// A tiny alphabet, so fronts often share a key: case, an accent, a
/// private-use character, spaces, newlines, a tab, punctuation.
const ALPHABET: &str = "[aAéÉ\u{E000} \n\t!]{1,4}";

fn typed_strategy() -> impl proptest::strategy::Strategy<Value = Typed> {
    use proptest::prelude::*;
    prop_oneof![
        3 => (0..12usize, any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
            |(index, upper, nfd, spaced, bold)| Typed::Front { index, upper, nfd, spaced, bold }
        ),
        1 => ALPHABET.prop_map(Typed::Other),
    ]
}

fn make_typed(typed: &Typed, fronts: &[String]) -> String {
    use unicode_normalization::UnicodeNormalization;
    match typed {
        Typed::Other(text) => text.clone(),
        Typed::Front {
            index,
            upper,
            nfd,
            spaced,
            bold,
        } => {
            let mut text = fronts[index % fronts.len()].clone();
            if *upper {
                text = text.to_uppercase();
            }
            if *nfd {
                text = text.nfd().collect();
            }
            if *spaced {
                text = format!(" \n {} \t", text.replace(' ', "  \t "));
            }
            if *bold && !text.trim().is_empty() {
                text = format!("**{}**", text.trim());
            }
            text
        }
    }
}

/// The lookup finds exactly the cards whose key equals the typed front's,
/// by the oracle's key; a front without a token character finds nothing.
/// Fronts and typed text come from a tiny alphabet, and the typed text is
/// usually one of the fronts changed in ways that keep its key, so most
/// cases have matches: at least half must, checked at the end, so a bug
/// that misses matches (a key without collapsed whitespace, say) fails.
#[test]
fn the_lookup_finds_what_comparing_every_key_finds() {
    use std::cell::Cell;

    use proptest::prelude::*;
    use proptest::test_runner::{Config, TestRunner};

    const CASES: u32 = 256;
    let mut runner = TestRunner::new(Config::with_cases(CASES));
    let with_matches = Cell::new(0u32);
    let strategy = (proptest::collection::vec(ALPHABET, 1..12), typed_strategy());
    runner
        .run(&strategy, |(fronts, typed)| {
            let typed = make_typed(&typed, &fronts);
            let (mut store, spanish, _) = two_decks();
            let key = oracle_key(&typed);
            let mut expected = Vec::new();
            for front in &fronts {
                if front.trim().is_empty() {
                    continue;
                }
                let id = add(&mut store, &spanish, front);
                if oracle_key(front) == key && key.contains(starts_token) {
                    expected.push(id);
                }
            }
            // Newest first.
            expected.reverse();
            if !expected.is_empty() {
                with_matches.set(with_matches.get() + 1);
            }
            prop_assert_eq!(found(&store, &typed, None), expected);
            Ok(())
        })
        .unwrap();
    assert!(
        with_matches.get() >= CASES / 2,
        "only {} of {CASES} cases had a match",
        with_matches.get()
    );
}
