//! One thread for the tests that need the default main context or GTK.
//!
//! Only the thread that owns the default main context may iterate it, and
//! `gtk::init` takes it for good: gtk4-rs 0.11 (`rt.rs`, `init`) calls
//! `g_main_context_acquire` and never releases it, and refuses to start GTK
//! again from another thread. The test harness runs each test on a thread
//! of its own, so two such tests on their own threads fail or crash. They
//! send their body here instead, and the bodies run one at a time.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use gtk::glib;

type Job = Box<dyn FnOnce() + Send>;

thread_local! {
    /// Whether this thread is the one that owns the default main context.
    static ON_WORKER: Cell<bool> = const { Cell::new(false) };
    /// Where the last panic on this thread happened, as the panic hook
    /// saw it (set on the main-context thread only).
    static PANICKED_AT: Cell<Option<String>> = const { Cell::new(None) };
}

/// Keeps the location of a panic on the main-context thread for
/// [`on_main_context`], then calls the hook that was there before (which
/// prints the message). That print goes to the output capture of the test
/// that started the thread, long finished, so it never showed: a body that
/// failed there printed nothing (`ui::dropdown`'s flaky test, ADR 0073).
fn keep_panic_locations() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if ON_WORKER.get() {
            PANICKED_AT.set(info.location().map(ToString::to_string));
        }
        previous(info);
    }));
}

/// The text of a panic's payload (`panic!` with a message gives a `String`
/// or a `&str`).
fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("a panic without a message")
}

/// The queue of the thread that owns the default main context, started on
/// first use.
fn queue() -> &'static mpsc::Sender<Job> {
    static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        keep_panic_locations();
        let (sender, jobs) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("main-context".to_owned())
            .spawn(move || {
                ON_WORKER.set(true);
                let context = glib::MainContext::default();
                let _owner = context
                    .acquire()
                    .expect("every test that uses the default main context goes through here");
                for job in jobs {
                    job();
                }
            })
            .expect("tests can start a thread");
        sender
    })
}

/// Runs `f` on the thread that owns the default main context and returns
/// its result. A panic in `f` fails the calling test, with its message and
/// place (a panic inside a C
/// callback that `f` runs aborts the test binary instead: it cannot unwind
/// through C). Called from that thread, it runs `f` directly: queueing it
/// would wait for itself.
pub fn on_main_context<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    if ON_WORKER.get() {
        return f();
    }
    let (sender, answer) = mpsc::channel();
    let job: Job = Box::new(move || {
        PANICKED_AT.set(None);
        // A panic's message and place are sent back to the caller, whose
        // test prints them.
        let result = panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
            let at = PANICKED_AT
                .take()
                .unwrap_or_else(|| "an unknown place".to_owned());
            format!(
                "{} (on the main-context thread, at {at})",
                panic_text(&*payload)
            )
        });
        // The caller may have stopped waiting; nothing to do then.
        let _ = sender.send(result);
    });
    queue()
        .send(job)
        .expect("the main-context thread runs for the whole test run");
    match answer
        .recv()
        .expect("the main-context thread answers every job")
    {
        Ok(value) => value,
        // A new panic on the test's own thread, not `resume_unwind`, which
        // calls no hook: the test's output then says what failed and where.
        Err(message) => panic!("{message}"),
    }
}

