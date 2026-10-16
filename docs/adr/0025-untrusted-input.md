# 25. Untrusted input: review, attack tests and limits

- Status: Accepted
- Date: 2026-10-16

## Context

SPEC §1 principle 5 says card content is untrusted. Issue #42 asked for worst-case tests of the Markdown parser, RaTeX and the importer, measurements at 5,000 cards with 2,000 due, and a check of the import's memory peak. This ADR reviews every place where content Rappel did not write itself comes in. Each finding is either fixed (with a test) or accepted (with a reason).

The work is split into two PRs: core first (`chore/hardening-core`), then the app (`chore/hardening-app`). Rows marked **PR 2** are finished there, and this ADR is updated with them.

## Decision

### How hostile input is tested

1. **proptest, in the gate.** cargo-fuzz needs a nightly compiler and runs outside `cargo test`. proptest is already a dev-dependency and runs on every `scripts/check.sh`. The random cases are generated from syntax, not from raw bytes, so they reach the parsers' corner cases. Heavy properties run few cases (24 for the 256 KiB Markdown sides).
2. **No hang: a time bound on a separate thread.** `test_support::within(limit, …)` runs the work on a new thread and fails the test when the limit passes, so a hang fails the gate instead of stalling it. Bounds are for a debug test build, at about 10 times the slowest measured value, so slow CI machines pass. `RAPPEL_TIMINGS=1` prints the measurements.
3. **No stack overflow: a small stack.** That thread has a 512 KiB stack, a sixteenth of the main thread's usual 8 MiB. A stack overflow aborts the process, and `catch_unwind` cannot catch it, so the margin has to come from the tests.
4. **No unbounded memory.**
   - A parsed side never has more nodes than its input has bytes, and never more text than three times the input plus one byte per node. That covers a NUL (1 byte) becoming U+FFFD (3 bytes) and a line break between blocks.
   - The import's peak memory is measured as a process peak (`VmHWM`, reset through `/proc/self/clear_refs`), in a test binary of its own (`crates/core/tests/import_memory.rs`). This needs no counting allocator, which would need `unsafe` or a new crate.

### Entry points

