//! Fills a scratch database with many cards, to check that the card list
//! stays smooth (ADR 0013), and optionally makes some of them due now, to
//! try studying without waiting days (ADR 0014).
//!
//! ```text
//! cargo run -p cards-core --example fill_scratch_db -- /tmp/cards-scratch [count] [--due N]
//! CARDS_DATA_DIR=/tmp/cards-scratch cargo run -p cards
//! ```
//!
//! It adds one deck, "Scratch (N cards)", with `count` cards (default
//! 5,000). With `--due N`, the first N cards get a review history in the
//! past, recorded through the real scheduler, so they are due today; the
//! rest stay new. The directory must be absolute and is never one of the
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
    let usage = "usage: fill_scratch_db <absolute scratch dir> [count] [--due N]";
    let number = |arg: Option<std::ffi::OsString>| {
        arg.as_ref()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or(usage)
    };
    let mut args = std::env::args_os().skip(1);
    let dir = PathBuf::from(args.next().ok_or(usage)?);
    let (mut count, mut due) = (None, 0);
    while let Some(arg) = args.next() {
        if arg == "--due" {
            due = number(args.next())?;
        } else if count.is_none() {
            count = Some(number(Some(arg))?);
        } else {
            return Err(usage.to_owned());
        }
    }
    let count = count.unwrap_or(DEFAULT_COUNT);
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
    Ok(format!(
        "Added {count} cards ({due} due now) to “{}” in {}",
        deck.name,
        dir.display()
    ))
}

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
