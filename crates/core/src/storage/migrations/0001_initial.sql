-- Migration 0001: v0 schema. NEVER EDIT THIS FILE; add a new migration.
--
-- Ids are lowercase UUIDv7 strings (36 characters). Timestamps are UTC
-- milliseconds since the Unix epoch. review_events is the source of truth;
-- card_schedule is a cache that can be rebuilt from it.

CREATE TABLE decks (
    id         TEXT    PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND id = lower(id)),
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE cards (
    -- Internal, stable integer key (FTS5 content_rowid). Never exported or
    -- shown; `id` is the card's identity.
    seq        INTEGER PRIMARY KEY,
    id         TEXT    NOT NULL UNIQUE CHECK (length(id) = 36 AND id = lower(id)),
    deck_id    TEXT    NOT NULL REFERENCES decks(id) ON DELETE RESTRICT,
    front_md   TEXT    NOT NULL,
    back_md    TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX cards_deck_id ON cards(deck_id);

CREATE TABLE review_events (
    id          TEXT    PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND id = lower(id)),
    card_id     TEXT    NOT NULL REFERENCES cards(id) ON DELETE RESTRICT,
    reviewed_at INTEGER NOT NULL,
    grade       TEXT    NOT NULL CHECK (grade IN ('fail', 'pass')),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0)
) STRICT;

CREATE INDEX review_events_card_reviewed ON review_events(card_id, reviewed_at);

-- review_events is append-only. Undo is a new event (SPEC §3).
CREATE TRIGGER review_events_no_update BEFORE UPDATE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review_events is append-only');
END;

CREATE TRIGGER review_events_no_delete BEFORE DELETE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review_events is append-only');
END;

-- Cache of the scheduler state, rebuilt from review_events. A card without
-- a row here is new. Later migrations may drop and recreate this table.
CREATE TABLE card_schedule (
    card_id        TEXT    PRIMARY KEY NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    due_at         INTEGER NOT NULL,
    stability      REAL    NOT NULL,
    difficulty     REAL    NOT NULL,
    last_review_at INTEGER NOT NULL,
    reps           INTEGER NOT NULL CHECK (reps >= 0),
    lapses         INTEGER NOT NULL CHECK (lapses >= 0)
) STRICT;

CREATE INDEX card_schedule_due_at ON card_schedule(due_at);
