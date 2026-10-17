-- Migration 0006: the settings table. NEVER EDIT THIS FILE; add a new
-- migration.
--
-- Key/value, one row per setting the user has changed; a missing key means
-- the default (ADR 0027). Keys: desired_retention, max_new_per_day,
-- max_reviews_per_day, day_start_hour. Values are validated on write by the
-- store and again on read.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
