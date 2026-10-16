//! `cards-core`: all logic of Cards, independent of any UI toolkit.
//!
//! This crate must never depend on gtk4, glib, gio, libadwaita or anything UI.

pub mod data_dir;
pub mod domain;
mod error;
mod id;
pub mod interchange;
pub mod scheduling;
pub mod storage;
pub mod time;

pub use error::{Error, Result};

/// Human-readable application name.
pub const APP_NAME: &str = "Cards";

/// Reverse-DNS application ID, used by GTK and the desktop environment.
pub const APP_ID: &str = "io.github.rsmirke.Cards";

/// Returns `true` if `id` is a valid application ID as defined by
/// `g_application_id_is_valid`: at least two dot-separated elements, each
/// non-empty, made of `[A-Za-z0-9_-]`, not starting with a digit, and at most
/// 255 bytes in total.
#[must_use]
pub fn is_valid_app_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 255 {
        return false;
    }
    let mut elements = 0;
    for element in id.split('.') {
        let mut chars = element.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if first.is_ascii_digit() {
            return false;
        }
        if !element
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return false;
        }
        elements += 1;
    }
    elements >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_app_id_is_valid() {
        assert!(is_valid_app_id(APP_ID));
    }

    #[test]
    fn accepts_valid_ids() {
        assert!(is_valid_app_id("org.gnome.Nautilus"));
        assert!(is_valid_app_id("a.b"));
        assert!(is_valid_app_id("io.github.some_user.App-Name"));
    }

    #[test]
    fn rejects_invalid_ids() {
        assert!(!is_valid_app_id(""));
        assert!(!is_valid_app_id("Cards"), "needs at least two elements");
        assert!(!is_valid_app_id("io..Cards"), "empty element");
        assert!(!is_valid_app_id(".io.Cards"), "leading dot");
        assert!(!is_valid_app_id("io.Cards."), "trailing dot");
        assert!(!is_valid_app_id("io.1Cards"), "element starts with digit");
        assert!(!is_valid_app_id("io.Car ds"), "space");
        assert!(!is_valid_app_id("io.Cartes✓"), "non-ASCII");
    }

    #[test]
    fn enforces_length_limit() {
        let ok = format!("a.{}", "b".repeat(253));
        assert_eq!(ok.len(), 255);
        assert!(is_valid_app_id(&ok));
        let too_long = format!("a.{}", "b".repeat(254));
        assert!(!is_valid_app_id(&too_long));
    }
}
