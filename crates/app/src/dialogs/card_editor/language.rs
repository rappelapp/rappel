//! The editor's highlighting language: Rappel's Markdown, `GtkSourceView`'s
//! own "markdown" with math (`data/language-specs/rappel-markdown.lang`,
//! ADR 0087). Inside `$…$` and `$$…$$` there is no emphasis, bold or other
//! Markdown styling, and math is found as the renderer finds it.
//!
//! The file is in the app's gresource. `GtkSourceView` 5.4 and later read
//! language files from a `resource:///` directory on the search path
//! (`gtk_source_language_manager_set_search_path`, GtkSource-5.gir: "Since
//! `GtkSourceView` 5.4 this function will allow you to provide paths in the
//! form of "resource:///" URIs"). The search path can only change before a
//! manager loads its first language, so the editor has a manager of its
//! own rather than changing the shared default one.

use std::cell::OnceCell;

use sourceview5::Language;
use sourceview5::LanguageManager;

/// The language's id, as in its file.
pub(super) const LANGUAGE_ID: &str = "rappel-markdown";

/// The gresource directory of the language file (the app's resource
/// prefix, `data/resources.gresource.xml`).
const SEARCH_DIR: &str = "resource:///io/github/rappelapp/Rappel/language-specs/";

thread_local! {
    /// GTK objects stay on the thread that made them (the main thread).
    static MANAGER: OnceCell<LanguageManager> = const { OnceCell::new() };
}

/// The manager that finds Rappel's language first, then `GtkSourceView`'s
/// own (which it builds on: `markdown`'s Markdown syntax, and `def`;
/// `html` is loaded as `markdown` needs it, but not used, ADR 0087).
fn manager() -> LanguageManager {
    MANAGER.with(|cell| {
        cell.get_or_init(|| {
            let manager = LanguageManager::new();
            manager.prepend_search_path(SEARCH_DIR);
            manager
        })
        .clone()
    })
}

