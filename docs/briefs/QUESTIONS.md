# Questions from batch mode

Questions and notes for the maintainer, written while working through `phase-1-finish.md`. Stop conditions (a migration, an export-format change, a dependency with a new licence, anything that deletes user data) are listed here, and the slice that hit one was skipped.

## Notes (no stop condition)

1. **#70 is on `phase-1` as a cherry-pick (#71).** The brief restyles the Keyboard Shortcuts and About dialogs, which only exist in #70 (open against `main`). The commit was cherry-picked unchanged. Close #70 once `phase-1` reaches `main`, or merge #70 first and rebase `phase-1`.
