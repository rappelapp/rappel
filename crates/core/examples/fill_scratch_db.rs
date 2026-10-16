//! Fills a scratch database with many cards, to check that the card list
//! stays smooth (ADR 0013), and optionally makes some of them due now, to
//! try studying without waiting days (ADR 0014).
//!
//! ```text
//! cargo run -p cards-core --example fill_scratch_db -- /tmp/cards-scratch [count] [--due N] [--markdown]
//! CARDS_DATA_DIR=/tmp/cards-scratch cargo run -p cards
//! ```
//!
//! It adds one deck, "Scratch (N cards)", with `count` cards (default
//! 5,000; none and no deck with 0). With `--due N`, the first N cards get a
//! review history in the past, recorded through the real scheduler, so
//! they are due today; the rest stay new. With `--markdown`, it also adds
//! the deck "Markdown (one card per construct)": new cards, one per
//! Markdown construct (ADR 0019), all studyable today, to check how they
//! look. The directory must be absolute and is never one of the
//! app's own data directories, release or debug: this is for throwaway
//! data only.

use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use cards_core::data_dir::{BuildProfile, DataDirEnv, ensure_data_dir, resolve_data_dir};
use cards_core::domain::Grade;
use cards_core::scheduling::{DAY_MS, STEP_MS};
use cards_core::storage::Store;
use cards_core::time::now_ms;

const DEFAULT_COUNT: usize = 5_000;

