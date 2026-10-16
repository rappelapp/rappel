//! Cards: a native GNOME flashcard app.

mod clock;
mod dialogs;
mod interchange;
mod pages;
mod render;
mod startup;
mod state;
mod ui;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use crate::window::CardsWindow;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            // Without RUST_LOG, still show warnings and errors.
            EnvFilter::builder()
                .with_default_directive(LevelFilter::WARN.into())
                .from_env_lossy(),
        )
        .init();

    if let Err(err) = gio::resources_register_include!("cards.gresource") {
        tracing::error!("failed to register bundled resources: {err}");
        return glib::ExitCode::FAILURE;
    }

    let app = adw::Application::builder()
        .application_id(cards_core::APP_ID)
        .build();
    app.set_accels_for_action("win.new", &["<Control>n"]);
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        // The database is opened here, in the one running instance, and not
        // in main(): a second launch only forwards `activate` to it, and must
        // not open (back up, migrate) the database too.
        let window = CardsWindow::new(app);
        match startup::start() {
            Ok(state) => window.show_main(state),
            Err(problem) => window.show_problem(problem),
        }
        window.present();
    });

    tracing::debug!("starting {}", cards_core::APP_NAME);
    app.run()
}
