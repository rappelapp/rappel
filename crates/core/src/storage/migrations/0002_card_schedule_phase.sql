-- Migration 0002: reshape the card_schedule cache for the FSRS scheduler
-- (ADR 0006). NEVER EDIT THIS FILE; add a new migration.
--
-- card_schedule is a cache of review_events, so it is dropped and
-- recreated instead of altered. The migration runner rebuilds it from the
-- review log inside this migration's transaction (rebuild_schedule).
-- Nothing references this table, so dropping it cannot cascade.

DROP TABLE card_schedule;

-- One row per card that has been reviewed at least once. A card without a
-- row here is new.
CREATE TABLE card_schedule (
    card_id        TEXT    PRIMARY KEY NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    phase          TEXT    NOT NULL CHECK (phase IN ('learning', 'review', 'relearning')),
    due_at         INTEGER NOT NULL,
    -- FSRS f32 values; REAL holds every f32 exactly.
    stability      REAL    NOT NULL,
    difficulty     REAL    NOT NULL,
    last_review_at INTEGER NOT NULL,
    reps           INTEGER NOT NULL CHECK (reps >= 1),
    lapses         INTEGER NOT NULL CHECK (lapses >= 0)
) STRICT;

CREATE INDEX card_schedule_due_at ON card_schedule(due_at);