/// Like [`on_main_context`], with GTK started first. Fails without a display:
/// run the tests in a desktop session or under `xvfb-run -a` (ADR 0024).
pub fn with_gtk<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    on_main_context(|| {
        // `gtk_init` would call `setlocale (LC_ALL, "")` while other test
        // threads read the locale (GLib's date formats, `strftime`):
        // glibc's `setlocale` is "MT-Unsafe const:locale" (GNU C Library
        // manual, "Setting the Locale"), so the tests keep the C locale.
        // Not in the child process that reads the French catalog through
        // the C library: it runs that one test alone and needs the locale.
        if !gtk::is_initialized() && !crate::i18n::tests::real_gettext() {
            gtk::disable_setlocale();
        }
        if !gtk::is_initialized() && x11_only(std::env::var_os("DISPLAY").as_deref()) {
            // `xvfb-run` sets DISPLAY but leaves WAYLAND_DISPLAY, and GTK
            // prefers Wayland: the tests then ran on the desktop's
            // compositor (their windows on the maintainer's screen), where
            // a text-input event for an input method context already
            // freed aborts the binary (`gtkimcontextwayland.c`,
            // `translate_purpose`: "code should not be reached", seen in
            // GTK 4.20; earlier SIGSEGVs, ADR 0024). Before GDK opens a
            // display, it is told to use X11 only
            // (`gdk_set_allowed_backends`, "must be called before
            // gdk_display_open()").
            gtk::gdk::set_allowed_backends("x11");
        }
        gtk::init().expect("GTK needs a display for this test; run it under `xvfb-run -a`");
        // The input method is GTK's own, never the session's (IBus over
        // D-Bus, or Wayland's text input): their replies come back after a
        // test has destroyed its window, and IBus then reads the freed
        // surface (`gdk_x11_surface_get_xid` from `libim-ibus.so`, a
        // SIGSEGV in every run once the tests drew on X11). The
        // `gtk-im-module` setting picks the module for every text widget
        // made after it (GtkSettings docs); CI has neither, so the tests
        // see what CI sees (ADR 0024, amended).
        if let Some(settings) = gtk::Settings::default()
            && settings.gtk_im_module().as_deref() != Some(SIMPLE_IM)
        {
            settings.set_gtk_im_module(Some(SIMPLE_IM));
        }
        f()
    })
}

/// GTK's own input method, which talks to no other process.
const SIMPLE_IM: &str = "gtk-im-context-simple";

/// Whether the tests' GTK is held to X11: whenever an X display is named
/// (`xvfb-run`, CI, or a desktop's Xwayland), so they never run on a
/// Wayland compositor (see [`with_gtk`], ADR 0024).
fn x11_only(display: Option<&std::ffi::OsStr>) -> bool {
    display.is_some_and(|d| !d.is_empty())
}

/// Like [`with_gtk`], with libadwaita started and the app's resources (the
/// templates of its widgets) registered, once per test run, for tests that
/// build its widgets.
pub fn with_widgets<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static RESOURCES: std::sync::Once = std::sync::Once::new();
    with_gtk(|| {
        adw::init().expect("libadwaita starts once GTK has");
        RESOURCES.call_once(|| {
            gtk::gio::resources_register_include!("rappel.gresource")
                .expect("the resources are built into the test binary");
        });
        f()
    })
}

/// A new database in a temporary directory, for tests that need the
/// store; the directory goes when the returned guard drops.
pub fn temporary_state() -> (tempfile::TempDir, crate::state::AppState) {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let state =
        crate::state::AppState::open(tmp.path().join("data")).expect("a new database opens");
    (tmp, state)
}

/// `list` as owned strings (tag names, ids), for tests that compare with
/// what a widget or a pure function returns.
pub fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|&s| s.to_owned()).collect()
}

/// The card rows a page or list has built (not one per card: the rows on
/// screen and GTK's margin around them).
pub fn card_rows(widget: &gtk::Widget) -> Vec<crate::widgets::CardRow> {
    descendants(widget)
}

/// A new database with a deck "Many" of `n` new cards ("Card 0", …), and
/// their ids in creation order. The cards are written in one transaction
/// straight into the database file, as an import would write them (rows
/// and search index), because adding them one by one through the store
/// waits for the disk at every card (`synchronous=FULL`): 5,000 took most
/// of a minute. For tests at scale (ADR 0073).
pub fn state_with_many_cards(
    n: usize,
) -> (
    tempfile::TempDir,
    crate::state::AppState,
    rappel_core::domain::Deck,
    Vec<String>,
) {
    state_with_cards(
        n,
        |i| (format!("Card {i}"), "back".to_owned()),
        &[],
        |_| Vec::new(),
    )
}

