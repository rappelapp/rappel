//! Ctrl+B / Ctrl+I in the card editor: wrapping a selection in Markdown
//! emphasis markers. Pure text, so the rules are tested without GTK.

use std::ops::Range;

/// The marker Ctrl+B puts around the selection.
pub const BOLD: &str = "**";
/// The marker Ctrl+I puts around the selection.
pub const ITALIC: &str = "*";

/// What replaces the selection, and what to select afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    /// The new text in place of the selection.
    pub text: String,
    /// The part of `text` to select, in characters (GTK text iterators
    /// count characters, not bytes). Empty means the cursor goes there.
    pub select: Range<usize>,
}

/// `selected` wrapped in `marker` on both sides, the selected text staying
/// selected. Nothing selected gives two markers with the cursor between
/// them, ready to type.
///
/// Whitespace at either edge of the selection stays outside the markers
/// (selecting `"word "` gives `"**word** "`): `CommonMark` does not read
/// `** word**` as bold. A selection across a blank line is wrapped as a
/// whole, which `CommonMark` does not read as emphasis: select one paragraph
/// at a time. Undoing is left to Ctrl+Z, which takes the whole wrap back in
/// one step.
pub fn wrap(selected: &str, marker: &str) -> Wrapped {
    let trimmed_start = selected.trim_start();
    let lead = &selected[..selected.len() - trimmed_start.len()];
    let core = trimmed_start.trim_end();
    let trail = &trimmed_start[core.len()..];

    let start = lead.chars().count() + marker.chars().count();
    let end = start + core.chars().count();
    Wrapped {
        text: format!("{lead}{marker}{core}{marker}{trail}"),
        select: start..end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(text: &str, select: Range<usize>) -> Wrapped {
        Wrapped {
            text: text.to_owned(),
            select,
        }
    }

    #[test]
    fn nothing_selected_puts_the_cursor_between_markers() {
        assert_eq!(wrap("", BOLD), wrapped("****", 2..2));
        assert_eq!(wrap("", ITALIC), wrapped("**", 1..1));
    }

    #[test]
    fn a_selection_is_wrapped_and_stays_selected() {
        assert_eq!(wrap("word", BOLD), wrapped("**word**", 2..6));
        assert_eq!(wrap("two words", ITALIC), wrapped("*two words*", 1..10));
    }

    #[test]
    fn whitespace_at_the_edges_stays_outside() {
        assert_eq!(wrap("word ", BOLD), wrapped("**word** ", 2..6));
        assert_eq!(wrap(" \tword\n", ITALIC), wrapped(" \t*word*\n", 3..7));
    }

    #[test]
    fn only_whitespace_gets_empty_markers_after_it() {
        assert_eq!(wrap("  ", BOLD), wrapped("  ****", 4..4));
    }

    #[test]
    fn the_selection_counts_characters_not_bytes() {
        // "é" is 2 bytes, "𝑥" is 4: offsets are in characters.
        assert_eq!(wrap(" é𝑥", BOLD), wrapped(" **é𝑥**", 3..5));
    }

    #[test]
    fn lines_inside_the_selection_are_kept() {
        assert_eq!(wrap("a\nb", BOLD), wrapped("**a\nb**", 2..5));
        // Across paragraphs too, as documented (not bold once rendered).
        assert_eq!(wrap("a\n\nb", BOLD), wrapped("**a\n\nb**", 2..6));
    }
}
