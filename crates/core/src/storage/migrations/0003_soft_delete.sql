-- Migration 0003: soft delete for decks and cards, and live-unique deck
-- names (ADR 0008). NEVER EDIT THIS FILE; add a new migration.
--
-- deleted_at is NULL for a live row, else the UTC milliseconds when it was
-- deleted. Deleted rows are hidden by every query but never removed, so
-- their review history stays. Deleting a deck stamps the deck and its live
-- cards with the same value, so an undo can restore exactly that batch.

ALTER TABLE decks ADD COLUMN deleted_at INTEGER;
ALTER TABLE cards ADD COLUMN deleted_at INTEGER;

-- Cards of a deck, for listing, counting and queues.
CREATE INDEX cards_deck_live ON cards(deck_id, seq) WHERE deleted_at IS NULL;

-- Two live decks cannot share a name; a deleted deck's name can be reused.
CREATE UNIQUE INDEX decks_live_name ON decks(name) WHERE deleted_at IS NULL;
