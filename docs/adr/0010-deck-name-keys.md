# 10. Deck names unique by key

- Status: Accepted
- Date: 2026-10-15
- Amends: ADR 0008 ("live-unique deck names", which compared names exactly)

## Context

ADR 0008 made deck names unique among live decks by exact comparison. That allows "Maths" and "maths" side by side. It also allows two names that look identical: "Économie" typed with a precomposed "É" (U+00C9) and "Économie" pasted with "E" + combining acute accent (U+0301).

## Decision

**Key.** Each deck has a `name_key`: the trimmed name in Unicode **NFC**, then **lower-cased**, then in NFC again (`domain::deck_name_key`). Case mapping does not promise NFC output, and Unicode's recommended caseless matching normalises on both sides. The partial unique index moves from `name` to `name_key` (`WHERE deleted_at IS NULL`). The displayed name is kept exactly as typed (trimmed).

- NFC makes composed and decomposed accents equal.
- Lower-casing (Rust's full Unicode `to_lowercase`) makes case differences equal.
- **Accents still count**: "Économie" and "Economie" are different decks. In French, e and é can distinguish words, so merging them would be wrong.
- This is lower-casing, not full case folding, so "STRASSE" and "straße" are different. That is acceptable for deck names.

**Dependency.** NFC needs Unicode data that std does not ship, so I added `unicode-normalization` 0.1 (unicode-rs, MIT OR Apache-2.0). Its one dependency is `tinyvec` (Zlib OR Apache-2.0 OR MIT).

**Migration 0004.** SQL cannot compute NFC, so the migration framework gained an optional Rust step (`Migration::backfill`). It runs after the SQL, in the same transaction, before the schedule rebuild, `user_version` and the foreign-key check, and its error rolls the migration back (`Error::MigrationBackfill`). 0004 does the following:
- drops the old index, adds `name_key`, and creates the new unique index;
- fills `name_key` in for every deck, deleted ones included, so a later undelete has a key;
- if two live decks now share a key, refuses the migration; the database stays at v3, with the pre-migration backup in place. No released build could create decks, so no such database should exist.
- `name_key` has to be nullable, because SQLite cannot add a NOT NULL column without a default. Triggers refuse an INSERT or UPDATE that leaves it NULL. Without them, a NULL key would slip past the unique index, which treats NULLs as distinct.

**Stability.** Keys are stored, so the key function must keep giving the same result for stored names. NFC is stable under Unicode's normalization stability policy. Lower-casing can only change for characters newly added to Unicode. If a Rust upgrade or a change to `deck_name_key` changes any key, a new migration must recompute the keys, like any backfill.

**Sorting.** `list_decks` sorts by the same key, then id.

## Consequences

- Creating or renaming a deck to a name that differs only in case or Unicode form gives `DuplicateDeckName`. Renaming a deck to another case of its own name is allowed.
- Every raw SQL insert into `decks` must set `name_key`; the triggers enforce it.