/// A card side full of math, number `n` (hardening round 5, ADR 0084):
/// `kind` 0 gives 32 formulas of four characters ("a^bc") no other side
/// numbered below about 4,000 has (the most a list row draws,
/// `MAX_LINE_FORMULAS`, all within a summary's 200 characters), 1 one
/// formula of about 1 KiB (the longest a summary keeps as a formula), 2
/// thirty-two pieces of broken LaTeX ("abc^") none of those has.
pub fn math_side(n: usize, kind: usize) -> String {
    use std::fmt::Write as _;
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let letter = |k: usize| char::from(LETTERS[k % LETTERS.len()]);
    let three = |k: usize| format!("{}{}{}", letter(k / 2704), letter(k / 52), letter(k));
    match kind {
        0 => (0..32)
            .map(|j| {
                let t = three(n * 32 + j);
                let mut chars = t.chars();
                let (a, rest) = (chars.next().unwrap_or('a'), chars.as_str());
                format!("${a}^{rest}$")
            })
            .collect::<Vec<_>>()
            .join(" "),
        1 => {
            let mut source = String::from("$");
            let mut k = 0;
            while source.len() < 1000 {
                // Writing to a String cannot fail.
                let _ = write!(source, "\\frac{{x_{{{k}}}}}{{y^{{{n}}}}}+");
                k += 1;
            }
            source.push_str("z$");
            source
        }
        _ => (0..32)
            .map(|j| format!("${}^$", three(n * 32 + j)))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// [`state_with_many_cards`] with card `i`'s sides from `sides` (ASCII
/// text: the search index holds it lower-cased), and the tags `tags`
/// (ASCII names) of which card `i` has those at the indices `tags_of(i)`
/// (ADR 0084: 10,000 cards full of math, grouped or not).
pub fn state_with_cards(
    n: usize,
    sides: impl Fn(usize) -> (String, String),
    tags: &[&str],
    tags_of: impl Fn(usize) -> Vec<usize>,
) -> (
    tempfile::TempDir,
    crate::state::AppState,
    rappel_core::domain::Deck,
    Vec<String>,
) {
    let (tmp, state) = temporary_state();
    let deck = state.create_deck("Many").expect("a deck is created");
    let mut conn = rusqlite::Connection::open(state.data_dir().join("rappel.sqlite3"))
        .expect("the database opens");
    let tx = conn.transaction().expect("a transaction starts");
    let now = rappel_core::time::now_ms();
    let mut ids = Vec::with_capacity(n);
    {
        let tag_id = |t: usize| format!("0ffe0000-0000-7000-8000-{t:012x}");
        let mut insert_tag = tx
            .prepare("INSERT INTO tags (id, name, name_key) VALUES (?1, ?2, ?3)")
            .expect("the insert is prepared");
        for (t, name) in tags.iter().enumerate() {
            insert_tag
                .execute(rusqlite::params![tag_id(t), name, name.to_lowercase()])
                .expect("a tag is inserted");
        }
        let mut insert = tx
            .prepare(
                "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            )
            .expect("the insert is prepared");
        let mut link = tx
            .prepare("INSERT INTO card_tags (card_id, tag_id) VALUES (?1, ?2)")
            .expect("the insert is prepared");
        for i in 0..n {
            // A lowercase UUIDv7 after every id the store issued so far.
            let id = format!("0fff0000-0000-7000-8000-{i:012x}");
            let at = now - i64::try_from(n - i).unwrap_or(i64::MAX);
            let (front, back) = sides(i);
            insert
                .execute(rusqlite::params![id, deck.id, front, back, at])
                .expect("a card is inserted");
            for t in tags_of(i) {
                link.execute(rusqlite::params![id, tag_id(t)])
                    .expect("a tag is linked");
            }
            ids.push(id);
        }
    }
    // The sides are ASCII: lower case is their folded text.
    tx.execute(
        "INSERT INTO cards_fts (rowid, front, back)
         SELECT seq, lower(front_md), lower(back_md) FROM cards WHERE deck_id = ?1",
        [&deck.id],
    )
    .expect("the index is written");
    tx.commit().expect("the cards are written");
    (tmp, state, deck, ids)
}

/// Every widget of type `T` at or under `widget`, depth first, in order
/// (the rows a list built, the labels of a page as drawn).
pub fn descendants<T: gtk::prelude::IsA<gtk::Widget>>(widget: &gtk::Widget) -> Vec<T> {
    use gtk::prelude::*;
    let mut found: Vec<T> = widget.clone().downcast().into_iter().collect();
    let mut child = widget.first_child();
    while let Some(c) = child {
        found.extend(descendants::<T>(&c));
        child = c.next_sibling();
    }
    found
}

/// The texts of the labels at or under `widget`, as drawn.
pub fn label_texts(widget: &gtk::Widget) -> Vec<String> {
    descendants::<gtk::Label>(widget)
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

/// Runs the default main context while it has something to do, at most
/// 200 iterations: lets a window map and a list build its rows.
pub fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        if !context.iteration(false) {
            break;
        }
    }
}

/// Whether the item `text` of `button`'s menu can be pressed, as drawn in
/// its open popover (the menu's `GtkModelButton`s, found by type name, as
/// GTK offers no public way to read a menu item's state). The button must
/// be in a window.
pub fn menu_item_sensitive(button: &gtk::MenuButton, text: &str) -> bool {
    use gtk::prelude::*;
    button.popup();
    let mut todo = vec![
        button
            .popover()
            .expect("the menu is open")
            .upcast::<gtk::Widget>(),
    ];
    let mut found = None;
    while let Some(widget) = todo.pop() {
        if widget.type_().name() == "GtkModelButton"
            && widget.property::<Option<String>>("text").as_deref() == Some(text)
        {
            found = Some(widget.is_sensitive());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            todo.push(c);
        }
    }
    button.popdown();
    found.unwrap_or_else(|| panic!("the menu has no item {text:?}"))
}

/// A real WAV of `samples` samples of 8 kHz, 8-bit mono silence (one
/// second is 8,000).
pub fn wav_bytes(samples: u32) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + samples).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00");
    out.extend_from_slice(&8_000_u32.to_le_bytes());
    out.extend_from_slice(&8_000_u32.to_le_bytes());
    out.extend_from_slice(b"\x01\x00\x08\x00data");
    out.extend_from_slice(&samples.to_le_bytes());
    out.resize(
        out.len() + usize::try_from(samples).expect("a small file"),
        128,
    );
    out
}

