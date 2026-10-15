//! Full-window pages. Each is a `.rs` file with its layout in `data/*.blp`.

mod deck_page;
mod decks_page;
mod recovery_page;

pub use deck_page::DeckPage;
pub use decks_page::DecksPage;
pub use recovery_page::RecoveryPage;
