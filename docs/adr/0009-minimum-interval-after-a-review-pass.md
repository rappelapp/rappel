# 9. Minimum interval after a review Pass

- Status: Accepted
- Date: 2026-10-15
- Amends: ADR 0006 (the Pass row for review cards)

## Context

ADR 0006 schedules a Pass with FSRS's interval for the new stability, fuzzed. For a hard card that is reviewed long after it was due, that interval can be shorter than the gap it just survived. Example: a card due after 1 day, recalled after 30 days, rescheduled in 8. Recall after 30 days is evidence the card lasts at least that long, so scheduling it sooner wastes a review. Anki applies a floor here; the review of the core-ops slice flagged that we did not.

## Decision

**Floor.** After a Pass on a card in the **review** phase, the interval is at least `elapsed + 1` days. `elapsed` is the rounded whole days since the last review, the same value FSRS receives (ADR 0006). The floor is capped at the 36 500-day maximum interval.

**Before and after fuzz, as Anki does.** The fuzz range is computed for `max(FSRS interval, floor)`. Both ends of the range are then raised to the floor, and the day is picked uniformly inside that range. The result is never below the floor, and the cards are not lumped onto the floor day. A floor of 1 changes nothing, so the code has one path (`fuzz::floored_fuzzed_days`). Anki also widens a range that collapsed to a single day above 2 days. That is not copied: here a range can only collapse when the floor is at most 2 and FSRS's interval is under 2.5 days, where no fuzz is wanted anyway.

**Learning and relearning.** A Pass from the 10-minute step has no floor beyond one day, because the gap there is minutes, not days.

**Purity.** The floor depends only on the previous state and the review time, so the scheduler is still a pure function of the log, and rebuild = review-by-review still holds. The property test now also checks that every review Pass satisfies `days ≥ elapsed + 1` and lands within the floored fuzz range.

**A scheduler change ships with a cache rebuild.** Any change to what `schedule` returns makes existing `card_schedule` rows differ from a rebuild. Such a change must therefore come with a migration that has `rebuild_schedule: true`. Here that is migration 0005, which has no schema change, only the rebuild. This rule applies to every future scheduler change.

## Consequences

- Late Passes on hard cards come back later than before. Cards reviewed on time are unaffected, because FSRS's interval is above the floor.
- Existing due dates may move when migration 0005 runs, but only for cards whose log contains such a late Pass.
- The FSRS memory state (stability, difficulty) is unchanged. Only the due date is affected, so the oracle comparison with `fsrs::memory_state` still holds.