fn main() -> ExitCode {
    match run() {
        Ok(message) => {
            // Nothing useful is left to do if stdout is closed.
            let _ = writeln!(io::stdout(), "{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            let _ = writeln!(io::stderr(), "fill_scratch_db: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let usage = "usage: fill_scratch_db <absolute scratch dir> [count] [--due N] [--markdown]";
    let number = |arg: Option<std::ffi::OsString>| {
        arg.as_ref()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or(usage)
    };
    let mut args = std::env::args_os().skip(1);
    let dir = PathBuf::from(args.next().ok_or(usage)?);
    let (mut count, mut due, mut markdown) = (None, 0, false);
    while let Some(arg) = args.next() {
        if arg == "--due" {
            due = number(args.next())?;
        } else if arg == "--markdown" {
            markdown = true;
        } else if count.is_none() {
            count = Some(number(Some(arg))?);
        } else {
            return Err(usage.to_owned());
        }
    }
    let count = count.unwrap_or(DEFAULT_COUNT);
    if count == 0 && !markdown {
        return Err("nothing to add: count is 0 and no --markdown".to_owned());
    }
    if due > count {
        return Err(format!("--due {due} is more than the {count} cards"));
    }
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    // `..` through a folder that does not exist yet would defeat the
    // comparison below (it cannot be resolved before it is created).
    if dir.components().any(|c| c == Component::ParentDir) {
        return Err(format!("{} must not contain `..`", dir.display()));
    }
    refuse_app_data_dirs(&dir)?;

    ensure_data_dir(&dir).map_err(|e| e.to_string())?;
    let mut store = Store::open(&dir).map_err(|e| e.to_string())?;
    let mut added = Vec::new();
    if count > 0 {
        let deck = store
            .create_deck(&format!("Scratch ({count} cards)"))
            .map_err(|e| e.to_string())?;
        let now = now_ms();
        for i in 0..count {
            let (front, back) = sample(i);
            let card = store
                .add_card(&deck.id, &front, &back)
                .map_err(|e| e.to_string())?;
            if i < due {
                for (grade, at) in past_reviews(i, now) {
                    store
                        .record_review(&card.id, grade, Duration::from_secs(8), at)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        added.push(format!("{count} cards ({due} due now) to “{}”", deck.name));
    }
    if markdown {
        let deck = store
            .create_deck("Markdown (one card per construct)")
            .map_err(|e| e.to_string())?;
        for (front, back) in MARKDOWN_CARDS {
            store
                .add_card(&deck.id, front, back)
                .map_err(|e| e.to_string())?;
        }
        added.push(format!("{} cards to “{}”", MARKDOWN_CARDS.len(), deck.name));
    }
    Ok(format!(
        "Added {} in {}",
        added.join(" and "),
        dir.display()
    ))
}

/// One card per Markdown construct, front and back, in study order. The
/// front says what to check. At most 20, the new cards allowed per day,
/// so all of them can be studied at once.
const MARKDOWN_CARDS: [(&str, &str); 20] = [
    (
        "Inline styles: *emphasis*, **strong**, ~~struck~~, `inline code`, and ***both***.",
        "Each word looks as its name says. `code` is monospace.",
    ),
    (
        "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6",
        "Six headings, each smaller than the one before (5 and 6 look alike).",
    ),
    (
        "A bullet list, nested:\n\n- one\n- two\n  - two a\n  - two b\n    - deeper\n- three",
        "Numbered, from 3:\n\n3. third\n4. fourth\n   1. nested one\n   2. nested two\n5. fifth",
    ),
    (
        "A code block:\n\n```rust\nfn main() {\n    println!(\"<b>not bold</b> & stays\");\n}\n```",
        "A very long line must wrap, not widen the window:\n\n```\nlet a_very_long_identifier_name = some_function(with, many, arguments, that, keep, going, and, going, past, the, edge);\n```",
    ),
    (
        "A block quote:\n\n> Quoted text, dimmed, with a line at the start.\n>\n> > A quote inside the quote.",
        "Text after a quote:\n\n> quoted\n\nnot quoted",
    ),
    (
        "Above the rule\n\n---\n\nBelow the rule",
        "One separator line between the two paragraphs.",
    ),
    (
        "A table:\n\n| Left | Centre | Right |\n|:-----|:------:|------:|\n| a | b | c |\n| *longer cell* | `code` | 12.50 |",
        "A wide table scrolls sideways at 360 px:\n\n| Column one | Column two | Column three | Column four | Column five | Column six |\n|--|--|--|--|--|--|\n| some text | more text | even more text | and more | still more | the end |",
    ),
    (
        "Inline math: the area is $\\pi r^2$ and $e^{i\\pi} + 1 = 0$.",
        "Shown as monospace source until math rendering comes.",
    ),
    (
        "Display math:\n\n$$\\int_0^1 x^2 \\, dx = \\frac{1}{3}$$\n\nText after it.",
        "The formula is centred, monospace, on its own line.",
    ),
    (
        "Currency is not math: $5 and $10, from $5-$10 a day, and \\$x\\$.",
        "All dollar signs show as typed; nothing is monospace.",
    ),
    (
        "A [link to example.org](https://example.org/a?b=1&c=2) and an autolink <https://gnome.org>.",
        "Hover each underlined link: its URL shows as a tooltip. Clicking does nothing.",
    ),
    (
        "A media image: ![a diagram](media:0123abcd.png) and one without alt text: ![](media:ffff.png)",
        "A remote image is never fetched and shows as typed: ![logo](https://example.org/logo.png)",
    ),
    (
        "Raw HTML shows as typed: <b>not bold</b>, <i>not italic</i>, <span foreground=\"red\">not red</span>, & and &amp;",
        "<div>\n<script>alert(1)</script>\n<img src=\"https://example.org/x.png\">\n</div>",
    ),
    (
        "This card has an empty back: after Show Answer, only Fail and Pass appear.",
        "",
    ),
    (
        "Hard line break:  \nthis is on a new line.\nThis is not (a soft break).",
        "Two paragraphs:\n\nfirst\n\nsecond",
    ),
    (
        "Deep nesting is flattened after 16 levels, keeping the text:\n\n>>>>>>>>>>>>>>>>>>>>>>>> deep quote",
        "- 1\n  - 2\n    - 3\n      - 4\n        - 5\n          - 6\n            - 7\n              - 8\n                - 9\n                  - 10\n                    - 11\n                      - 12\n                        - 13\n                          - 14\n                            - 15\n                              - 16\n                                - 17\n                                  - 18",
    ),
    (
        "A simple front: one short paragraph, like every other card.",
        "A simple back.",
    ),
    (
        "# A long mixed card\n\nScroll it at 360 px. It has a paragraph with **strong** text and a [link](https://example.org).\n\n- a list item with `code`\n- another, with $x^2$\n\n> a quote\n\n```\ncode block\n```\n\n| a | b |\n|--|--|\n| 1 | 2 |\n\n$$\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}$$\n\n---\n\nThe end of the front.",
        "## Back heading\n\nThe back can be rich too.\n\n1. first\n2. second",
    ),
    (
        "Escapes: \\*not emphasis\\*, \\`not code\\`, \\# not a heading, 1\\. not a list.",
        "All shown with the characters, without the backslashes.",
    ),
    (
        "Strikethrough and code in a heading:\n\n## ~~old~~ `new` title",
        "Rule only on the back:\n\n---",
    ),
];

/// Fails if `dir` is, or is inside, the release or debug data directory
/// the app would use without `CARDS_DATA_DIR`.
fn refuse_app_data_dirs(dir: &Path) -> Result<(), String> {
    let env = DataDirEnv {
        cards_data_dir: None,
        ..DataDirEnv::from_process()
    };
    for profile in [BuildProfile::Release, BuildProfile::Debug] {
        let Ok(app_dir) = resolve_data_dir(&env, profile) else {
            continue;
        };
        let same = |a: &Path, b: &Path| a.starts_with(b); // a is b or inside it
        let resolved = (dir.canonicalize(), app_dir.canonicalize());
        let inside = same(dir, &app_dir)
            || matches!(&resolved, (Ok(d), Ok(a)) if same(d, a))
            || matches!(&resolved, (Err(_), Ok(a)) if same(dir, a));
        if inside {
            return Err(format!(
                "{} is the app's data directory ({}); use a scratch directory",
                dir.display(),
                app_dir.display()
            ));
        }
    }
    Ok(())
}

/// A past history after which card `i` is due today: learned 40 days ago
/// (Pass, then Pass out of the 10-minute step). Every third card also
/// lapsed 20 days ago (Fail, then Pass out of the step). FSRS gives such
/// cards intervals of days, so they are all overdue now.
fn past_reviews(i: usize, now: i64) -> Vec<(Grade, i64)> {
    let learned = now - 40 * DAY_MS;
    let mut reviews = vec![(Grade::Pass, learned), (Grade::Pass, learned + STEP_MS)];
    if i.is_multiple_of(3) {
        let lapsed = now - 20 * DAY_MS;
        reviews.extend([(Grade::Fail, lapsed), (Grade::Pass, lapsed + STEP_MS)]);
    }
    reviews
}

/// Card `i`: mostly short fronts, with some long, multi-line and
/// markup-like ones to check ellipsising and escaping.
fn sample(i: usize) -> (String, String) {
    let n = i + 1;
    let front = match i % 10 {
        0 => format!(
            "Card {n}: a long front that should not fit on one line, so the list must \
             ellipsise it at the end instead of wrapping or widening the window. {}",
            "More words. ".repeat(20)
        ),
        1 => format!("Card {n}: first line\nsecond line\n\nthird line"),
        2 => format!("Card {n}: <b>not bold</b> & <i>not italic</i> &amp; must show as typed"),
        3 => format!("Card {n}: what is $\\frac{{d}}{{dx}} x^2$?"),
        _ => format!("Card {n}: what is {n} × 7?"),
    };
    let back = format!("Answer {n}: {}", n * 7);
    (front, back)
}
