-- Migration 0007: kinds of log entries, for undo and reset (ADR 0028).
-- NEVER EDIT THIS FILE; add a new migration.
--
-- review_events stays the one ordered, append-only log of a card. Next to
-- reviews it now holds resets ("new from here") and undos (an entry that
-- cancels an earlier review or reset; SPEC §3 `undoes`). A reset or an
-- undo has no grade, and ALTER TABLE cannot drop NOT NULL or change a
-- CHECK, so the table is rebuilt: create, copy, drop, rename.
--
-- No other table references review_events, so dropping it cascades
-- nowhere and needs no change to foreign_keys. SQLite drops a table's
-- triggers before the implicit DELETE that DROP TABLE performs, so the
-- append-only trigger does not fire (https://sqlite.org/lang_droptable.html;
-- tested in storage/tests/migrations.rs). The self-reference names the new
-- table; RENAME rewrites it to review_events.
--
-- reviewed_at keeps its name: it is when the entry was made, whatever its
-- kind. Every existing row is a review.

CREATE TABLE review_events_v7 (
    id          TEXT    PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND id = lower(id)),
    card_id     TEXT    NOT NULL REFERENCES cards(id) ON DELETE RESTRICT,
    kind        TEXT    NOT NULL CHECK (kind IN ('review', 'reset', 'undo')),
    reviewed_at INTEGER NOT NULL,
    grade       TEXT    CHECK (grade IN ('fail', 'pass')),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    undoes      TEXT    REFERENCES review_events_v7(id) ON DELETE RESTRICT,
    -- A review has a grade, and only a review.
    CHECK ((kind = 'review') = (grade IS NOT NULL)),
    -- An undo names the entry it cancels, and only an undo.
    CHECK ((kind = 'undo') = (undoes IS NOT NULL)),
    -- Only a review takes time.
    CHECK (kind = 'review' OR duration_ms = 0)
) STRICT;

INSERT INTO review_events_v7 (id, card_id, kind, reviewed_at, grade, duration_ms, undoes)
SELECT id, card_id, 'review', reviewed_at, grade, duration_ms, NULL FROM review_events;

DROP TABLE review_events;

ALTER TABLE review_events_v7 RENAME TO review_events;

CREATE INDEX review_events_card_reviewed ON review_events(card_id, reviewed_at);

-- An entry is cancelled at most once. NULLs (every entry that is not an
-- undo) are distinct, so they do not collide.
CREATE UNIQUE INDEX review_events_undoes ON review_events(undoes);

-- review_events is append-only. Undo is a new entry (SPEC §3).
CREATE TRIGGER review_events_no_update BEFORE UPDATE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review_events is append-only');
END;

CREATE TRIGGER review_events_no_delete BEFORE DELETE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review_events is append-only');
END;

-- An undo cancels a review or a reset of the same card, made no later
-- than the undo, and with a smaller id, so replaying the log in
-- (reviewed_at, id) order meets the target first. The foreign key already
-- requires the target to exist.
CREATE TRIGGER review_events_undo_target BEFORE INSERT ON review_events
WHEN NEW.kind = 'undo'
BEGIN
    SELECT RAISE(ABORT, 'an undo must cancel an earlier review or reset of the same card')
    WHERE NOT EXISTS (
        SELECT 1 FROM review_events t
        WHERE t.id = NEW.undoes
          AND t.card_id = NEW.card_id
          AND t.kind IN ('review', 'reset')
          AND t.reviewed_at <= NEW.reviewed_at
          AND t.id < NEW.id
    );
END;
