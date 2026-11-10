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
*Answered (3 Nov): as recommended (the direction only). Follow-up not done in phase 3b and still open: CLAUDE.md is the maintainer's file, so its CI-budget wording is his to change; a `.claude/hooks/` filter in `ci.yml` and an ADR 0003 amendment are not scheduled. The skipped-`check` question also stays open until the first docs-only PR shows it.*

## Phase 3b

1. **STOP CONDITION, slice 7 (the snap): glycin's image sandbox does not run under strict confinement** (ADR 0082, finding 2). Nothing was packaged and the image code is unchanged.
   - Found: bubblewrap cannot make its sandbox inside a strict snap. `unshare(CLONE_NEWUSER)` gives `EPERM` in two strict snaps here, and the platform's own `bwrap` stops with "Can't read /proc/sys/kernel/overflowuid: Permission denied". Canonical's GNOME platform snap (`gnome-core26`) patches its glycin for this ("We cannot run bwrap within snap confinement, but we can still set a seccomp filter at least"). Only `browser-support` with `allow-sandbox` grants user namespaces, which the brief excludes, and bubblewrap would still lack `mount`.
   - Worse than a weaker sandbox: with today's code (`SandboxSelector::Auto`), glycin 3.1 does not recognise that error, picks bubblewrap anyway, and **every image would be "could not be shown"** in the snap. A future bubblewrap that fails at `unshare` instead would make glycin fall back to a plain child with no seccomp and no memory limit, with only a log line.
   - What strict confinement gives instead: the loader runs inside the snap's own AppArmor and seccomp profile, with no network and no home folder (no `network`, `home` or `removable-media` plugs). It can still read and change what Rappel can: the database, the media, the backup folder granted through the portal. Under bubblewrap it reaches none of them.
   - Options:
     - **A. Accept the snap's confinement as the image boundary.** In Rappel's own snap (`SNAP_NAME` is `rappel` and the running executable is under `$SNAP`; not just `$SNAP` set, which snaps such as an editor's terminal pass on to the programs they start, so a native Rappel started there keeps bubblewrap), Rappel selects `SandboxSelector::NotSandboxed` explicitly, so images work. The sizes are still checked, the 10 s timeout still applies, and SVG is still refused before decoding. There is no seccomp filter on the loader and no memory limit (glycin 3.1's plain child gets neither, and adding them would mean patching glycin, which needs `unsafe` in `pre_exec`). ADR 0055 row 42 and SPEC §4.3 would say so, and the log keeps its warning. The snap ships in 0.4.0 as planned.
     - **B. A, plus upstream work.** Propose a snap mode to glycin (seccomp and a memory limit for the plain child, as Canonical's patch does) and adopt it when released. glycin 4.x needs Rust 1.93, one above the workspace's floor.
     - **C. Flatpak first.** In a Flatpak, glycin's `flatpak-spawn` sandbox works. Move the snap to phase 3c, and release 0.4.0 without a store package (or as a Flatpak bundle, pulled forward from 3c).
     - **D. A snap without images:** keep `Auto`, and images stay placeholders. Not recommended: the store screenshots show pictures.
   - Recommendation: **A now, with B as follow-up.** The snap still confines the whole app (no network, no home), which the unprivileged-container case ADR 0055 already accepts does not have. SPEC §4.3 already says "sandboxed where the system allows". And Canonical's own GNOME platform makes the same trade-off. The residual risk is a decoder exploit, from a hostile image in an imported deck, reaching Rappel's own data and backups, with no memory limit against a bomb. Choose C if the image sandbox must stay an invariant on every package.

2. **The snap needs a `dbus` slot for the app id** (ADR 0082, "Also found"; not a stop condition, but a permission outside the brief's list). GApplication owns `io.github.rappelapp.Rappel` on the session bus, and strict confinement refuses that unless the snap declares `slots: { rappel-name: { interface: dbus, bus: session, name: io.github.rappelapp.Rappel } }`. This is a slot, not a plug, and gives no access to anything else; many GNOME snaps declare it. The alternative is to run without single-instance behaviour in the snap (a second launch would open, back up and possibly migrate the same database while the first has it open, which ADR 0011 and restore, ADR 0007, rely on never happening). Recommendation: allow the slot. Not checked on a real snap (no root here); the policy read is in the ADR.

3. **Where the snap keeps the collection** (ADR 0082, finding 4). By default it goes to `~/snap/rappel/<revision>/.local/share/rappel`. snapd copies that folder on every refresh and keeps the copies of the retained revisions, so `snap revert` also brings back the data from before a migration, but the media folder is stored two or three times, and a revert shows the collection as it was at the refresh: reviews and edits made since stay only in the newer revision's folder, which snapd deletes when that revision is dropped. The alternative is `RAPPEL_DATA_DIR=$SNAP_USER_COMMON/rappel`: one copy, but a revert keeps the migrated database, which the older version then refuses as "too new". Recommendation: the default (per revision), because a revert must never meet a database migrated past what it can read; the README says that after a revert the newest work is in the other revision's folder (and how to copy it back with Export / Import), and mentions the disk use.

4. **The snap's grade**, for when packaging resumes. The channel is already decided (brief, 9 Nov: 0.4.x goes to beta; stable waits for 1.0). Recommendation: `grade: stable`. The store refuses `devel` grade on candidate and stable, so `stable` keeps the later move to stable a channel change only, and the channel, not the grade, says "beta".

5. **Confirm that the automatic backup may remove its own leftover partial files** (slice 9, #141, ADR 0084; outside the letter of the brief's pre-approved deletion rule, which allows deleting "regular files of its own exact pattern beyond the 14 newest"). *What it deletes:* only hidden files named exactly as the job names its partial files (`.Rappel backup YYYY-MM-DD.rappel.<16 lowercase hex digits>.partial`, with a real date), only regular files (checked without following links, and again just before deleting), only when not modified for at least 24 hours, and only after a backup succeeded or today's backup was found already there; a failed run deletes none. Links, folders, the user's own `.partial` files and every other name stay. *Why it is safe:* a running backup is cut off after five minutes and 30 seconds, so a day-old partial file is not being written; the name has 64 random bits, so it is never a user's file; nothing outside the folder can be reached (`unlinkat` on the folder's handle never follows a link). *Why it is needed:* a backup killed by a shutdown or a crash leaves its partial file, as large as a backup; on a computer often shut down while Rappel closes, one piles up per day without bound. *The alternative:* leave them, and document in the README and the Preferences group's description that `.Rappel backup ….partial` files may be deleted by hand. Recommendation: keep the rule (it is in #141, behind this question); if you prefer the alternative, the change is to remove `remove_stale_partials` and its two calls.

## Phase 3b, part 2 (the snap): answers

Recorded by slice 0 of `docs/briefs/phase-3b-snap.md`, from the brief's own text.

1. **The image sandbox in the snap: A** (maintainer, 10 Nov). Inside Rappel's own snap only, the image loader runs without bubblewrap; the snap's strict confinement is the boundary. Sizes, timeout and the SVG refusal stay. Outside the snap nothing changes (ADR 0055). Option B (a snap mode in glycin with seccomp and a memory limit) is for later.
2. **The `dbus` slot for `io.github.rappelapp.Rappel`: allowed.**
3. **Where the snap keeps the collection: per revision**, with a README note on reverts and disk use.
4. **The snap's grade: `stable`.**
5. **The backup's `.partial` clean-up stays.**