| # | Where it comes in | Risk | Status | Test |
|---|---|---|---|---|
| 1 | **Card text from the editor.** At most 1 MiB per side, no NUL (`card_side_fits`, `has_nul`). | Slow or deep parse; huge trees. | **OK.** 44 hand-made 1 MiB hostile sides parse in 0.56 s at worst in a debug build, and their one-line summary in 12 ms. Random syntax repeated to 256 KiB also passes. Depth stays within `MAX_DEPTH`, and node and text counts stay within the input size. | `content::tests::hostile_sides_*`, `repeated_random_syntax_*` |
| 2 | **Card text drawn with Pango.** | Markup injection. | **OK, existing.** Every piece is escaped by `inline_markup`, and labels with card text have markup off. | `render::markup` tests |
| 3 | **A huge side drawn on the review page.** | Pango lays out up to 1 MiB in one label: the UI freezes (the editor preview already cuts at 32 KiB for this reason). | **PR 2**: measure, and cap if too slow. | PR 2 |
| 4 | **Deck names.** At most 1 KiB, no NUL. | Markup injection; look-alike names. | **Markup: OK.** Deck rows, toasts, alert bodies and page titles are plain text. **Look-alikes (bidi overrides, confusables): accepted.** The names are the user's own, or come from a file the user chose to import, and a conflict message names the user's own deck, never the file's (`transfer::names`). | No test: `use-markup: false` in `deck_row.blp` and `ui::toast` (read in this review) |
| 5 | **LaTeX.** | Deep nesting, `\frac` towers, macro expansion bombs (`\def\a{\a\a}`), huge `\rule`s; a panic inside RaTeX. | **Mostly OK, PR 2 for the tests.** RaTeX has its own depth guard and expansion budget (`ratex-parser` `stack_safety.rs`, `expansion_budget.rs`). Rappel adds the 4 KiB source cap, the 128 em size cap and a `catch_unwind` (ADR 0023). PR 2 adds the proptest, the small-stack runs and a `compile_error!` if `panic = "abort"`. | PR 2 |
| 6 | **The import file: which file.** | A FIFO blocks `open` forever: the import worker never ends, and since only one job runs at a time, Import and Export stay disabled until restart. A device such as `/dev/zero` never ends. | **Fixed.** Only a regular file is read (`ImportProblem::NotAFile`), checked before opening (through symbolic links) and again on the open file. **Accepted:** a FIFO swapped in between those two checks still blocks. The path comes from the user's own file chooser, and only someone who can already write to the user's files could do the swap. | `refused::only_regular_files_are_imported` |
| 7 | **The import file: zip.** | Zip-slip, bombs, lying headers, overlapping entries, zip64, encryption. | **OK, existing:** nothing is extracted; the names are compared as bytes; the count is checked against the end record; an end record with zip64 markers, a comment, encryption and methods other than Stored and Deflate are refused (zip64 extra fields on an entry are accepted: the size limits still apply to what is read); decompression stops one byte past the limit; the CRC is checked. **Fixed:** two central records pointing at the same bytes (overlapping entries, the trick of "better" zip bombs) were accepted. That was harmless, since only `cards.json` is read, but it is not a file Rappel writes, so it is now refused (the entries must not overlap and must end before the central directory; each entry's extent is taken from its local header). Every header field set to lying values, byte flips, cuts and inserts: each is refused, or plans exactly as the undamaged file. | `fuzz::lying_headers_*`, `flipped_bytes_*`, `cut_or_padded_*`, `random_bytes_*`, `zip_tricks_are_refused`, `an_entry_reaching_into_the_central_directory_is_refused` |
| 8 | **The import file: JSON.** | Deep nesting (stack), invalid UTF-8, NUL, huge numbers, duplicate keys; card text quoted in an error. | **OK, existing:** the free JSON values of the format (`settings` values, `media` items) are where serde_json recurses, and it stops at depth 128 with an error (tested there on a 512 KiB stack up to a depth of 1,000,000, checked to be the recursion error); everywhere else a nested value is a type error at once, or skipped without recursion. Invalid UTF-8 and duplicate keys are refused; unknown fields are denied; messages give a place, never text. Random JSON values and damaged JSON never put the planted marker text into a message. | `fuzz::random_json_*`, `damaged_json_*`, `deeply_nested_json_*`, `invalid_utf8_*`, `duplicate_keys_*` |
| 9 | **The import file: memory.** Limits 256 MiB of file and 256 MiB of JSON. | The archive, the JSON and the document were all held at once. | **Fixed and measured.** The archive is dropped once `cards.json` is out, and the JSON once it is parsed. Peak per byte of JSON, measured at 32 MiB: 3.49 bytes for a stored archive of review events (4.48 before the fix), 1.64 deflated, 1.36 for long cards. Extrapolated to the limit (8 times the measured peak), that is **about 895 MiB** at worst, against about 1,150 MiB before the fix, which was over the 1 GiB budget. With the fix the limits stay. The margin is about 13 %: a format change that makes the parsed document larger per row must re-check this. The worst case runs first in the test, so memory kept by the allocator from another case cannot hide its peak. The plan keeps the document while the confirmation dialog is open. | `tests/import_memory.rs` |
| 10 | **File names and paths.** | Paths from a file, path traversal, markup. | **OK.** Zip entry names are never used as paths, and a refused name is shown escaped and cut to 64 characters. The export path comes from the file chooser, and the file is written atomically with mode `0600`. Backup names are made by Rappel and listed from its own directory; a restore validates a copy first (ADR 0007). `RAPPEL_DATA_DIR` is the user's own environment, and debug builds refuse the release directory (ADR 0004). Paths in toasts are plain text. | `refused::archive_entries_*`, `data_dir` tests |
| 11 | **Links in cards.** | Opening a URL from a card. | **OK.** No `<a>` in markup and no URI launcher anywhere: the URL is only a plain-text tooltip. | `render::markup` tests |
| 12 | **Images** (SPEC §4.3, not built yet). | Decoder bombs, SVG scripts, remote fetches, paths. | **Not built; requirements for that slice:** check the pixel size from the header before decoding, keep the 10 MB cap, no SVG, file names made from the hash and never taken from the file, `media:` URLs only. Today, every image shows as text. | — |
| 13 | **Logs.** SPEC §2: no card content above `debug`. | Card or deck text in an error's `Display` logged at `warn`/`error`. | **Core: OK.** Every core `warn!`/`error!` (backup, prune, restore, integrity, orphaned files) logs paths and SQLite or IO errors only. The one error carrying user text, `DuplicateDeckName`, comes only from create, rename and undelete, which the app logs at debug level (`ui::toast::log_failure`). The migration 0004 backfill fails with SQLite's `UNIQUE constraint failed: decks.name_key`, which has no value. `InvalidColumn` holds a grade, phase, count or id read from the database, never text. **Paths in logs: accepted.** The log stays on the user's machine, and a path is what makes a failure fixable. **App: PR 2** (the recovery page and `window.rs` log `{err}` directly). | PR 2 |
| 14 | **Pasting or dropping into the editor.** | Same as row 1. | **OK.** The text goes into GtkSourceView as plain text; a save checks the same limits. | `check_card` tests |

### Speed at 5,000 cards (issue #42 items 1–2)

Debug test build, in-memory store, one deck of 5,000 cards with 2,000 due (`storage/tests/perf.rs`):

| Operation | Measured | Bound |
|---|---|---|
| `list_decks` | 12 ms | 150 ms |
| `study_input` + `next_study_change` (each deck-page change) | 14 ms | 150 ms |
| `list_cards` | 6 ms | 100 ms |
| `decks` / `deck` (new) | 0.01 ms | at least 10 times faster than `list_decks` |
| `plan_import` of the 5,000-card export | 80 ms | 1 s |

`Store::deck` and `Store::decks` read decks without counting their cards. The deck list's open, rename and delete use them in PR 2 (item 1).

## Consequences

- The gate takes about 10 s longer (the hostile Markdown sides, the import fuzz and the 32 MiB memory test).
- A file Rappel did not write, with overlapping zip entries, is refused. No file Rappel ever wrote has them.
- Importing a pipe or a device says "it is not a regular file (a folder, a pipe or a device)" instead of hanging.
- The memory test runs on Linux only (`/proc`). The app targets Linux.
- New entry points (images, tags, search) add their rows here.
