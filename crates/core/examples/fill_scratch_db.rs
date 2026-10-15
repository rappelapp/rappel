//! Fills a scratch database with many cards, to check that the card list
//! stays smooth (ADR 0013).
//!
//! ```text
//! cargo run -p cards-core --example fill_scratch_db -- /tmp/cards-scratch [count]
//! CARDS_DATA_DIR=/tmp/cards-scratch cargo run -p cards
//! ```
//!
//! It adds one deck, "Scratch (N cards)", with `count` cards (default
//! 5,000). The directory must be absolute and is never one of the app's own
//! data directories, release or debug: this is for throwaway data only.

use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use cards_core::data_dir::{BuildProfile, DataDirEnv, ensure_data_dir, resolve_data_dir};
use cards_core::storage::Store;

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
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: fill_scratch_db <absolute scratch dir> [count]";
    let dir = PathBuf::from(args.next().ok_or(usage)?);
    let count = match args.next() {
        Some(n) => n
            .to_str()
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or(usage)?,
        None => DEFAULT_COUNT,
    };
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
    for i in 0..count {
        let (front, back) = sample(i);
        store
            .add_card(&deck.id, &front, &back)
            .map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "Added {count} cards to “{}” in {}",
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
