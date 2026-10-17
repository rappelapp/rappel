//! Full-window pages. Each is a `.rs` file with its layout in `data/*.blp`.

mod deck_page;
mod decks_page;
mod recovery_page;
mod review_page;
mod stats_page;

pub use deck_page::DeckPage;
pub use decks_page::DecksPage;
pub use recovery_page::{Recovered, RecoveryPage};
pub use review_page::ReviewPage;
pub use stats_page::StatsPage;
