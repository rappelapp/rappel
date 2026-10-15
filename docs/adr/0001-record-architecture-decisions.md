# 1. Record architecture decisions

- Status: Accepted
- Date: 2026-10-15

## Context

Cards is built in small slices, mostly by an AI coding agent, and reviewed by a maintainer who does not read most diffs. Decisions made in one slice (a crate choice, a storage format, a rendering approach) must be visible and justified later, without digging through chat logs or commit history.

## Decision

We record every significant architecture decision as an Architecture Decision Record (ADR), as described by Michael Nygard in [Documenting Architecture Decisions](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).

- ADRs live in `docs/adr/`, named `NNNN-short-kebab-title.md`, numbered in order and never renumbered.
- Each ADR has: title, status (Proposed, Accepted, Deprecated, Superseded by NNNN), date, Context, Decision, Consequences.
- An accepted ADR is not rewritten. To change a decision, write a new ADR that supersedes it and update the old one's status line only.
- A slice that makes a significant decision adds its ADR in the same PR.

"Significant" means: hard to reverse, affects several modules, adds a major dependency, changes the data format or storage, or picks between real alternatives.

## Consequences

- The reasoning behind the code stays in the repository, next to it.
- Each significant slice has a small extra writing cost.
- The reviewer agent checks that significant decisions come with an ADR.
