# 6. FSRS scheduling

- Status: Accepted
- Date: 2026-10-15
- Amended by: ADR 0009 (the Pass row for review cards), ADR 0027 (the stored desired retention is used)

## Context

SPEC says scheduling is FSRS, with two grades (Fail, Pass). `review_events` is the source of truth and `card_schedule` is a cache (ADR 0005). So the scheduler must be a pure function of the review log: replaying a card's events must reproduce the cached state exactly. The maintainer asked for the `fsrs` crate, default parameters, desired retention 0.90, one 10-minute short-term step for new and failed cards, and deterministic fuzz.

## Decision

**Crate.** `fsrs` 6.6 (Open Spaced Repetition's reference Rust implementation, BSD-3-Clause, the same code Anki uses). Since 6.x its training backend (`burn`) is only a dev-dependency. It still brings rayon, ndarray, rand, serde, snafu, strum and priority-queue, which we do not otherwise need. We use only `FSRS::default()` (the 21 FSRS-6 default parameters), `next_states` and `next_interval`. The crate has no learning steps and no fuzz; both are ours.

**Grades.** Fail is FSRS rating 1 (Again), Pass is rating 3 (Good).

**Phases.** A card with no `card_schedule` row is new. Otherwise it is in one of:

| from | Fail | Pass |
|---|---|---|
| new | learning, due +10 min | learning, due +10 min |
| learning | learning, +10 min | review, FSRS interval |
| review | relearning, +10 min, lapses + 1 | review, FSRS interval |
| relearning | relearning, +10 min | review, FSRS interval |

A new card is always shown once more 10 minutes later, even after a Pass. A card only gets a day interval after a Pass from the step. Every review, including the ones during a step, updates the FSRS memory state (stability, difficulty). FSRS-6 has its own formula for same-day reviews (elapsed days = 0), which is what a 10-minute step is.

**Elapsed days.** FSRS takes whole days since the last review. We use `round((now − last_review) / 24 h)` (12 h rounds up). This uses no time zone, so a rebuild needs only the stored UTC timestamps, and travelling or DST cannot change history. The trade-off: a review 11 h after the previous one counts as same-day, while local calendar days would have counted it as "tomorrow" if midnight fell in between. We did not choose calendar days because each event would then have to store its UTC offset.

**Intervals.** `next_interval(stability, desired_retention)` is clamped to [1, 36 500] days, then fuzzed. `due_at = now + days × 24 h`. Day boundaries (what counts as "due today") belong to the session queue, not to the scheduler.

**Fuzz.** We use Anki's ranges. Intervals under 2.5 days are not fuzzed. Otherwise the spread is ±(1 day + 15 % of the part of the interval between 2.5 and 7 days + 10 % of the part between 7 and 20 days + 5 % of the part above 20 days), with the lower end at least 2 days. Each day in the range is equally likely. The "random" number is FNV-1a over the card id bytes and the number of earlier reviews (`u32`, little-endian), mixed with SplitMix64 and mapped to [0, 1). `std`'s hasher is not used because its output may change between Rust versions. A test pins the output against values computed by an independent Python implementation.

**Rebuild.** `Scheduler::rebuild_state` sorts a card's events by `(reviewed_at, id)` and folds `Scheduler::schedule` over them. The cache and a rebuild agree as long as the log is recorded in that same order:
- `schedule` refuses a review dated before the card's last review (`Error::ReviewBeforeLastReview`). Otherwise a clock that went backwards would let `schedule` apply events in an order that the time-sorted rebuild would not reproduce. Code that records reviews must therefore store `reviewed_at = max(now, last_review_at)`.
- Two reviews in the same millisecond are ordered by id. The recording code must use a `UUIDv7` generator that is monotonic within a millisecond.

Property tests check that rebuilding equals scheduling review by review. They also check every memory state against `fsrs`'s own `memory_state` replay, and that going back in time is refused.

**Cache shape (migration 0002).** `card_schedule` is dropped and recreated with a `phase` column. The same migration transaction rebuilds it from `review_events`. If the rebuild fails, the migration rolls back too, and the error is `Error::MigrationRebuild { version }`. Any later migration that reshapes the cache sets `rebuild_schedule: true`.

**Desired retention.** `Scheduler::new` accepts 0.80–0.97 (SPEC §3). There is no settings table yet, so the migration rebuild uses 0.90. A rebuild uses the *current* desired retention for the whole history, so changing the setting reschedules every card, as it does in Anki. When the settings slice lands, the migration rebuild and any rebuild must read the stored value.

## Consequences

- Scheduling is a pure function of the review log, the card id and the desired retention, and is easy to test.
- FSRS works in `f32`, and SQLite `REAL` (`f64`) stores every `f32` exactly, so a cache row equals a rebuild bit for bit on the same machine. A rebuild on a different CPU or libm could differ in the last bit of a `powf`. Such differences can add up over many reviews and can move a fuzz range boundary, so due dates may shift slightly (typically by a day). That only matters if a database moves between machines.
- The undo slice will make `rebuild_state` skip events cancelled by an `undoes` event (SPEC §3).
- Optimising FSRS parameters from the user's log (SPEC §5 "Later") can reuse the same crate.
