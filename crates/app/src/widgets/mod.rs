//! Reusable widgets. Each is a `.rs` file; the row widgets have their
//! layout in `data/*.blp`, and `CardObject` is a list item with none.

mod card_object;
mod card_row;
mod deck_row;

pub use card_object::CardObject;
pub use card_row::CardRow;
pub use deck_row::DeckRow;
