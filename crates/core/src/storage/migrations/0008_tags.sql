-- Migration 0008: tags (ADR 0029). NEVER EDIT THIS FILE; add a new
-- migration.
--
-- Flat tags, unique by name_key: the trimmed name in Unicode NFC,
-- lower-cased, as for decks (ADR 0010). The store computes it on write;
-- the table starts empty, so nothing needs a backfill. Tags are never
-- deleted (an unused tag stays, so its id is reused when the name comes
-- back); the cascades only follow SPEC §3.

CREATE TABLE tags (
    id       TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36 AND id = lower(id)),
    name     TEXT NOT NULL,
    name_key TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE card_tags (
    card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    tag_id  TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (card_id, tag_id)
) STRICT, WITHOUT ROWID;

-- The cards of a tag.
CREATE INDEX card_tags_tag ON card_tags(tag_id, card_id);
