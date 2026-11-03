-- Migration 0012: archived decks (ADR 0068). NEVER EDIT THIS FILE; add a
-- new migration.
--
-- archived_at is NULL for a deck that is not archived, else the UTC
-- milliseconds when it was archived. An archived deck is live (its name
-- stays taken, its cards stay in All Cards and in exports) but is left
-- out of the deck list, study and the global forecast and retention.
-- NULL for every existing deck.

ALTER TABLE decks ADD COLUMN archived_at INTEGER;
