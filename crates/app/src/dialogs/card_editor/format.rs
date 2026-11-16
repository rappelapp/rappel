//! Ctrl+B / Ctrl+I / Ctrl+M in the card editor: wrapping a selection in
//! Markdown emphasis markers or math dollars. Pure text, so the rules are
//! tested without GTK.

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

/// What a formatting key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Wrap the selection in this marker ([`wrap`]).
    Wrap(&'static str),
    /// Cycle the selection through `$x$`, `$$x$$` and back ([`math`]).
    Math,
}

/// An edit of the selection that may reach past it, to the dollars around
/// it (Ctrl+B / Ctrl+I never do).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Characters before the selection that `text` also replaces.
    pub before: usize,
    /// Characters after the selection that `text` also replaces.
    pub after: usize,
    /// The new text, in place of `before`, the selection and `after`.
    pub text: String,
    /// The part of `text` to select, in characters.
    pub select: Range<usize>,
}

/// How many `$` the formula is delimited by in a run: 1 or 2, else 0.
fn delimiter(run: usize) -> usize {
    run.min(2)
}

/// Ctrl+M: no dollars gives `$x$`, one pair `$$x$$`, two pairs none.
///
/// `before` and `after` are the text just around the selection (a few
/// characters each is enough: three before, two after). The dollars the
/// selection sits in are found there, or inside the selection itself when
/// it includes them. A `$` after a backslash is not a delimiter, as in the
/// renderer (`content::parse`). Nothing selected gives `$$` with the cursor
/// between. Whitespace at the edges of a plain selection stays outside the
/// dollars, as for [`wrap`]. Undoing is Ctrl+Z, one step.
pub fn math(before: &str, selected: &str, after: &str) -> Edit {
    let next = |k: usize| (k + 1) % 3;
    let dollars = |k: usize| "$".repeat(k);
    let plain = |select_from: usize, text: &str| Edit {
        before: 0,
        after: 0,
        select: select_from..select_from + text.chars().count(),
        text: text.to_owned(),
    };

    // The selection includes its own dollars: `$x$` or `$$x$$`.
    let trimmed_start = selected.trim_start();
    let lead = &selected[..selected.len() - trimmed_start.len()];
    let core = trimmed_start.trim_end();
    let trail = &trimmed_start[core.len()..];
    if !before.ends_with('\\') {
        for k in [2, 1] {
            let marker = dollars(k);
            let Some(inner) = core
                .strip_prefix(marker.as_str())
                .and_then(|rest| rest.strip_suffix(marker.as_str()))
            else {
                continue;
            };
            if inner.starts_with('$') || inner.ends_with('$') || inner.ends_with('\\') {
                continue;
            }
            let to = dollars(next(k));
            let start = lead.chars().count() + to.chars().count();
            let mut edit = plain(start, inner);
            edit.text = format!("{lead}{to}{inner}{to}{trail}");
            return edit;
        }
    }

    // The dollars are just outside the selection.
    let run_before = before.chars().rev().take_while(|c| *c == '$').count();
    let escaped_before = before
        .chars()
        .rev()
        .nth(run_before)
        .is_some_and(|c| c == '\\');
    let run_after = after.chars().take_while(|c| *c == '$').count();
    let k = if escaped_before || selected.ends_with('\\') {
        0
    } else if run_before == run_after {
        // Uneven runs (`$$x$`) are not a pair around the selection.
        delimiter(run_before)
    } else {
        0
    };
    if k > 0 {
        let to = dollars(next(k));
        let mut edit = plain(to.len(), selected);
        edit.before = k;
        edit.after = k;
        edit.text = format!("{to}{selected}{to}");
        return edit;
    }

    let wrapped = wrap(selected, "$");
    Edit {
        before: 0,
        after: 0,
        text: wrapped.text,
        select: wrapped.select,
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

    /// What `math` gives once applied, for the tests: the text around plus
    /// the edit, and the selected text after.
    fn applied(before: &str, selected: &str, after: &str) -> (String, String) {
        let edit = math(before, selected, after);
        let keep_before: Vec<char> = before.chars().collect();
        let keep_before: String = keep_before[..keep_before.len() - edit.before]
            .iter()
            .collect();
        let keep_after: String = after.chars().skip(edit.after).collect();
        let picked: String = edit
            .text
            .chars()
            .skip(edit.select.start)
            .take(edit.select.len())
            .collect();
        (format!("{keep_before}{}{keep_after}", edit.text), picked)
    }

    #[test]
    fn math_with_nothing_selected_puts_the_cursor_between_dollars() {
        assert_eq!(
            math("a ", "", " b"),
            Edit {
                before: 0,
                after: 0,
                text: "$$".into(),
                select: 1..1
            }
        );
    }

    #[test]
    fn math_wraps_then_doubles_then_removes() {
        // The selection stays on the inner text each time, so the next
        // press finds the dollars around it.
        assert_eq!(applied("", "x", ""), ("$x$".into(), "x".into()));
        assert_eq!(applied("$", "x", "$"), ("$$x$$".into(), "x".into()));
        assert_eq!(applied("$$", "x", "$$"), ("x".into(), "x".into()));
        // Mid-text, with the context a buffer gives.
        assert_eq!(
            applied("see $", "x", "$ now"),
            ("see $$x$$ now".into(), "x".into())
        );
        assert_eq!(
            applied("see $$", "x", "$$ now"),
            ("see x now".into(), "x".into())
        );
    }

    #[test]
    fn math_reads_dollars_inside_the_selection_too() {
        assert_eq!(applied("", "$x$", ""), ("$$x$$".into(), "x".into()));
        assert_eq!(applied("", "$$x$$", ""), ("x".into(), "x".into()));
        assert_eq!(applied("", " $x$ ", ""), (" $$x$$ ".into(), "x".into()));
    }

    #[test]
    fn math_keeps_whitespace_of_a_plain_selection_outside() {
        assert_eq!(applied("", "x ", ""), ("$x$ ".into(), "x".into()));
    }

    #[test]
    fn an_escaped_dollar_is_not_a_delimiter() {
        // `\$x\$` is text with two dollar signs: Ctrl+M makes a formula.
        assert_eq!(applied("\\$", "x", "$"), ("\\$$x$$".into(), "x".into()));
        assert_eq!(
            applied("", "\\$x\\$", ""),
            ("$\\$x\\$$".into(), "\\$x\\$".into())
        );
    }

    #[test]
    fn dollars_that_are_not_a_pair_around_it_are_ignored() {
        // Only one side has a dollar: not delimiters of this selection.
        assert_eq!(applied("$", "x", ""), ("$$x$".into(), "x".into()));
        assert_eq!(applied("", "x", "$"), ("$x$$".into(), "x".into()));
    }

    #[test]
    fn uneven_dollars_around_it_are_not_a_pair() {
        assert_eq!(applied("$$", "x", "$"), ("$$$x$$".into(), "x".into()));
        assert_eq!(applied("$", "x", "$$"), ("$$x$$$".into(), "x".into()));
    }

    #[test]
    fn a_selection_that_includes_the_dollars_of_a_formula_it_is_in_is_taken_literally() {
        // Odd, but one Ctrl+Z away: the dollars selected are plain text.
        assert_eq!(applied("$", "$x$", "$"), ("$$$x$$$".into(), "x".into()));
    }

    #[test]
    fn a_selection_ending_in_a_backslash_is_wrapped_as_it_is() {
        // The closing dollar is then escaped, so it is not a formula yet:
        // left alone, the user sees it and decides. One Ctrl+Z takes it back.
        assert_eq!(applied("", "x\\", ""), ("$x\\$".into(), "x\\".into()));
    }

    #[test]
    fn math_counts_characters_not_bytes() {
        let edit = math("", "é𝑥", "");
        assert_eq!(edit.select, 1..3);
    }
}