/// The language of the editor's buffers: Rappel's Markdown, else (with a
/// warning) `GtkSourceView`'s stock Markdown, else none.
pub(super) fn markdown() -> Option<Language> {
    let manager = manager();
    if let Some(language) = manager.language(LANGUAGE_ID) {
        return Some(language);
    }
    tracing::warn!(
        "{LANGUAGE_ID} language missing from the resources; using GtkSourceView's markdown"
    );
    let stock = manager.language("markdown");
    if stock.is_none() {
        tracing::warn!("GtkSourceView has no markdown language; no highlighting");
    }
    stock
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use sourceview5::prelude::*;

    /// Per character of `text`, whether the highlighter put it in math,
    /// and whether it is drawn italic or bold, with the Adwaita scheme.
    fn highlight(text: &str) -> Vec<(char, bool, bool)> {
        let text = text.to_owned();
        crate::testing::with_widgets(move || {
            let language = super::markdown().expect("the language loads");
            assert_eq!(language.id(), super::LANGUAGE_ID);
            let buffer = sourceview5::Buffer::with_language(&language);
            buffer.set_style_scheme(
                sourceview5::StyleSchemeManager::default()
                    .scheme("Adwaita")
                    .as_ref(),
            );
            buffer.set_highlight_syntax(true);
            buffer.set_text(&text);
            let (start, end) = buffer.bounds();
            buffer.ensure_highlight(&start, &end);
            let mut out = Vec::new();
            let mut iter = buffer.start_iter();
            while !iter.is_end() {
                let math = buffer.iter_has_context_class(&iter, "math");
                let tags = iter.tags();
                let italic = tags
                    .iter()
                    .any(|t| t.is_style_set() && t.style() == gtk::pango::Style::Italic);
                // 700 is Pango's bold (`PANGO_WEIGHT_BOLD`).
                let bold = tags.iter().any(|t| t.is_weight_set() && t.weight() >= 700);
                out.push((iter.char(), math, italic || bold));
                iter.forward_char();
            }
            out
        })
    }

    /// HTML is shown as plain text: it is never interpreted (SPEC §1.5),
    /// and its contexts of several lines would escape the long-line cap.
    #[test]
    fn html_gets_no_highlighting() {
        crate::testing::with_widgets(|| {
            let language = super::markdown().expect("the language loads");
            let buffer = sourceview5::Buffer::with_language(&language);
            buffer.set_style_scheme(
                sourceview5::StyleSchemeManager::default()
                    .scheme("Adwaita")
                    .as_ref(),
            );
            buffer.set_highlight_syntax(true);
            buffer.set_text("<b>x</b> <!-- c -->\n<title>\nt</title>");
            let (start, end) = buffer.bounds();
            buffer.ensure_highlight(&start, &end);
            let mut iter = buffer.start_iter();
            while !iter.is_end() {
                assert!(iter.tags().is_empty(), "{:?} is highlighted", iter.char());
                iter.forward_char();
            }
        });
    }

    /// `text` with each math character as `m`, each styled (italic or
    /// bold) one outside math as `s`, the rest as `.`; a styled math
    /// character is `!`.
    fn picture(text: &str) -> String {
        highlight(text)
            .into_iter()
            .map(|(c, math, styled)| match (c, math, styled) {
                ('\n', _, _) => '\n',
                (_, true, true) => '!',
                (_, true, false) => 'm',
                (_, false, true) => 's',
                (_, false, false) => '.',
            })
            .collect()
    }

    #[test]
    fn math_has_no_markdown_styling() {
        let snell = r"$$n_1\sin\theta_1 = n_2\sin\theta_2$$";
        let all_math = "m".repeat(snell.len());
        for (text, expected) in [
            (snell, all_math.as_str()),
            ("$a*b*c$ and $x_1 y_2$", "mmmmmmm.....mmmmmmmmm"),
            ("$$\na_1 *b* **c**\n$$", "mm\nmmmmmmmmmmmmm\nmm"),
            ("Area $\\pi r^2$, done", ".....mmmmmmmmm......"),
        ] {
            assert_eq!(picture(text), expected, "{text}");
        }
    }

    #[test]
    fn dollars_that_are_not_math_stay_text() {
        // The renderer's cases (`content/tests.rs`,
        // `dollars_as_currency_stay_text`, `currency_beside_real_math`).
        for (text, expected) in [
            ("$5 and $10", ".........."),
            ("costs $5.", "........."),
            ("$5-$10", "......"),
            ("from $5-$10 a day", "................."),
            (r"\$x\$", "....."),
            (r"\$5 and \$10", "............"),
            ("$ x $", "....."),
            ("a $ sign", "........"),
            ("$5-$10 for $x$", "...........mmm"),
            (r"$a\$b$", "mmmmmm"),
            (r"\\$x$", "..mmm"),
            ("`$x_1$`", "......."),
        ] {
            assert_eq!(picture(text), expected, "{text}");
        }
    }

    #[test]
    fn emphasis_skips_math() {
        for (text, expected) in [
            ("*see $x*y$ here*", "sssssmmmmmssssss"),
            ("_a $x_1$ b_", "sssmmmmmsss"),
            ("**a $x**y$ b**", "ssssmmmmmmssss"),
            ("x_1 = $y_2$", "......mmmmm"),
            ("x_1 and y_2", "..........."),
            ("*a* $b$ _c_", "sss.mmm.sss"),
            // The blockquote marker and the heading are bold, the math not.
            ("> $a_1 b_2$", "s.mmmmmmmmm"),
            ("# Snell $n_1 n_2$", "ssssssssmmmmmmmmm"),
        ] {
            assert_eq!(picture(text), expected, "{text}");
        }
    }

    /// A line of 768 characters or more is left plain (the cap against
    /// `GtkSourceView`'s cost per piece found on a line, ADR 0087), and the
    /// next line is highlighted again. The cap holds from wherever the
    /// highlighting resumes on a line, also after a context of several
    /// lines that closes at its start.
    #[test]
    fn a_line_of_768_characters_or_more_is_left_plain() {
        let short = format!("$x$ {}", "a".repeat(763));
        assert!(picture(&short).starts_with("mmm."));
        let long = format!("$x$ {}", "a".repeat(764));
        assert!(picture(&long).starts_with("...."));
        assert!(picture(&format!("{long}\n$x$")).ends_with("\nmmm"));
        let after_math = picture(&format!("$$\n$$$x$ {}", "a".repeat(800)));
        assert!(
            after_math.starts_with("mm\nmm...."),
            "{}",
            &after_math[..12]
        );
    }

    #[test]
    fn a_lone_display_opener_stops_at_a_blank_line() {
        assert_eq!(picture("$$x\ny\n\n*z*"), "mmm\nm\n\nsss");
    }
}

/// Hostile sides in the editor's text field (ADR 0025 row 14, ADR 0087):
/// one paragraph of the largest size the store accepts, made of what the
/// language's math, emphasis and heading contexts look for, so that a
/// highlighter that is quadratic in a line would freeze the window. Same
/// measure and bounds as the editor's
/// `a_huge_one_paragraph_side_is_usable_in_the_text_field`. Without the
/// long-line cap of the language file, `GtkSourceView` 5.22 took 2.4 s for
/// 20 KB of `$$a `, and minutes for a whole side; with a cap of 1,024
/// characters, lines just under it gave 0.45 s steps (768: 0.33 s).
#[cfg(test)]
mod hostile_tests {
    use std::time::{Duration, Instant};