/// A real PNG of `w` × `h` grey pixels, encoded by GDK (needs GTK).
pub fn png_bytes(w: i32, h: i32) -> Vec<u8> {
    use gtk::gdk;
    use gtk::prelude::*;
    let pixels = vec![0x80_u8; usize::try_from(w * h * 4).expect("a small image")];
    let texture = gdk::MemoryTexture::new(
        w,
        h,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        usize::try_from(w * 4).expect("a small image"),
    );
    texture.save_to_png_bytes().to_vec()
}

/// Runs the default main context until `done` holds or `timeout` has
/// passed, and says whether `done` held. For waiting on GTK (a widget being
/// mapped), not for timing: the timeout only stops a test that would hang.
pub fn settle_until(done: impl Fn() -> bool, timeout: Duration) -> bool {
    let context = glib::MainContext::default();
    let end = Instant::now() + timeout;
    loop {
        while context.iteration(false) {}
        if done() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Runs the test `name` (its full path) alone in a new process of this
/// test binary, with `configure` applied to the command (its environment),
/// fails the calling test unless it passed, and returns its standard
/// output (printed with `--nocapture`). For what needs a process of its
/// own: GTK types not registered yet, `GStreamer` started with given
/// variables, a locale, or code that may bring the process down. The
/// child test tells itself apart by a variable `configure` sets, and
/// does nothing in a normal run.
pub fn run_alone(name: &str, configure: impl FnOnce(&mut std::process::Command)) -> String {
    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut command = std::process::Command::new(exe);
    command.args(["--exact", name, "--test-threads=1", "--nocapture"]);
    configure(&mut command);
    let output = command.output().expect("the test binary runs again");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{name} failed in a process of its own ({}):\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// The calling thread's CPU time (user and system) from
/// `/proc/thread-self/schedstat`, whose first field is its `sum_exec_runtime`
/// in nanoseconds (Linux `fs/proc/base.c`, `proc_pid_schedstat`), or `None`
/// where that cannot be read. Unlike a wall clock it does not grow while
/// the thread waits for a CPU, so a bound on it holds on a loaded machine
/// (#67, ADR 0049). The kernel brings that sum up to date at each scheduler
/// tick and context switch, not on the read, so a reading can be up to one
/// tick old (1 ms at `CONFIG_HZ=1000`, 4 ms at 250): time work of a few
/// milliseconds in batches.
fn schedstat_time() -> Option<Duration> {
    let schedstat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanos = schedstat.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_nanos(nanos))
}

/// The calling thread's CPU time from the utime and stime of
/// `/proc/thread-self/stat`, in `USER_HZ` clock ticks of 10 ms, or `None`
/// where that cannot be read. Coarser than [`schedstat_time`], and sampled
/// at scheduler ticks.
fn stat_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").ok()?;
    // The command name, field 2, is in parentheses and may hold spaces;
    // utime and stime are fields 14 and 15, the 12th and 13th after it.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_name.split_whitespace().skip(11);
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(Duration::from_millis((utime + stime) * 10))
}

/// Both readings of the thread's CPU time, taken at one moment.
#[derive(Clone, Copy, Debug)]
pub struct CpuReading {
    pub schedstat: Option<Duration>,
    pub stat: Option<Duration>,
}

/// Reads [`CpuReading`] for the calling thread.
pub fn thread_cpu_time() -> CpuReading {
    CpuReading {
        schedstat: schedstat_time(),
        stat: stat_time(),
    }
}

/// Wall-clock time after which a [`schedstat_time`] that has not moved is
/// taken as dead (an old kernel prints `0 0 0`). Work running that long
/// either saw a scheduler tick (at any `CONFIG_HZ` of 100 or more) or was
/// switched out, and both bring the sum up to date.
const SCHEDSTAT_DEAD_AFTER: Duration = Duration::from_millis(50);

/// The same for [`stat_time`], which only counts the ticks the thread was
/// running at, so it may miss a short run on a loaded machine.
const STAT_DEAD_AFTER: Duration = Duration::from_secs(1);

/// The CPU time between two readings, from the first clock that works: the
/// precise [`schedstat_time`], else [`stat_time`], else the wall clock. A
/// clock that did not move while `wall` passed its dead-after time is taken
/// as not working, so a dead clock makes the bounds use the wall clock
/// (stricter under load) instead of passing everything silently.
pub fn cpu_time_between(start: CpuReading, end: CpuReading, wall: Duration) -> Duration {
    let moved = |start: Option<Duration>, end: Option<Duration>, dead_after: Duration| {
        let (start, end) = (start?, end?);
        (end > start || wall < dead_after).then(|| end.saturating_sub(start))
    };
    moved(start.schedstat, end.schedstat, SCHEDSTAT_DEAD_AFTER)
        .or_else(|| moved(start.stat, end.stat, STAT_DEAD_AFTER))
        .unwrap_or(wall)
}

/// The CPU time `f` used on this thread, and its result
/// ([`cpu_time_between`]: the thread's CPU time, else the wall clock).
/// For time bounds on work that has to run on the main-context thread
/// (GTK), where [`within`] cannot be used. The same as `rappel-core`'s
/// `test_support::cpu_time_of`, which is not exported.
pub fn cpu_time_of<T>(f: impl FnOnce() -> T) -> (Duration, T) {
    let wall = Instant::now();
    let start = thread_cpu_time();
    let value = f();
    let used = cpu_time_between(start, thread_cpu_time(), wall.elapsed());
    (used, value)
}

/// How long [`within`] waits before it calls the work hung: ten times its
/// CPU-time bound, and at least 30 s. It is a wall clock, so it grows with
/// the load on the machine; a thread that gets a third of a CPU (three busy
/// threads per core) still finishes far within it. The same as
/// `rappel-core`'s `test_support::hang_limit`.
pub fn hang_limit(limit: Duration) -> Duration {
    limit.saturating_mul(10).max(Duration::from_secs(30))
}

/// A fixed piece of work that allocates, formats, sorts and looks up: the
/// yardstick of [`slowdown`]. Larger in a release build, so it stays well
/// above the CPU clock's precision. The same as `rappel-core`'s
/// `test_support::reference_work`, so [`REFERENCE_CPU`] is the same.
fn reference_work() {
    let n: u64 = if cfg!(debug_assertions) {
        20_000
    } else {
        100_000
    };
    let mut words: Vec<String> = (0..n)
        .map(|i| format!("{:016x}", i.wrapping_mul(0x9E37_79B9_7F4A_7C15)))
        .collect();
    words.sort_unstable();
    let index: std::collections::BTreeMap<&str, usize> = words
        .iter()
        .enumerate()
        .map(|(i, w)| (w.as_str(), i))
        .collect();
    let sum: usize = words.iter().map(|w| index[w.as_str()]).sum();
    std::hint::black_box(sum);
}

/// The CPU time of [`reference_work`] on the machine the time bounds were
/// set on (an i7-12700H laptop, unloaded, best of 30 runs), in a debug and
/// a release test build. The bounds are in CPU time on that machine.
const REFERENCE_CPU: Duration = if cfg!(debug_assertions) {
    Duration::from_millis(17)
} else {
    Duration::from_millis(21)
};

/// How many times slower this thread runs now than the machine the bounds
/// were set on, never less than 1: the least CPU time of three runs of
/// [`reference_work`], over [`REFERENCE_CPU`]. Load makes a thread slower
/// even in CPU time (a busy thread on the same core, a lower clock speed, a
/// slower core, shared caches), and this measures by how much (ADR 0049).
/// Never less than 1, so a faster machine is held to the bounds as they
/// are. The same as `rappel-core`'s `test_support::slowdown`.
pub fn slowdown() -> f64 {
    let best = (0..3)
        .map(|_| cpu_time_of(reference_work).0)
        .min()
        .unwrap_or(REFERENCE_CPU);
    let slowdown = (best.as_secs_f64() / REFERENCE_CPU.as_secs_f64()).max(1.0);
    if std::env::var_os("RAPPEL_TIMINGS").is_some() {
        eprintln!("slowdown: {slowdown:.2} (reference work: {best:?} CPU)");
    }
    slowdown
}

/// Whether `used`, a CPU time of this thread, is within `limit` on the
/// machine the bounds were set on: at once if `used` is within it, else
/// once divided by the [`slowdown`] measured now, on this thread ([`judge`]).
/// Only a time over the bound pays for the measurement, and the measurement
/// is taken in the conditions the work just ran in.
pub fn cpu_within(used: Duration, limit: Duration) -> Result<(), String> {
    judge(used, limit, slowdown)
}

/// The verdict of [`cpu_within`], with the slowdown given: `used` within
/// `limit` is Ok without calling `slowdown`; otherwise `used` divided by
/// `slowdown()`, never by less than 1, must be within `limit`.
fn judge(used: Duration, limit: Duration, slowdown: impl FnOnce() -> f64) -> Result<(), String> {
    if used <= limit {
        return Ok(());
    }
    let slowdown = slowdown().max(1.0);
    let scaled = used.div_f64(slowdown);
    if scaled <= limit {
        Ok(())
    } else {
        Err(format!(
            "{used:?} of CPU time, {scaled:?} at this machine's slowdown of {slowdown:.2}, \
             more than {limit:?}"
        ))
    }
}

/// [`cpu_within`] for work that can be run again from the same state:
/// `measure` runs it and returns the CPU time it used, on this thread. A
/// time over `limit`, even at the slowdown measured then, is measured
/// again, up to [`TRIES`] times in all, and the first verdict within the
/// bound passes. A burst of load during one run (the slowdown is measured
/// after the work, and the reference does not slow down exactly as the
/// work does, ADR 0049) then fails nothing, while a regression, slow every
/// time, still fails (ADR 0063).
pub fn cpu_within_retried(
    limit: Duration,
    mut measure: impl FnMut() -> Duration,
) -> Result<Duration, String> {
    let mut last = String::new();
    for _ in 0..TRIES {
        let used = measure();
        match cpu_within(used, limit) {
            Ok(()) => return Ok(used),
            Err(message) => last = message,
        }
    }
    Err(format!("{last} (measured {TRIES} times)"))
}

/// How many times [`cpu_within_retried`] measures before it fails.
pub const TRIES: usize = 3;

/// Fails the test unless `used`, a CPU time of this thread, is within
/// `limit` ([`cpu_within`]). Call it on the thread that did the work: for
/// GTK work, inside the [`with_gtk`] or [`with_widgets`] body.
#[track_caller]
pub fn assert_cpu_within(used: Duration, limit: Duration, what: &str) {
    if let Err(message) = cpu_within(used, limit) {
        panic!("{what}: {message}");
    }
}

/// The stack hostile input is handled on in tests: a sixteenth of the main
/// thread's usual 8 MiB, where the app draws cards, so a recursion that only
/// just passes here has a wide margin in the app. A stack overflow aborts
/// the process; it cannot be caught. The same as `rappel-core`'s
/// `test_support::SMALL_STACK`, which is not exported.
pub const SMALL_STACK: usize = 512 * 1024;

/// Runs `f` on a new thread with a [`SMALL_STACK`] and returns its result.
/// Two bounds, kept apart (ADR 0049):
/// - **Performance:** the test fails if `f` used more than `limit` of CPU
///   time ([`cpu_within`], on that thread). Waiting for a CPU on a loaded
///   machine does not count, and a thread slowed by the load is measured
///   against a reference, so the bound holds under load and still fails a
///   blow-up.
/// - **Hang:** the test fails if `f` has not returned after [`hang_limit`]
///   of wall-clock time, so a hang fails the test instead of blocking the
///   run (the thread is left behind; the test process ends anyway).
///
/// `RAPPEL_TIMINGS=1` prints the CPU and wall-clock times. The same as
/// `rappel-core`'s `test_support::within`, which is not exported.
pub fn within<T: Send + 'static>(
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    within_stack(SMALL_STACK, limit, what, f)
}

