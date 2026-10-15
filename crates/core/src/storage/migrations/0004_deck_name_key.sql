-- Migration 0004: deck names unique by key, not by exact spelling (ADR
-- 0010). NEVER EDIT THIS FILE; add a new migration.
--
-- name_key is the trimmed name in Unicode NFC, lower-cased. SQL cannot
-- compute NFC, so the migration's Rust backfill step fills it in for every
-- deck (deleted ones too), after this SQL and in the same transaction.
-- Two live decks with the same key make that step fail, and the migration
-- rolls back.

DROP INDEX decks_live_name;

-- Nullable only because SQLite cannot add a NOT NULL column without a
-- default; the triggers below refuse NULL from now on.
ALTER TABLE decks ADD COLUMN name_key TEXT;

CREATE UNIQUE INDEX decks_live_name_key ON decks(name_key) WHERE deleted_at IS NULL;

-- A unique index treats NULLs as distinct, so a NULL key would bypass it.
CREATE TRIGGER decks_name_key_required_on_insert BEFORE INSERT ON decks
WHEN NEW.name_key IS NULL
BEGIN
    SELECT RAISE(ABORT, 'decks.name_key is required');
END;

CREATE TRIGGER decks_name_key_required_on_update BEFORE UPDATE ON decks
WHEN NEW.name_key IS NULL
BEGIN
    SELECT RAISE(ABORT, 'decks.name_key is required');
END;