    use gtk::glib;
    use gtk::prelude::*;
    use rappel_core::domain::MAX_CARD_SIDE_BYTES;
    use sourceview5::prelude::*;

    use crate::testing::{cpu_time_of, cpu_within};

    /// The longest step of the main loop until it has been idle for
    /// 200 ms (or `settle` ends), as CPU time of this thread (ADR 0049).
    fn longest_stall(settle: Duration) -> Duration {
        let context = glib::MainContext::default();
        let end = Instant::now() + settle;
        let mut idle_since = None;
        let mut longest = Duration::ZERO;
        while Instant::now() < end {
            let (step, worked) = cpu_time_of(|| context.iteration(false));
            if worked {
                longest = longest.max(step);
                idle_since = None;
            } else {
                let since = *idle_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_millis(200) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        longest
    }

    /// The length from which a line is left plain (`long-line` in the
    /// language file).
    const LINE_CAP: usize = 768;

    /// `unit` repeated to the largest side, after `prefix`.
    fn side(prefix: &str, unit: &str) -> String {
        let mut text = prefix.to_owned();
        text.push_str(&unit.repeat((MAX_CARD_SIDE_BYTES - prefix.len()) / unit.len()));
        text
    }

    #[test]
    fn hostile_one_line_sides_are_usable_in_the_text_field() {
        let sides = [
            ("display math", side("", "$$a ")),
            ("display math in a heading", side("# ", "$$a ")),
            ("dollars in a heading", side("# ", "a$ ")),
            ("asterisks", side("", "*a ")),
            ("underscores", side("", "_a ")),
            ("asterisks and dollars", side("", "*$a ")),
            // A line that starts inside a context of several lines, which
            // closes at once: the cap is checked where the search resumes.
            (
                "display math closed at a line's start",
                side("$$\n$$", "$a$ "),
            ),
            // HTML's contexts of several lines bypassed the cap before HTML
            // was left out of the language (ADR 0087); now plain text.
            ("a former HTML tag over two lines", side("<a\n>", "$a$ ")),
            ("after a title tag", side("<title>\n", "$a$ ")),
            ("after a style tag", side("<style>\n", "a{} ")),
            // Inside display math the cap does not apply: only escapes are
            // looked for there.
            ("escapes in display math", side("$$\n", "\\a ")),
            // A fence's lines are not capped either: it has no children.
            ("fenced code", side("```\n", "$a$ ")),
            // Many lines just under the cap, each full of formulas.
            (
                "lines just under the cap",
                side("", &format!("{}\n", "$a$ ".repeat((LINE_CAP - 1) / 4))),
            ),
        ];
        let results = crate::testing::with_widgets(move || {
            sides.map(|(what, side)| {
                let buffer = sourceview5::Buffer::new(None);
                buffer.set_language(super::markdown().as_ref());
                buffer.set_highlight_syntax(true);
                let view = sourceview5::View::with_buffer(&buffer);
                view.set_monospace(true);
                view.set_wrap_mode(gtk::WrapMode::WordChar);
                let (load, ()) = cpu_time_of(|| buffer.set_text(&side));
                let (first, ()) = cpu_time_of(|| {
                    let _ = view.measure(gtk::Orientation::Horizontal, -1);
                    let _ = view.measure(gtk::Orientation::Vertical, 600);
                });
                let stall = longest_stall(Duration::from_secs(20));
                let middle = buffer.iter_at_offset(buffer.char_count() / 2);
                buffer.place_cursor(&middle);
                let (typed, ()) = cpu_time_of(|| {
                    buffer.insert_at_cursor("y");
                    let _ = view.measure(gtk::Orientation::Vertical, 600);
                });
                let stall_after = longest_stall(Duration::from_secs(20));
                let verdict = cpu_within(load, Duration::from_secs(1))
                    .and(cpu_within(first, Duration::from_secs(2)))
                    .and(cpu_within(stall, Duration::from_secs(2)))
                    .and(cpu_within(typed, Duration::from_secs(1)))
                    .and(cpu_within(stall_after, Duration::from_secs(2)));
                (what, load, first, stall, typed, stall_after, verdict)
            })
        });
        let mut failed = Vec::new();
        for (what, load, first, stall, typed, stall_after, verdict) in results {
            let print =
                |step: &str, t| crate::testing::print_timing(&format!("1 MiB {what}: {step}"), t);
            print("load", load);
            print("first layout", first);
            print("longest main-loop step while highlighting", stall);
            print("type one character and lay out", typed);
            print("longest main-loop step after typing", stall_after);
            if let Err(message) = verdict {
                failed.push(format!(
                    "{what}: load {load:?}, layout {first:?}, stall {stall:?}, \
                     typing {typed:?}, stall after typing {stall_after:?}: {message}"
                ));
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}
