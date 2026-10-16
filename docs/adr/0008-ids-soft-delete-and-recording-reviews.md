# 8. Ids, soft delete and recording reviews

- Status: Accepted
- Date: 2026-10-15
- Amended by: ADR 0010 (deck names compared by their key), ADR 0012 (undeleting a deck)

## Context

The first operations on decks, cards and reviews need three things the earlier slices left open:
- an id generator;
- a delete that keeps review history;
- a way to record a review so the `card_schedule` cache always equals a rebuild from the log.

ADR 0006 sets the rule the log must follow: `rebuild_state` replays a card's events in `(reviewed_at, id)` order. So a new event must sort after every earlier event of its card, even when the clock goes backwards, several reviews share a millisecond, or the app restarted in between.

## Decision

**UUIDv7 as a 122-bit counter** (`crates/core/src/id.rs`).
- Without its 6 fixed version and variant bits, a `UUIDv7` is the number `v = ts · 2^74 + r`. Here `ts` is the 48-bit Unix millisecond time and `r` is 74 random bits. The lowercase string order of ids is the order of `v`.
- `IdGen` remembers the largest `v` it has issued or observed and returns `max(candidate, last + 1)`. The candidate is the current time with fresh random bits from `getrandom`. So ids strictly increase:
  - within one millisecond (no counter field is needed: `+1` carries into `ts` if `r` overflows);
  - when the clock goes backwards.
- The 74 random bits keep ids from two devices apart, for a future sync or import.
- The ordering argument assumes every stored id is a `UUIDv7` with variant `10`. `parse_id` drops the version and variant bits, so a foreign UUID (say v4 or v8) could be the string `max(id)` while sorting differently as a 122-bit value. Only `IdGen` writes ids today; **import must check that every id is a `UUIDv7`** (or re-issue ids). A stored id that is not a UUID at all is refused: a write that would need to issue above it fails with `InvalidColumn` instead of risking an out-of-order id.
- `getrandom` was already in the build through `fsrs → rand`. I wrote the 60 lines myself instead of using the `uuid` crate, because `uuid`'s `ContextV7` cannot be told to issue above an id read from the database.
- **Every write that issues an id first observes the largest id of its table** (`SELECT max(id)`, an index lookup), in the same transaction. Observing only the reviewed card's last event is not enough. In the same millisecond, a previous run may have issued `x` for card A and `x + 1` for card B. A fresh generator that observed only `x` would issue `x + 1` again. The property test found this case. Observing on every write instead of once at open also covers a second process writing to the same database.

**Soft delete** (migration 0003).
- `decks.deleted_at` and `cards.deleted_at` are NULL for live rows, else the UTC millisecond of the delete.
- Every deck, card and queue operation hides deleted rows. A deleted card's history and cache stay readable by its id (`review_events`, `card_state`). Operations on a deleted deck or card fail with `DeckNotFound` or `CardNotFound`, as if it did not exist.
- Nothing is ever removed, so `review_events` (with `ON DELETE RESTRICT`) and their history stay intact. A deleted card's `card_schedule` row also stays. It is a cache, hidden by the queries, and a rebuild recreates it anyway.
- **Deleting a deck stamps the deck and all its live cards with one shared `deleted_at` value**, in one transaction. That value is `max(now, latest deleted_at among the deck's cards + 1)`. So it is strictly later than any card of the deck deleted on its own before, even within the same millisecond or with a clock that went backwards.
- A future undo of a deck delete can therefore restore exactly that batch: `UPDATE cards SET deleted_at = NULL WHERE deck_id = ? AND deleted_at = <the deck's stamp>`, plus the deck. Cards deleted separately before stay deleted. There is no undelete in this slice.

**Deck names are unique among live decks.**
- A partial unique index `decks_live_name ON decks(name) WHERE deleted_at IS NULL` enforces it.
- Names are trimmed, and the comparison is exact (case-sensitive). A deleted deck's name can be reused.
- The index's constraint error is mapped to `Error::DuplicateDeckName`. There is no separate check-then-insert, so there is nothing to race.
- A database with two live decks of the same name would fail migration 0003 and stay at v2 (tested). No released build could create decks, so no such database exists.

**Recording a review is one `BEGIN IMMEDIATE` transaction.**
1. Check the card is live.
2. Read the cached state and the card's latest `reviewed_at`.
3. Use `t = max(now, latest)`.
4. Observe the log's largest id and issue the new id.
5. Call `Scheduler::schedule` at `t`.
6. Insert the event and upsert `card_schedule`.

Any error rolls back both writes. The rebuild and `record_review` write cache rows through the same helper.
- The property test runs random reviews, clock jumps (backwards, zero, up to 60 days) and simulated restarts (a fresh `IdGen`). It checks that the cache equals `rebuild_state` for every card, that a full `rebuild_schedule_cache` changes no byte of the cache, and that each card's log replays in recording order.

**Queues and counts** ignore daily limits for now. Settings and limits arrive with the review slice. `due_cards` gives reviewed cards with `due_at <= now`, most overdue first. `new_cards` gives never-reviewed cards in creation order, up to a limit. Deck lists are sorted by lower-cased name in Rust, not by locale collation.

## Consequences

- Ids are not secrets and are partly predictable (`+1` steps). Nothing uses them for security.
- Each new deck, card or review costs one extra index lookup.
- Soft-deleted rows accumulate. A future "empty trash" can only remove decks and cards without reviews, because of `ON DELETE RESTRICT`. Purging history would need its own decision.
- `Store` now owns a `Scheduler` with the default desired retention. When settings exist, it must be built from the stored retention (ADR 0006).
