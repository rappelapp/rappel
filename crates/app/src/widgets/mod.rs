//! Reusable widgets. Each is a `.rs` file with its layout in `data/*.blp`.

mod card_object;
mod card_row;
mod deck_row;

pub use card_object::CardObject;
pub use card_row::CardRow;
pub use deck_row::DeckRow;
