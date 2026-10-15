//! Cards: a native GNOME flashcard app.

mod window;

use std::cell::Cell;
use std::path::PathBuf;

use adw::prelude::*;
use cards_core::data_dir::{self, BuildProfile, DataDirEnv};
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

    // Nothing needs the directory yet, so a failure is reported and the app
    // still starts. Taken by the first window so the toast shows only once.
    let startup_error = Cell::new(prepare_data_dir().err());

    let app = adw::Application::builder()
        .application_id(cards_core::APP_ID)
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let window = CardsWindow::new(app);
        if let Some(message) = startup_error.take() {
            window.add_toast(
                adw::Toast::builder()
                    .title(message)
                    // The message contains a path, which may contain `<` or `&`.
                    .use_markup(false)
                    // Stays until dismissed: the user must not miss it.
                    .timeout(0)
                    .build(),
            );
        }
        window.present();
    });

    tracing::debug!("starting {}", cards_core::APP_NAME);
    app.run()
}

/// Resolves and creates the data directory. Returns the path, or a short
/// user-facing message after logging the full error.
fn prepare_data_dir() -> Result<PathBuf, String> {
    let profile = if cfg!(debug_assertions) {
        BuildProfile::Debug
    } else {
        BuildProfile::Release
    };
    let env = DataDirEnv::from_process();
    if profile == BuildProfile::Debug
        && env
            .cards_data_dir
            .as_ref()
            .is_some_and(|dir| !dir.as_os_str().is_empty())
    {
        // The guard only knows the default release locations; real data kept
        // in a custom CARDS_DATA_DIR inherited from the session is not caught.
        tracing::warn!("debug build is using CARDS_DATA_DIR; make sure it is not real data");
    }
    let result = data_dir::resolve_data_dir(&env, profile)
        .and_then(|dir| data_dir::ensure_data_dir(&dir).map(|()| dir));
    match result {
        Ok(dir) => {
            tracing::info!(path = %dir.display(), ?profile, "data directory");
            Ok(dir)
        }
        Err(err) => {
            tracing::error!("data directory unavailable: {err}");
            Err(match err {
                cards_core::Error::DebugUsesReleaseDir { .. } => {
                    "Debug build refused to use the release data folder".to_owned()
                }
                cards_core::Error::CreateDataDir { path, .. }
                | cards_core::Error::NotADirectory(path) => {
                    format!("Could not create the data folder {}", path.display())
                }
                cards_core::Error::RelativeDataDir(_) => {
                    "CARDS_DATA_DIR must be an absolute path".to_owned()
                }
                cards_core::Error::NoHomeDir => "Could not find the data folder".to_owned(),
            })
        }
    }
}
