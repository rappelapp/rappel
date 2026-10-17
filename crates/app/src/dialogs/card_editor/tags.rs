//! The editor's tags field, decided without GTK (SPEC §4.4, ADR 0031): what
//! the user is typing, which tags to suggest, what accepting one does to
//! the text, what the field's error says, and how the highlight moves.
//!
//! The text is a comma-separated list; the cursor is a character offset, as
//! `GtkEditable` has it.

use std::ops::Range;

use rappel_core::Error;
use rappel_core::domain::{
    MAX_TAG_NAME_BYTES, Tag, TagFieldError, name_key, parse_tag_field, search_key,
};

use crate::ui::text::group_digits;

/// The most suggestions shown at once.
pub const MAX_SUGGESTIONS: usize = 8;

/// The name being typed: the piece of `text` between the commas around
/// `cursor` (a character offset), without the spaces at its ends, as a byte
/// range of `text`. Empty (and placed at the cursor) when the piece is blank.
pub fn name_at(text: &str, cursor: usize) -> Range<usize> {
    let cursor_byte = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(i, _)| i);
    let start = text[..cursor_byte].rfind(',').map_or(0, |i| i + 1);
    let end = text[cursor_byte..]
        .find(',')
        .map_or(text.len(), |i| cursor_byte + i);
    let piece = &text[start..end];
    let trimmed = piece.trim();
    if trimmed.is_empty() {
        return cursor_byte..cursor_byte;
    }
    let lead = piece.len() - piece.trim_start().len();
    let from = start + lead;
    from..from + trimmed.len()
}

/// Up to [`MAX_SUGGESTIONS`] of `all` for the name typed at `cursor`.
/// A tag matches when its [`search_key`] contains the typed name's, so
/// "eco" finds "Économie" in any case. Best first: tags that start with
/// the name, then those with a word that does, then the rest; each group by
/// key, then name. Tags already in the field (by [`name_key`]) are left out.
pub fn suggestions(all: &[Tag], text: &str, cursor: usize) -> Vec<Tag> {
    let typed = name_at(text, cursor);
    let needle = search_key(&text[typed.clone()]);
    if needle.is_empty() {
        return Vec::new();
    }
    let typed_key = name_key(&text[typed.clone()]);
    let mut others = Vec::new();
    let mut at = 0;
    for piece in text.split(',') {
        let end = at + piece.len();
        // The piece being typed is not "already there".
        if !(at <= typed.start && typed.end <= end) {
            others.push(name_key(piece));
        }
        at = end + 1;
    }
    let mut found: Vec<(u8, String, &Tag)> = all
        .iter()
        .filter(|tag| !others.contains(&name_key(&tag.name)))
        .filter_map(|tag| {
            let key = search_key(&tag.name);
            // The tag the user typed in full comes first, so Tab cannot swap
            // it for a look-alike ("Économie" / "Economie" are two tags).
            let rank = if name_key(&tag.name) == typed_key {
                0
            } else if key.starts_with(&needle) {
                1
            } else if key
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word.starts_with(&needle))
            {
                2
            } else if key.contains(&needle) {
                3
            } else {
                return None;
            };
            Some((rank, key, tag))
        })
        .collect();
    found.sort_by(|a, b| (a.0, &a.1, &a.2.name).cmp(&(b.0, &b.1, &b.2.name)));
    found
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, tag)| tag.clone())
        .collect()
}

/// `text` with the name at `typed` replaced by `name`, followed by ", " ready
/// for the next one, and the cursor (a character offset) after it. Spaces
/// after the comma that ended the piece are not doubled.
pub fn accept(text: &str, typed: Range<usize>, name: &str) -> (String, usize) {
    let before = text[..typed.start].trim_end();
    let tail = text[typed.end..].trim_start();
    let tail = tail.strip_prefix(',').unwrap_or(tail).trim_start();
    let head = if before.is_empty() {
        name.to_owned()
    } else {
        format!("{before} {name}")
    };
    let head = format!("{head}, ");
    let cursor = head.chars().count();
    (format!("{head}{tail}"), cursor)
}

