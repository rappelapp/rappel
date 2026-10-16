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

use gtk::glib;

type Job = Box<dyn FnOnce() + Send>;

thread_local! {
    /// Whether this thread is the one that owns the default main context.
    static ON_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// The queue of the thread that owns the default main context, started on
/// first use.
fn queue() -> &'static mpsc::Sender<Job> {
    static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
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
/// its result. A panic in `f` fails the calling test (a panic inside a C
/// callback that `f` runs aborts the test binary instead: it cannot unwind
/// through C). Called from that thread, it runs `f` directly: queueing it
/// would wait for itself.
pub fn on_main_context<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    if ON_WORKER.get() {
        return f();
    }
    let (sender, answer) = mpsc::channel();
    let job: Job = Box::new(move || {
        // The caller may have stopped waiting; nothing to do then.
        let _ = sender.send(panic::catch_unwind(AssertUnwindSafe(f)));
    });
    queue()
        .send(job)
        .expect("the main-context thread runs for the whole test run");
    match answer
        .recv()
        .expect("the main-context thread answers every job")
    {
        Ok(value) => value,
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Like [`on_main_context`], with GTK started first. Fails without a display:
/// run the tests in a desktop session or under `xvfb-run -a` (ADR 0024).
pub fn with_gtk<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    on_main_context(|| {
        gtk::init().expect("GTK needs a display for this test; run it under `xvfb-run -a`");
        f()
    })
}
