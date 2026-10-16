//! Toasts: persistent ones for errors and warnings, and Undo toasts.

use adw::prelude::*;
use rappel_core::storage::OpenWarning;

/// A toast with plain text (paths and errors may contain `<` or `&`) that
/// stays until dismissed, so the user cannot miss it.
pub fn persistent_toast(text: &str) -> adw::Toast {
    adw::Toast::builder()
        .title(text)
        .use_markup(false)
        .timeout(0)
        .build()
}

/// Shows `toast` in the toast overlay that contains `widget`.
pub fn show_toast_from(widget: &impl IsA<gtk::Widget>, toast: adw::Toast) {
    if let Some(overlay) = widget
        .ancestor(adw::ToastOverlay::static_type())
        .and_downcast::<adw::ToastOverlay>()
    {
        overlay.add_toast(toast);
    } else {
        tracing::error!("widget is not inside a toast overlay");
    }
}

/// Logs that `what` failed, e.g. "a deck operation". `message` may hold
/// user content (names, card text), so it is only logged at debug level
/// (SPEC §2).
pub fn log_failure(what: &str, message: &str) {
    tracing::warn!("{what} failed");
    tracing::debug!("{message}");
}

/// Logs the failure (see [`log_failure`]) and shows `message` in a
/// persistent toast near `widget`.
pub fn show_error(widget: &impl IsA<gtk::Widget>, what: &str, message: &str) {
    log_failure(what, message);
    show_toast_from(widget, persistent_toast(message));
}

/// A toast offering Undo after something was deleted at once.
pub fn undo_toast(title: &str, on_undo: impl Fn() + 'static) -> adw::Toast {
    let toast = adw::Toast::builder()
        .title(title)
        .use_markup(false)
        .button_label("_Undo")
        // Never queued behind a toast that stays until dismissed.
        .priority(adw::ToastPriority::High)
        .build();
    toast.connect_button_clicked(move |_| on_undo());
    toast
}

/// The toast text for a warning from opening the database.
pub fn warning_message(warning: &OpenWarning) -> String {
    match warning {
        OpenWarning::DailyBackupFailed(err) => format!("Today’s backup could not be made: {err}"),
        OpenWarning::PruneFailed(err) => format!("Old backups could not be removed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn warnings_name_what_failed_and_why() {
        let err = || rappel_core::Error::NotADirectory(PathBuf::from("/data/backups"));
        let text = warning_message(&OpenWarning::DailyBackupFailed(err()));
        assert!(text.starts_with("Today’s backup could not be made: "));
        assert!(text.contains("/data/backups"));
        let text = warning_message(&OpenWarning::PruneFailed(err()));
        assert!(text.starts_with("Old backups could not be removed: "));
    }
}
