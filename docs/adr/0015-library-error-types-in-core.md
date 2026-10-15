# 15. Library error types in the public `Error` of cards-core

- Status: Accepted
- Date: 2026-10-15

## Context

`cards_core::Error` carried four library error types: `getrandom::Error` (`Random`), `rusqlite::Error` (`Sqlite`, `Migration`, `Backup`) and `fsrs::FSRSError` (`Fsrs`). A type that appears in a public API ties that API to the library's version: a major bump of the library changes `cards-core`'s API even when nothing else changed. Issue #13 raised this for `getrandom`.

## Decision

- **`getrandom` is wrapped.** `Error::Random` holds a `Box<dyn std::error::Error + Send + Sync>`. Its message and `source()` still give the cause. `getrandom` is an implementation detail of id generation, and it is also in the tree through `fsrs` → `rand`, so its version is not ours to choose. Nothing should depend on its error type.
- **`rusqlite` and `fsrs` stay visible.** They are the storage and scheduler that `cards-core` is built around (ADR 0005, ADR 0006). Replacing either would be a large change regardless of the error type. The app never matches on their errors; it only displays them. `cards-core` is not published (`publish = false`), so its only consumer is the app in this workspace, and a bump is one commit.

## Consequences

- A `getrandom` major bump no longer touches `cards-core`'s API.
- If `cards-core` is ever published, or gets a second front end (SPEC §2 mentions WASM or mobile), revisit this and wrap `rusqlite` and `fsrs` the same way.
