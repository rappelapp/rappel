//! Running blocking work (export, import, restore) on a worker thread, so
//! the window never freezes.

use gtk::gio;
use gtk::prelude::*;

/// The worker panicked; its work may be half done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panicked;

/// Runs `f` on GIO's thread pool and returns its result on the main
/// thread. While `f` runs, the application of `widget`'s window is held, so
/// closing the window lets the work finish instead of cutting it off. Only
/// then: a chooser or dialog left open must not keep a windowless process
/// alive.
pub async fn run<T: Send + 'static>(
    widget: &impl IsA<gtk::Widget>,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Panicked> {
    let _hold = widget
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.application())
        .map(|app| app.hold());
    join(f).await
}

/// Runs `f` on GIO's thread pool. `gio::spawn_blocking` catches a panic in
/// `f` and returns it as the error (gio's `task.rs`, `spawn_blocking`).
async fn join<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, Panicked> {
    gio::spawn_blocking(f).await.map_err(|_| Panicked)
}

#[cfg(test)]
mod tests {
    use gtk::glib;

    use super::*;

    #[test]
    fn a_worker_returns_its_value_or_that_it_panicked() {
        crate::testing::on_main_context(|| {
            let context = glib::MainContext::default();
            assert_eq!(context.block_on(join(|| 6 * 7)), Ok(42));
            let panicked: Result<(), Panicked> =
                context.block_on(join(|| panic!("the worker failed")));
            assert_eq!(panicked, Err(Panicked));
        });
    }
}