/// Like [`within`], on a stack of `stack` bytes.
pub fn within_stack<T: Send + 'static>(
    stack: usize,
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, answer) = mpsc::channel();
    let wall = Instant::now();
    thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let (cpu, value) = cpu_time_of(f);
            let verdict = cpu_within(cpu, limit);
            // The receiver is gone only if the test already failed.
            let _ = sender.send((cpu, verdict, value));
        })
        .expect("tests can start a thread");
    let hang = hang_limit(limit);
    match answer.recv_timeout(hang) {
        Ok((cpu, verdict, value)) => {
            print_timing(&format!("{what} (wall)"), wall.elapsed());
            print_timing(&format!("{what} (CPU)"), cpu);
            if let Err(message) = verdict {
                panic!("{what}: {message}");
            }
            value
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("{what} hung: no result after {hang:?}"),
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("{what} panicked"),
    }
}

/// Prints `elapsed` for `what` when `RAPPEL_TIMINGS` is set, for tests that
/// time work that must stay on the main-context thread.
pub fn print_timing(what: &str, elapsed: Duration) {
    if std::env::var_os("RAPPEL_TIMINGS").is_some() {
        eprintln!("{what}: {elapsed:?}");
    }
}

/// The reference work takes a few milliseconds of measured CPU time, so a
/// CPU clock that reads zero or does not move fails here. The same check as
/// `rappel-core`'s `test_support::reference_work_is_long_enough_to_time`.
#[test]
fn reference_work_is_long_enough_to_time() {
    let least = (0..5)
        .map(|_| cpu_time_of(reference_work).0)
        .min()
        .unwrap_or_default();
    assert!(least >= Duration::from_millis(5), "{least:?}");
    assert!(slowdown() >= 1.0);
}