/// The names of `tags` as the field shows them: "maths, algebra".
pub fn field_text(tags: &[Tag]) -> String {
    tags.iter()
        .map(|tag| tag.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the field holds: the names to save, or the error to show under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsCheck {
    /// The names, in the order typed, when they are all valid.
    pub names: Vec<String>,
    /// Why they cannot be saved.
    pub error: Option<String>,
}

/// Checks the field's text with the store's rules (`parse_tag_field`).
pub fn check_tags(text: &str) -> TagsCheck {
    match parse_tag_field(text) {
        Ok(names) => TagsCheck { names, error: None },
        Err(err) => TagsCheck {
            names: Vec::new(),
            error: Some(error_text(&err)),
        },
    }
}

fn error_text(err: &TagFieldError) -> String {
    let n = err.position;
    match &err.error {
        Error::EmptyTagName => format!("Tag {n} is empty: remove the extra comma"),
        Error::TagNameTooLong { .. } => format!(
            "Tag {n} is too long (the limit is {} bytes)",
            group_digits(MAX_TAG_NAME_BYTES)
        ),
        Error::ContainsNul => format!("Tag {n} contains a null character"),
        Error::TagNameHasComma => format!("Tag {n} contains a comma"),
        other => format!("Tag {n}: {other}"),
    }
}

/// The names of `names` as keys, in a fixed order, to tell whether two fields
/// hold the same tags whatever their spelling, spacing or order.
pub fn tag_keys(names: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = names.iter().map(|n| name_key(n)).collect();
    keys.sort();
    keys.dedup();
    keys
}

/// A key that moves the highlight in the suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
}

/// The highlighted row after `movement`, among `len` rows. It stops at the
/// first and the last. `None` when there are no rows.
pub fn highlight_after(movement: Move, current: Option<usize>, len: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (movement, current) {
        (Move::Down, None) => 0,
        (Move::Down, Some(i)) => (i + 1).min(last),
        (Move::Up, None) => last,
        (Move::Up, Some(i)) => i.saturating_sub(1),
    })
}

/// What a screen reader is told when the highlight lands on `name`.
pub fn highlight_announcement(name: &str, index: usize, len: usize) -> String {
    format!("{name}, suggestion {} of {len}", index + 1)
}

/// A key the tags entry treats specially while suggestions show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestKey {
    Down,
    Up,
    Enter,
    Tab,
    Escape,
}

/// What a key does to the suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Move the highlight.
    Highlight(Move),
    /// Take suggestion `n` in place of the name being typed.
    Accept(usize),
    /// Close the list and keep the text.
    Close,
    /// Enter with nothing highlighted: close the list, keep the text, and
    /// let the entry have the key. A later Tab must not take a suggestion
    /// for the name just confirmed.
    KeepTyped,
    /// Not ours: the entry (or the dialog) handles the key.
    Pass,
}

/// The keys of the suggestions (ADR 0031). Nothing is highlighted until
/// Down or Up is pressed. **Tab** takes the highlighted suggestion, or the
/// first one if none is highlighted. **Enter** takes only a highlighted
/// suggestion; otherwise the typed text stays as it is and the list closes.
/// **Escape** closes the list. Without a list showing, every key is the
/// entry's own (Tab moves the focus on).
pub fn key_action(key: SuggestKey, showing: bool, highlighted: Option<usize>) -> KeyAction {
    if !showing {
        return KeyAction::Pass;
    }
    match key {
        SuggestKey::Down => KeyAction::Highlight(Move::Down),
        SuggestKey::Up => KeyAction::Highlight(Move::Up),
        SuggestKey::Enter => highlighted.map_or(KeyAction::KeepTyped, KeyAction::Accept),
        SuggestKey::Tab => KeyAction::Accept(highlighted.unwrap_or(0)),
        SuggestKey::Escape => KeyAction::Close,
    }
}

