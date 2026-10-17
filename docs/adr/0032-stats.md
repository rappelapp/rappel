# 32. Statistics and card information in core

- Status: Accepted
- Date: 2026-10-17
- Amends: SPEC §4.7 (time studied per day and the card information are listed), ADR 0028 (what the statistics count: activity and memory are counted differently)
- Refers to: #39 (card information screen)

## Context

SPEC §4.7 wants reviews per day, pass rate, a due forecast and the expected retention, per deck and global, as pure functions in `core::stats`. Issue #39 wants a card's properties and history. ADR 0028 left a rule for them: count surviving entries only. A reset makes this ambiguous: it restarts a card's memory, but the studying before it did happen. Four things needed a decision: what counts, how days are cut without a time zone in core, which cards are covered, and how to stay fast at 5,000 cards and 100,000 reviews.

## Decision

**What counts: activity and memory.** Two rules, because a reset means two things.
- **Activity** (reviews, passes and time per day, pass rate) counts every *surviving review*: a review no undo cancels, whatever resets came later. The studying happened, so Reset Progress does not change the past days' bars. An undo removes the review everywhere. `stats::activity_reviews` states this for one card's log; the SQL in `storage/stats.rs` states it for the database and is tested against it.
- **Memory** (expected retention, due forecast, scheduled cards, and a card's reps, lapses, stability, difficulty, next due) starts afresh at a surviving reset: a card with a surviving reset and no review since has no schedule. This is the cache, i.e. the replay of ADR 0028. `stats::counted_reviews` is the same restart for counting a card's reviews.
- The **daily limits** keep their own definition (ADR 0028: reviews after the card's last surviving reset, `COUNTED_REVIEWS` in `storage/review.rs`), because a reset card is offered as new again.

**Days.** Core knows no time zones (ADR 0006, 0014). The caller passes a `DayGrid`: strictly increasing UTC timestamps at which consecutive study days begin (n + 1 bounds for n days), which the app computes with `clock.rs` (GLib, already tested across daylight-saving days) from the stored day start hour. A 23- or 25-hour day is only a shorter or longer bucket, so core needs no calendar code. A grid with fewer than two bounds, or bounds not strictly increasing, is `Error::InvalidDayGrid`. The app passes 31 bounds for the last 30 days (today last) and 15 for the next 14 (today first).

**The figures.**
- Reviews, passes and time per day: surviving reviews in the past grid. Time is the sum of the recorded durations (already capped at 60 s).
- Pass rate: passes over reviews in the past grid, all phases, 10-minute steps included; `None` without reviews.
- Due forecast: live scheduled cards by the day their `due_at` falls on, from the cache. Day 0 also takes every overdue card; `overdue` says how many were due before the grid began.
- Expected retention: the mean over live scheduled cards of `fsrs::current_retrievability` with `FSRS6_DEFAULT_DECAY`, the decay of the default parameters the scheduler uses (a test checks that recall after the scheduler's own interval equals the desired retention). Elapsed time is a fraction of days (to the second) since the last review, not the scheduler's rounded whole days: it is a value for this moment. `None` without scheduled cards.

**Which cards.** Live cards in live decks only, for every figure and every scope (`Scope::All` or `Scope::Deck`). Deleting a card or deck takes it out of the statistics, and the per-deck figures add up to the global ones exactly (the mean retention through `scheduled_cards`).

**Card information.** `Store::card_info(card, now)` replays the surviving log with `Scheduler::schedule`, as the cache is rebuilt, and returns the dates, the reviews, passes and lapses since the last surviving reset (its memory side), stability, difficulty, retrievability now and next due, and the history: every surviving review (time, grade, duration, the interval it gave) and reset (a marker), oldest first. Undone entries are left out. Reviews before a reset stay in the history; the card's own counts start after the reset.

**SQL and Rust.** SQL picks the surviving reviews of the window (one scan) and the live schedules; Rust buckets them. That is simpler than a grid table in SQL and fast enough.

## Consequences

- The UI slice only builds two grids from `clock.rs` and draws `Stats`; it does no counting.
- Timings at 5,000 cards and about 100,000 reviews, debug build: global 0.22 s, one deck 0.15 s, `card_info` of a card with 2,000 reviews 9 ms. The tests bound them at about 10 times that (ADR 0025).
- A card's information is replayed on demand, so it cannot disagree with the cache; a property test checks it does not.
- The pass rate (activity) counts every answer, so a card in its 10-minute step weighs as much as a review. A "true retention" figure (review-phase answers only) can be added later without changing the log.
- No migration: the activity query needs no index. (A partial index on resets was tried for the reset-aware rule and dropped with it.)
