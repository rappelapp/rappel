# 17. Export / import format and merge rules

- Status: Accepted
- Date: 2026-10-16
- Amends: SPEC §4.1 (one JSON file with base64 media)
- Closes: #12

## Context

SPEC §4.1 asks for a full export of the database and an import that works into an empty or an existing database, with a summary before anything is written. The export is the user's way to keep or move their data, so it must not lose anything (deleted rows and the whole review log included). The import reads a file from outside the app, so it is untrusted. #12 showed one concrete risk: ADR 0008's ordering argument holds only if every stored id is a `UUIDv7` with the RFC variant.

## Decision

**A zip archive.** An export is a `.cards` file: a zip archive with exactly two entries, `cards.json` and an empty `media/` directory. Media (SPEC §4.3) will go into `media/` as files named by their hash, instead of base64 inside the JSON, which would make the JSON a third larger and force the whole thing into memory as one string. The zip is read and written with the `zip` crate, with only Deflate enabled (through `zlib-rs`, pure Rust): no encryption and no other codecs, so less code reads untrusted input.

**`cards.json`, format version 1.** It holds `format_version`, `exported_at`, `decks`, `cards`, `review_events`, `settings` (an empty object for now) and `media` (an empty list for now). Every row is exported, deleted ones with their `deleted_at`. The derived or internal columns are left out: `decks.name_key` (recomputed), `cards.seq` (internal) and `card_schedule` (a cache, rebuilt from the log). Tags are not in version 1 because no tags table exists yet. The slice that adds them also adds them to the format as version 2. A reader accepts every version up to its own and refuses newer ones.

**Deterministic bytes.**
- Lists are sorted by id, struct fields have a fixed order, and `settings` is a `BTreeMap`.
- The JSON is pretty-printed with a trailing newline.
- Zip entries come in a fixed order, with the zip format's earliest timestamp (1980-01-01) instead of the clock, fixed permissions, no extra fields and no comment.

So the same database exported at the same time gives identical bytes; only `exported_at` changes from one export to the next. Identical holds for one build: another Deflate implementation could compress differently, though the JSON inside stays identical. The export is written to `<path>.partial` (mode `0600`), synced, then renamed.

**Import in two steps.** `plan_import` writes nothing: it reads and checks the file, then compares it with the database in a read transaction. `apply_import` writes.

**Merge rules, by id.**
- An id not in the database is added.
- The same id with exactly the same exported fields is already present and is skipped.
- The same id with different content is a conflict.
- A live deck in the file whose name key is used by another live deck in the database is a conflict too.

Any conflict refuses the whole import. Merging field by field would need rules for which side wins, and a wrong guess would lose data silently. Refusing keeps both sides intact and makes the user decide. Rows are only ever added: nothing in the database is changed or deleted by an import.

**Untrusted file.** Each of these refuses the file with a precise `ImportProblem` that names the place in the file but never quotes its text:
- The file is over 256 MiB, or `cards.json` over 256 MiB once decompressed. Reading stops one byte past the limit whatever the entry claims, which cuts off zip bombs.
- The archive's entries are not exactly `cards.json` and `media/`. Names are compared byte for byte, so `../`, absolute paths, other files, and a name used twice (detected by comparing with the count in the archive's end record) are all refused. Nothing is ever extracted to disk, so zip-slip cannot happen.
- An entry is encrypted or uses an unsupported compression method. A CRC mismatch is also refused.
- `format_version` is not supported. It is checked before the rest of the file, so a newer file is reported as newer, not as malformed.
- The JSON has unknown fields (the message gives only the kind of error and its line and column, because serde's own message can quote any amount of the file), missing fields (`deleted_at` must be present, even when it is `null`) or values of the wrong type.
- An id is not a lowercase hyphenated `UUIDv7` with variant `10`, appears twice in its list, or carries a timestamp after the year 9999. `IdGen` issues every new id above the largest one stored, so an id near the top of the range (year 10889) would leave it no room, and every later write would fail with `IdsExhausted`. That could not be repaired, because rows are never removed.
- A card's deck or an event's card is not in the file.
- A grade is invalid, a duration is negative, or a timestamp falls outside 1970 to 9999.
- A deck name is empty or untrimmed, a card front is empty, a card side is over 1 MiB, or a deck name is over 1 KiB.
- Two live decks in the file share a name key, or a live card is in a deleted deck (states the app never produces).
- `settings` or `media` is not empty.

**Foreign ids are refused, not re-issued (#12).** Re-issuing would need remapping every reference. It would also break the merge rules: the same file imported twice would add everything twice, because its rows would get new ids each time. Every export Cards writes passes the check. The optional `CHECK` tightening in #12 (a migration) is left for later; today only `IdGen` and this checked import write ids.

**Apply.**
1. If nothing is new, apply returns at once. Nothing is written and no backup is made, so importing a file twice is a true no-op.
2. Otherwise the database is backed up first with the online backup API, as before a migration (ADR 0007). If the backup fails, there is no import. Old backups are pruned as at startup.
3. In one `IMMEDIATE` transaction the file is compared again. If the result differs from the plan, the import is refused (`ImportPlanStale`). Rows are never removed, so an equal summary means the same rows are new.
4. The new rows are inserted.
5. The `card_schedule` row of every card that got events is rebuilt from its whole log. Imported events may come before or between the existing ones, and the replay order (time, then id) handles both.

An in-memory store has no backup directory and refuses to import.

## Consequences

- Export, import into an empty database, export again gives the same bytes. This is tested on fixed and random databases, together with the schedule cache.
- An import never edits existing data. To bring in a changed card, the user has to resolve the conflict first. A UI that offers this (for example "import as a copy") would be a later decision.
- New dependencies: `zip` (MIT), with `flate2` and `zlib-rs` (MIT/Apache-2.0, Zlib), `serde` and `serde_json` (MIT/Apache-2.0).
- The 1 KiB deck name, 1 MiB card side and 256 MiB file limits are not enforced when decks and cards are written. A larger deck name or card side would export but not import; nothing in the app comes close today. Enforcing them when writing is a follow-up.
- A review dated in the future (up to the year 9999) is accepted. Refusing reviews after `exported_at` would also refuse the app's own exports after its clock went backwards. Such a review keeps that card's later reviews at its date (`max(now, last review)`, ADR 0008).
- Each import that adds rows makes a backup, and pruning keeps the newest 14. Many imports in one day can push the older daily backups out (ADR 0007).
- Imported cards get new internal `seq` values, so in a deck's card list they appear after the cards that were already there, even if they were created earlier.
