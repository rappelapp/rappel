# Questions from batch mode

Questions and notes for the maintainer, written while working through `phase-1-finish.md`. Stop conditions (a migration, an export-format change, a dependency with a new licence, anything that deletes user data) are listed here, and the slice that hit one was skipped.

## Notes (no stop condition)

1. **#70 is on `phase-1` as a cherry-pick (#71).** The brief restyles the Keyboard Shortcuts and About dialogs, which only exist in #70 (open against `main`). The commit was cherry-picked unchanged. Close #70 once `phase-1` reaches `main`, or merge #70 first and rebase `phase-1`.

2. **A damaged stored setting stops the app with no way on** (ADR 0050 row 37). If a value in the `settings` table is out of range (a damaged or hand-edited database), Rappel shows "Rappel Could Not Start" with the cut error and nothing else. Resetting the value to its default would overwrite user data, so batch mode left it. Options: offer "Reset This Setting" on that page (one row, with a backup taken first), or keep refusing and point to Restore a backup. Which do you want?
   *Answered by the Phase 2 brief (offer "Reset This Setting" after a backup); done in Phase 2 slice 0, ADR 0052.*
3. **Restoring a backup still replaces a waiting restore copy** (ADR 0007, noted in ADR 0050 row 35). This was already so before batch mode. The copy is the user's data. Should a restore set it aside (like Start Fresh now does) instead?
   *Answered by the Phase 2 brief (set it aside); done in Phase 2 slice 0, ADR 0052: it is kept as `rappel.sqlite3.restore.set-aside-…`.*
4. **Open PRs against `main` that overlap `phase-1`.** #69 (release 0.2.0-alpha.4) bumps the same version lines as 0.2.0-beta.1 on `phase-1`: close it, or merge it first and resolve the CHANGELOG heading. #56 (disable Reset Progress for new cards) touches the card rows, the review store and ADR 0028, which `phase-1` also changed: rebase it after `phase-1` reaches `main`. #70 see note 1. #35, #32 and #25 were not touched.
5. **Tags are never deleted** (ADR 0029). A typo tag that was added and then undone stays in the tag suggestions and the tag filter. Hiding tags no card uses from the suggestions is a small phase 2 change; deleting unused tags would be a data change and needs your decision.
   *Answered by the Phase 2 brief (suggestions only offer used tags); done in Phase 2 slice 0, ADR 0052. The tag filter hides unused tags too; no tag is deleted.*
6. **The draft PR `phase-1` → `main`.** The brief asks for it, while batch mode says never to target `main`. Batch mode opened it as a **draft** only. The guard blocks merging it, and nothing was pushed to `main`. Merging it is yours.

## Phase 2

