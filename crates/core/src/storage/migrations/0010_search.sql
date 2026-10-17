-- Migration 0010: the full-text search index (ADR 0035). NEVER EDIT THIS
-- FILE; add a new migration.
--
-- A cache, like card_schedule: the rows are written by the store, in the
-- transaction of every write of a card's sides, from the plain text of the
-- Markdown (what the user reads), folded to lower case without accents.
-- The rowid is cards.seq. The migration's Rust step fills it for the
-- existing cards. A change to the indexed text needs a new migration that
-- rebuilds it.

CREATE VIRTUAL TABLE cards_fts USING fts5(
    front, back,
    tokenize = 'unicode61 remove_diacritics 0'
);
