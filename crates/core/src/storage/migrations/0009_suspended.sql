-- Migration 0009: suspended cards (ADR 0035). NEVER EDIT THIS FILE; add a
-- new migration.
--
-- A suspended card is excluded from study and from the schedule's
-- statistics, and kept otherwise: its content, tags and review log are
-- untouched. 0 for every existing card.

ALTER TABLE cards ADD COLUMN suspended INTEGER NOT NULL DEFAULT 0 CHECK (suspended IN (0, 1));