#[test]
fn a_time_within_its_limit_passes_without_measuring_the_slowdown() {
    let limit = Duration::from_millis(100);
    let never = || -> f64 { panic!("the slowdown is measured only over the limit") };
    assert_eq!(judge(Duration::from_millis(100), limit, never), Ok(()));
    assert_eq!(judge(Duration::ZERO, limit, never), Ok(()));
}

#[test]
fn a_time_over_its_limit_is_divided_by_the_slowdown() {
    let limit = Duration::from_millis(100);
    let measured = std::cell::Cell::new(0);
    let slowdown = |s: f64| {
        let measured = &measured;
        move || {
            measured.set(measured.get() + 1);
            s
        }
    };
    // 300 ms at three times slower is 100 ms: within.
    assert_eq!(
        judge(Duration::from_millis(300), limit, slowdown(3.0)),
        Ok(())
    );
    // Still over once divided.
    assert!(judge(Duration::from_millis(301), limit, slowdown(3.0)).is_err());
    assert!(judge(Duration::from_millis(400), limit, slowdown(2.0)).is_err());
    assert_eq!(measured.get(), 3);
    // Never divided by less than 1: a fast machine is held to the limit.
    assert!(judge(Duration::from_millis(101), limit, || 0.5).is_err());
    assert_eq!(
        judge(Duration::from_millis(150), limit, || 0.5).map_err(|m| m.contains("1.00")),
        Err(true)
    );
}

