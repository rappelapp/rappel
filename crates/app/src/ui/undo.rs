//! What a failed undo means, for the review page's Ctrl+Z and the Undo
//! toast of a reset. Both undo a log entry (`Store::undo`, ADR 0028, 0030).

use rappel_core::Error;

/// Why an undo did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoFailure {
    /// The entry is no longer the card's latest, or the card is gone: it
    /// changed since, e.g. an import or another reset added entries. Nothing
    /// was written, and trying again cannot work.
    Stale,
    /// Anything else (the database): nothing was written, and trying again
    /// may work.
    Failed,
}

/// Which kind of failure `err` is.
#[must_use]
pub fn failure(err: &Error) -> UndoFailure {
    match err {
        Error::NotLatestEntry(_) | Error::EventNotFound(_) | Error::CardNotFound(_) => {
            UndoFailure::Stale
        }
        _ => UndoFailure::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_log_is_stale_and_other_errors_are_failures() {
        for err in [
            Error::NotLatestEntry("e".to_owned()),
            Error::EventNotFound("e".to_owned()),
            Error::CardNotFound("c".to_owned()),
        ] {
            assert_eq!(failure(&err), UndoFailure::Stale);
        }
        assert_eq!(failure(&Error::TagNameHasComma), UndoFailure::Failed);
    }
}