/// What a screen reader is told when suggestions appear.
pub fn suggestions_announcement(len: usize) -> String {
    format!(
        "{}: Tab takes the first, the arrow keys choose, Escape keeps what you typed",
        crate::ui::text::count(len, "suggestion", "suggestions")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            id: format!("id-{name}"),
            name: name.to_owned(),
        }
    }

    fn tags(names: &[&str]) -> Vec<Tag> {
        names.iter().map(|n| tag(n)).collect()
    }

    fn names(found: &[Tag]) -> Vec<&str> {
        found.iter().map(|t| t.name.as_str()).collect()
    }

    /// The name under the cursor, where `|` marks the cursor.
    fn typed(marked: &str) -> String {
        let cursor = marked.chars().position(|c| c == '|').unwrap();
        let text = marked.replacen('|', "", 1);
        text[name_at(&text, cursor)].to_owned()
    }

    #[test]
    fn the_name_is_the_piece_around_the_cursor() {
        assert_eq!(typed("|"), "");
        assert_eq!(typed("mat|"), "mat");
        assert_eq!(typed("ma|ths"), "maths");
        assert_eq!(typed("a, b|c , d"), "bc");
        assert_eq!(typed("a,| b"), "b");
        assert_eq!(typed("a, |"), "");
        assert_eq!(typed("a,|,b"), "");
        assert_eq!(typed("é, Éco|"), "Éco");
    }

    #[test]
    fn a_blank_piece_is_an_empty_range_at_the_cursor() {
        assert_eq!(name_at("a, ", 3), 3..3);
        assert_eq!(name_at("a,  ,b", 3), 3..3);
        assert_eq!(name_at("", 0), 0..0);
    }

    #[test]
    fn a_cursor_past_the_end_is_the_end() {
        assert_eq!(name_at("ab", 99), 0..2);
    }

    #[test]
    fn suggestions_ignore_accents_and_case() {
        let all = tags(&["Économie", "Ecologie", "history", "maths"]);
        for typed in ["eco", "ECO", "éco", "Éco"] {
            let found = suggestions(&all, typed, typed.chars().count());
            assert_eq!(names(&found), ["Ecologie", "Économie"], "{typed}");
        }
        assert_eq!(names(&suggestions(&all, "économie", 8)), ["Économie"]);
        assert_eq!(suggestions(&all, "", 0), []);
        assert_eq!(suggestions(&all, "a, ", 3), []);
        assert_eq!(suggestions(&all, "zzz", 3), []);
    }

    #[test]
    fn prefix_matches_come_before_word_starts_and_the_rest() {
        let all = tags(&["quartic", "linear algebra", "algebra", "Algèbre", "malgré"]);
        let found = suggestions(&all, "alg", 3);
        assert_eq!(
            names(&found),
            ["algebra", "Algèbre", "linear algebra", "malgré"]
        );
    }

    #[test]
    fn tags_already_in_the_field_are_left_out_but_not_the_one_typed() {
        let all = tags(&["maths", "Mathématiques", "math-art"]);
        let text = "MATHS, mat";
        let found = suggestions(&all, text, text.chars().count());
        assert_eq!(names(&found), ["math-art", "Mathématiques"]);
        // Typing the full name of a tag still offers it.
        let found = suggestions(&all, "maths", 5);
        assert_eq!(names(&found), ["maths"]);
        // Editing an earlier piece: only the other pieces are excluded.
        let found = suggestions(&all, "mat, maths", 3);
        assert_eq!(names(&found), ["math-art", "Mathématiques"]);
    }

    #[test]
    fn the_name_typed_in_full_comes_first() {
        // Two tags that differ only by an accent are two tags.
        let all = tags(&["Economie", "Économie", "Économies"]);
        let found = suggestions(&all, "Économie", 8);
        assert_eq!(names(&found), ["Économie", "Economie", "Économies"]);
        let found = suggestions(&all, "economie", 8);
        assert_eq!(names(&found), ["Economie", "Économie", "Économies"]);
    }

    #[test]
    fn at_most_eight_are_suggested() {
        let all: Vec<Tag> = (0..20).map(|i| tag(&format!("tag {i:02}"))).collect();
        let found = suggestions(&all, "tag", 3);
        assert_eq!(found.len(), MAX_SUGGESTIONS);
        assert_eq!(found[0].name, "tag 00");
    }

    #[test]
    fn accepting_replaces_the_name_and_makes_room_for_the_next() {
        let accept_at = |text: &str, cursor: usize, name: &str| {
            let range = name_at(text, cursor);
            accept(text, range, name)
        };
        assert_eq!(accept_at("alg", 3, "algebra"), ("algebra, ".to_owned(), 9));
        assert_eq!(
            accept_at("maths, al", 9, "algebra"),
            ("maths, algebra, ".to_owned(), 16)
        );
        assert_eq!(
            accept_at("maths,al", 8, "algebra"),
            ("maths, algebra, ".to_owned(), 16)
        );
        // In the middle: the rest follows, without a doubled comma.
        assert_eq!(
            accept_at("al, maths", 2, "Algèbre"),
            ("Algèbre, maths".to_owned(), 9)
        );
        assert_eq!(
            accept_at("a, al ,b", 5, "algebra"),
            ("a, algebra, b".to_owned(), 12)
        );
    }

    #[test]
    fn the_field_shows_the_names() {
        assert_eq!(field_text(&[]), "");
        assert_eq!(field_text(&tags(&["maths", "algebra"])), "maths, algebra");
    }

    #[test]
    fn the_field_is_checked_with_the_stores_rules() {
        let ok = check_tags(" maths,algebra , ");
        assert_eq!(ok.names, ["maths", "algebra"]);
        assert_eq!(ok.error, None);
        assert_eq!(check_tags("").names, Vec::<String>::new());
        assert_eq!(check_tags("").error, None);

        let message = |text: &str| check_tags(text).error;
        assert_eq!(
            message("a,,b").as_deref(),
            Some("Tag 2 is empty: remove the extra comma")
        );
        assert_eq!(
            message(&format!("a, {}", "x".repeat(257))).as_deref(),
            Some("Tag 2 is too long (the limit is 256 bytes)")
        );
        assert_eq!(
            message("a\0").as_deref(),
            Some("Tag 1 contains a null character")
        );
        assert_eq!(check_tags("a,,b").names, Vec::<String>::new());
    }

    #[test]
    fn keys_ignore_spelling_spacing_and_order() {
        let keys =
            |names: &[&str]| tag_keys(&names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>());
        assert_eq!(keys(&["Maths", "a"]), keys(&["a", "maths"]));
        assert_ne!(keys(&["a"]), keys(&["a", "b"]));
    }

    #[test]
    fn the_highlight_stops_at_both_ends() {
        use Move::{Down, Up};
        assert_eq!(highlight_after(Down, None, 0), None);
        assert_eq!(highlight_after(Up, Some(0), 0), None);
        assert_eq!(highlight_after(Down, None, 3), Some(0));
        assert_eq!(highlight_after(Down, Some(0), 3), Some(1));
        assert_eq!(highlight_after(Down, Some(2), 3), Some(2));
        assert_eq!(highlight_after(Up, Some(2), 3), Some(1));
        assert_eq!(highlight_after(Up, Some(0), 3), Some(0));
        assert_eq!(highlight_after(Up, None, 3), Some(2));
    }

    #[test]
    fn announcements_say_where_the_highlight_is() {
        assert_eq!(
            highlight_announcement("algebra", 0, 3),
            "algebra, suggestion 1 of 3"
        );
        assert_eq!(
            suggestions_announcement(1),
            "1 suggestion: Tab takes the first, the arrow keys choose, Escape keeps what you typed"
        );
        assert_eq!(
            suggestions_announcement(3),
            "3 suggestions: Tab takes the first, the arrow keys choose, Escape keeps what you typed"
        );
    }

    #[test]
    fn tab_takes_the_first_or_the_highlighted_suggestion() {
        use SuggestKey::Tab;
        assert_eq!(key_action(Tab, true, None), KeyAction::Accept(0));
        assert_eq!(key_action(Tab, true, Some(2)), KeyAction::Accept(2));
        // No list: Tab moves the focus on.
        assert_eq!(key_action(Tab, false, None), KeyAction::Pass);
    }

    #[test]
    fn enter_takes_only_a_highlighted_suggestion() {
        use SuggestKey::Enter;
        assert_eq!(key_action(Enter, true, None), KeyAction::KeepTyped);
        assert_eq!(key_action(Enter, true, Some(1)), KeyAction::Accept(1));
        assert_eq!(key_action(Enter, false, Some(1)), KeyAction::Pass);
    }

    #[test]
    fn the_arrows_highlight_and_escape_closes() {
        use SuggestKey::{Down, Escape, Up};
        assert_eq!(
            key_action(Down, true, None),
            KeyAction::Highlight(Move::Down)
        );
        assert_eq!(
            key_action(Up, true, Some(0)),
            KeyAction::Highlight(Move::Up)
        );
        assert_eq!(key_action(Escape, true, Some(0)), KeyAction::Close);
        for key in [Down, Up, Escape] {
            assert_eq!(key_action(key, false, None), KeyAction::Pass);
        }
    }
}