1. **Confirm four ignored "unmaintained" advisories** (slice 3, images, ADR 0055; not a stop condition). The image decoder glycin brings `paste` (RUSTSEC-2024-0436, a build-time macro) and `zvariant`, `zvariant_derive`, `zbus_names` (RUSTSEC-2026-0370, -0372, -0371: merged into zbus 6). None is a vulnerability and none has a fixed version; glycin 4.0.3 has the same tree. `deny.toml` ignores them with reasons, to review on every glycin release and by 2027-01-16 at the latest, as ADR 0021 did for `ttf-parser`. Note that `zvariant` parses the replies of glycin's loader process, which a hostile image could take over, so it sits on the sandbox boundary inside Rappel's process. Keep the ignores, or decode images another way (GTK's in-process loaders, no sandbox)?
   *Answered (Phase 3a brief): keep the ignores; review them on every glycin release and by 2027-01-16 at the latest.*
2. **SPEC §4.9 asks for an allow-list of audio decoders; there is none yet** (slice 4, audio, ADR 0056; not a stop condition, the slice ships without it). Formats are allow-listed by their structure over the whole file (`media::check_structure`: MP3 frames, Ogg pages of one Opus or Vorbis stream, WAV PCM or float, M4A boxes without references or other tracks), and GStreamer's streaming demuxers and network sources are demoted to rank none (`GST_PLUGIN_FEATURE_RANK`, `gst_env.rs`). But GStreamer still chooses the demuxer and decoder itself, in Rappel's process (FFmpeg's where `gstreamer1.0-libav` is installed). A real decoder allow-list needs `gstreamer-rs` (MIT/Apache-2.0, licences already allowed) and the GStreamer development packages (`libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev`, which need sudo and are not installed here), then either an `autoplug-select` veto in playbin or Rappel's own pipeline instead of `GtkMediaFile`. Install the packages so the phase 2 refactor and hardening slice can add it, or accept the deviation from SPEC §4.9 (and amend it)?
   *Done in slice 10, without gstreamer-rs (ADR 0062): `GStreamer`'s own `GST_PLUGIN_LOADING_WHITELIST` lets only 21 plugins load in Rappel's process (not FFmpeg's `libav`), with a registry of Rappel's own. No packages to install. Cost: an Apple Lossless M4A, or an AAC one where FFmpeg is the only AAC decoder, does not play.*

## Phase 3a

None of these was a stop condition: the brief pre-approved the migration (0012), the export field `archived_at` and the soft deletion of a merged deck. Each is a decision taken in the run that you may want to reverse.

1. **The import keeps a card's own deck** (slice 3, #114, ADR 0067).
   - Decided: a card that differs from the database only in its deck is "already present", and the database keeps its deck. The import plan says how many cards stay where they are ("3 cards are in another deck in the file and stay in the ones they are in now."). A card that differs in its deck and in its content is still a conflict.
   - Why: before, every moved card was a conflict, and any conflict refuses the whole import. So an export made before any Move to Deck or merge could never be imported again, and exports are the backups.
   - The cost: a reorganisation on machine A is not carried to machine B by import. A new deck that A made for the moved cards arrives empty on B, and after a merge an older backup brings the merged deck back empty.
   - Question: keep this rule?
*Answered (3 Nov): as recommended — keep the rule: a card that differs only in its deck is already present and the database keeps its deck.*
2. **0.3.0-beta.1 refuses every export from this version** (slice 4, #115, ADR 0068).
   - Decided: every deck record now has `archived_at` (`null` when not archived), written always, as the brief pre-approved. 0.3.0-beta.1's reader refuses unknown fields, so it refuses the whole file. The CHANGELOG warns testers.
   - Why: "written always" keeps exports deterministic and the same as for `suspended` (ADR 0035); nothing is stable before 1.0.
   - Question: acceptable for a beta, given that testers moving files between the two betas will hit it? (The alternative is to leave the field out when it is `null`, which changes the export format's rule.)
*Answered (3 Nov): as recommended — acceptable for a beta: `archived_at` is written always, and 0.3.0-beta.1 refuses these exports.*
3. **Suspend, Move to Deck and Delete stay available during practice** (slice 6, #118, ADR 0070).
   - Decided: they work as in a session, with their own Undo toasts, and take the card out of the practice. Edit and Card Information stay too.
   - Why: they are explicit card commands, not practice answers, and they do not reschedule anything, so "Practice: nothing was rescheduled" stays true. The reviewer recommended keeping them.
   - Question: keep them, or hide them during practice?
*Answered (3 Nov): as recommended — keep Suspend, Move to Deck and Delete available during practice.*
4. **How slice 7 was "redone" on Opus** (#121).
   - Background: the brief's model line says Opus for everything. At the start of the run you overrode it in chat (Sonnet for slices 0, 1, 7 and 9, Opus for 2–6 and 8, the reviewer always on Opus, and a Sonnet slice needing more than two review rounds redone on Opus); after slice 7 you withdrew that ("no more Sonnet, everything on Opus"), so 7b, 8 and 9 ran on Opus.
   - Decided: the Sonnet implementation of slice 7 needed a third review round (its round-2 fix, a far too high size estimate, stopped users with 1.5–2 GiB of media from making a full export). Under your rule, an Opus implementer took over the same branch, re-examined every commit, kept the work it verified and fixed the rest in new commits; a fresh Opus reviewer then reviewed the whole slice. Nothing was reset or rewritten from scratch.
   - Why: most of the slice was sound and tested, and history is never discarded.
   - Question: is that how a "redo" should work next time, or do you want a rewrite from the integration branch?
*Answered (3 Nov): as recommended — a "redo" is an Opus takeover on the same branch, keeping the verified work and fixing the rest in new commits, not a rewrite from the integration branch.*
5. **`docs/briefs/phase-3a.md` does not list slice 7b (#119, done in #122) or #117 (done in slice 7, #121)**, which you added during the run. SPEC §4 lists 7b as Phase 3a item 9 and the report covers both.
   - Question: should the brief be amended for the record? It is your file, so I left it.
*Answered (3 Nov): as recommended — amend the brief for the record (done in Phase 3b slice 0).*
6. **Under 400 px, selection mode's title is just the count** (slice 7, #121, ADR 0071).
   - Decided: "30" instead of "30 Selected", with the full text as the tooltip and as the screen reader's label. "Select Cards" stays when nothing is chosen.
   - Why: with minimise and maximise shown at 360 px, "30 sélectionnées" was cut.
   - Question: keep the bare number, or find a shorter French wording and keep words?
*Answered (3 Nov): as recommended — keep the bare number under 400 px ("30"), with the full text as tooltip and screen-reader label.*
7. **Two points of CLAUDE.md's CI budget, left for you in #112** (CLAUDE.md is yours, so I did not edit it):
   - "nor add docs to a code PR after CI has passed" can clash with fixing reviewer findings, which are often docs. Suggested wording: "batch such fixes into one push".
   - The docs-only check classes `.claude/` (the guard hook and the agents) as docs, so a change to the guard skips the gate. Should `.claude/hooks/` run it?
   - Still unchecked: whether `gh pr checks` (and so batch mode's wait) counts a skipped `check` as passing. No docs-only PR was opened after #111 and #112: #109 came before them and ran the gate, and every later PR changed code. The first docs-only PR will show it.
*Answered (3 Nov): as recommended — take the suggested CLAUDE.md wording ("batch such fixes into one push") and run the gate on `.claude/hooks/` changes. The skipped-`check` question stays open until the first docs-only PR shows it.*
