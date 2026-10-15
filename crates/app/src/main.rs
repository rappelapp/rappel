//! Cards: a native GNOME flashcard app.

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
    app.connect_activate(|app| {
        let window = app
            .active_window()
            .unwrap_or_else(|| CardsWindow::new(app).upcast());
        window.present();
    });

    tracing::debug!("starting {}", cards_core::APP_NAME);
    app.run()
}