#[test]
fn a_cpu_clock_that_does_not_move_is_not_trusted() {
    let ms = Duration::from_millis;
    let at = |schedstat: Option<u64>, stat: Option<u64>| CpuReading {
        schedstat: schedstat.map(ms),
        stat: stat.map(ms),
    };
    // schedstat moved: it is used, whatever the wall clock says.
    assert_eq!(
        cpu_time_between(at(Some(5), Some(0)), at(Some(25), Some(10)), ms(900)),
        ms(20)
    );
    // Short work may not move it: zero, not the wall clock.
    assert_eq!(
        cpu_time_between(at(Some(5), Some(0)), at(Some(5), Some(0)), ms(3)),
        Duration::ZERO
    );
    // schedstat dead (`0 0 0`) on long work: stat instead.
    assert_eq!(
        cpu_time_between(at(Some(0), Some(10)), at(Some(0), Some(90)), ms(100)),
        ms(80)
    );
    // No schedstat at all: stat.
    assert_eq!(
        cpu_time_between(at(None, Some(10)), at(None, Some(30)), ms(25)),
        ms(20)
    );
    // Both dead, or neither there: the wall clock.
    assert_eq!(
        cpu_time_between(at(Some(0), Some(0)), at(Some(0), Some(0)), ms(2_000)),
        ms(2_000)
    );
    assert_eq!(
        cpu_time_between(at(None, None), at(None, None), ms(7)),
        ms(7)
    );
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;

    use super::*;

    #[test]
    fn an_x_display_holds_gtk_to_x11() {
        assert!(x11_only(Some(std::ffi::OsStr::new(":99"))));
        assert!(!x11_only(Some(std::ffi::OsStr::new(""))));
        assert!(!x11_only(None));
    }

    /// Under `xvfb-run` (or CI) the tests draw on X11, even when the
    /// session around them has a Wayland compositor (ADR 0024).
    #[test]
    fn the_tests_draw_on_x11_when_an_x_display_is_named() {
        let named = x11_only(std::env::var_os("DISPLAY").as_deref());
        let backend = with_gtk(|| {
            gtk::gdk::Display::default()
                .map(|display| display.type_().name().to_owned())
                .unwrap_or_default()
        });
        if named {
            assert_eq!(backend, "GdkX11Display");
        }
    }

    /// Text widgets in the tests use GTK's own input method (ADR 0024).
    #[test]
    fn the_tests_use_gtks_own_input_method() {
        let module = with_gtk(|| {
            gtk::Settings::default()
                .and_then(|settings| settings.gtk_im_module())
                .map(|m| m.to_string())
        });
        assert_eq!(module.as_deref(), Some(SIMPLE_IM));
    }

    /// A body that fails on the main-context thread fails the calling
    /// test with its message and place, which the test's output shows
    /// (before, nothing was printed: ADR 0073).
    #[test]
    fn a_failure_on_the_main_context_says_what_and_where() {
        let caught = panic::catch_unwind(|| {
            on_main_context(|| {
                let width = 7;
                assert!(width < 5, "the rows are {width} px wide");
            });
        });
        let payload = caught.expect_err("the body failed");
        let text = panic_text(&*payload);
        assert!(text.contains("the rows are 7 px wide"), "{text}");
        assert!(text.contains("main-context thread, at "), "{text}");
        assert!(text.contains("testing.rs:"), "{text}");
        // The thread still runs the next body.
        assert_eq!(on_main_context(|| 2 + 2), 4);
    }
}
