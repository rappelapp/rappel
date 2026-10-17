//! Helpers shared by the tests of several modules. Test builds only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::{CardState, Grade, ReviewEvent, Schedule};
pub(crate) use crate::scheduling::DAY_MS;
use crate::storage::Store;

/// 2026-10-15T12:34:56.789Z, the "now" most tests start from.
pub(crate) const T0: i64 = 1_792_067_696_789;

/// The schedule of a card that must be scheduled.
pub(crate) fn scheduled(state: CardState) -> Schedule {
    match state {
        CardState::Scheduled(s) => s,
        CardState::New => panic!("expected a scheduled card"),
    }
}

/// Records `grade` for `card` at `at`, answered in 3 seconds.
pub(crate) fn answer(store: &mut Store, card: &str, grade: Grade, at: i64) -> ReviewEvent {
    store
        .record_review(card, grade, Duration::from_secs(3), at)
        .unwrap()
}

/// A `card_schedule` row: card id, phase, due, stability and difficulty as
/// bits, last review, reps, lapses.
pub(crate) type ScheduleRow = (String, String, i64, u64, u64, i64, i64, i64);

/// Every `card_schedule` row, with the floats as bits so equality is exact.
pub(crate) fn schedule_table(store: &Store) -> Vec<ScheduleRow> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT card_id, phase, due_at, stability, difficulty, last_review_at, reps, lapses
             FROM card_schedule ORDER BY card_id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, f64>(3)?.to_bits(),
            r.get::<_, f64>(4)?.to_bits(),
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

/// `path` with `suffix` appended to its file name, e.g. the `-wal` file of
/// a database.
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The stack hostile input is parsed on in tests: a quarter of what a
/// spawned thread gets by default (2 MiB) and a sixteenth of the main
/// thread's usual 8 MiB, so a recursion that only just passes here has a
/// wide margin in the app. A stack overflow aborts the process; it cannot
/// be caught.
pub(crate) const SMALL_STACK: usize = 512 * 1024;

/// Runs `f` on a new thread with a [`SMALL_STACK`] and returns its result,
/// failing the test if it takes longer than `limit`. A hang fails the test
/// instead of blocking the run (the thread is left behind; the test
/// process ends anyway).
pub(crate) fn within<T: Send + 'static>(
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let start = std::time::Instant::now();
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            // The receiver is gone only if the test already failed.
            let _ = tx.send(f());
        })
        .unwrap();
    match rx.recv_timeout(limit) {
        Ok(value) => {
            if std::env::var_os("RAPPEL_TIMINGS").is_some() {
                eprintln!("{what}: {:?}", start.elapsed());
            }
            value
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("{what} took longer than {limit:?}")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("{what} panicked"),
    }
}
