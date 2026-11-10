# Phase 3b: final report

This report covers batch mode's run through `docs/briefs/phase-3b.md` (9–10 November 2026), and it is written for the maintainer.

## Summary
- **Shipped in `phase-3b`:** the stray white dots fixed (#120), the app id renamed to `io.github.rappelapp.Rappel` (#130), new card lists with drawn maths (#129, #128), the deck page grouped by tag (design E, added by you mid-batch), Statistics a month at a time, automatic backups to a folder you choose (#127), S to start studying and 3 and 4 to pass (your request for slice 6, as no `bug` or `polish` issue was open), the store listing for 0.4.0, hardening round 5, and the version bump.
- **The snap did not ship.** Slice 7 hit a stop condition: glycin's image sandbox (bubblewrap) cannot run under strict confinement (ADR 0082). Nothing was packaged, there is no `snapcraft.yaml` and no snap workflow, and the image code is unchanged. Packaging waits on your answer to QUESTIONS.md, Phase 3b, question 1. So 0.4.0 is a source release, with no store package.
- **Version:** **0.4.0** in `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` (dated 2026-11-10, with a sentence saying there is no snap) and the metainfo. It is **not tagged**.
- **Branches:** every slice was squash-merged into `phase-3b` after the local gate, the reviewer and a green CI `check`. Nothing was merged into, pushed to or aimed at `main`. The draft PR `phase-3b` → `main` is opened right after this report merges (see the PR list).

**How each slice was done:** I (the Sonnet orchestrator) gave a fresh sub-agent the slice's text; it implemented, ran `scripts/check.sh` under Xvfb, and the Opus `reviewer` agent reviewed it until nothing was blocking (often mutating the code in a scratch copy to show a test then fails). PRs were opened only after the local gate and the review, so most needed one CI run.

## What changed, per slice

| Slice | PR | ADR | Model | Commits | In short |
|---|---|---|---|---|---|
| 0. Housekeeping | #131 | — | Sonnet | 2 | Brief and mockups committed, CLAUDE.md, SPEC (3b ends in 0.4.0, 3c in 1.0), 3a answers recorded. |
| 1. Stray dots (#120) | #132 | 0076 | Opus | 1 | Each card side repaints a 2 px margin around its text. Also took the **SQLite security bump** (below). |
| 2. App id rename (#130) | #133 | 0075 | Sonnet | 1 | New id, GResource paths, URLs, "The Rappel Project" as developer. Data folder unchanged. |
| 3. Card lists (#129, #128) | #134 | 0077 | Opus | 2 | One shared row for the deck page and All Cards, shared tags once above the list, maths drawn in rows. |
| 3b. Deck page grouped by tag | #136 | 0079 | Opus | 5 | Flat rows with separators, groups by extra tags with headers, due word per row ("Today", "In 3 days"…). |
| 4. Statistics | #135 | 0078 | Opus | 2 | One card, Reviews / Time / Forecast, 30 bars, paging by 30 days, a Memory list. |
| 5. Automatic backups (#127) | #137 | 0080 | Opus | 3 | Preferences → Automatic Backups; at close, one export a day if something changed; 14 newest kept. |
| 6. Study keys | #138 | 0081 | Sonnet → **Opus** | 4 | S starts studying (deck list and deck page), 3 and 4 pass. **Escalated** (below). |
| 7. Snap | #139 | 0082 | Opus | 1 | Feasibility check only. **Stopped** at the image sandbox; four questions. |
| 8. Store listing | #140 | 0083 | Sonnet | 2 | Seven new store screenshots, URLs pinned to `v0.4.0`, 0.4.0 release notes. |
| 9. Clean-up #5, hardening 5 | #141 | 0084 | Opus | 2 | Rows 89–100 of the attack table, six real bugs fixed, carry-overs closed. |
| 10. Version 0.4.0 | #142, this PR | — | Sonnet (bump), Opus (this report) | 2 | 0.4.0 everywhere, not tagged. |

**Models and escalation.** As in the brief's model table: the orchestrator on Sonnet; every review by the Opus `reviewer`. Slice 6 started on Sonnet; after its **second** review round still had a blocking finding (a held S kept suspending cards), an Opus implementer took over the same branch and kept the verified work, as in 3a slice 7. Its third and fourth review rounds were on Opus work; the fourth had nothing blocking. "Commits" above counts the commits on each PR branch: the first is the implementation, and each later one answers one review round (or, on #136, a CI failure), so it is a rough count of rounds with findings; the last round of each PR had none.

## Highlights
- **Stray dots (#132).** Cause found and measured: GTK computes a text's damage box from unhinted glyphs but draws them hinted, so an accent can land one pixel outside what GTK repaints. A small box (`InkBox`) now makes GTK repaint 2 px more around each card side whenever it draws. A test checks that the margin contains every painted pixel. Reproduced and fixed only with the cairo renderer under Xvfb (GL and Vulkan there repaint the whole window); the cause is shared by all renderers, but **your GL/Vulkan desktop is unverified**.
- **Card lists (#134, #136).** Rows: the front, the back dimmed under it, a due word at the end, maths drawn ("σ", not `\sigma`). The tags every card shares are said once above the list. On a deck's page, cards are grouped by their other tags ("Only the shared tags", "Also Analyse MAT3101", with counts), unless grouping would not help (more than 6 groups, or more than half of them with one card: a pure, tested rule), in which case rows show their own pills with "+N". All Cards is never grouped. 10,000 cards open in about 0.3 s in a release build.
- **Statistics (#135).** Failed stacked on passed in the same accent at lower opacity, never by colour alone (the screen-reader text gives the count). Arrow keys select a day; paging stops at your first review.
- **Automatic backups (#137, #141).** The window closes at once; the backup finishes in the background (at most 5 minutes, then cancelled cleanly). It deletes only its own regular files named `Rappel backup YYYY-MM-DD.rappel` beyond the 14 newest, through a handle on the folder that never follows links (tested with links, look-alikes and a folder of 20,000 entries). A failure says why at the next start.
- **Study keys (#138).** S acts on its release, so holding S never starts a session and then suspends cards; in a session a held S suspends one card per press.
- **Hardening (#141), real bugs found:** a full disk was reported as "something unexpected"; killed backups' `.partial` files piled up (question 5); a backup folder inside the data folder was refused only when chosen; an overflow at extreme clocks; the first group header scrolled out after a bulk action; the chart ignored a CSS font size.

## Decisions and questions

### Questions for you (`docs/briefs/QUESTIONS.md`, Phase 3b)
| # | Question | Recommendation | You answer |
|---|---|---|---|
| 1 | **STOP: the image sandbox in the snap.** bubblewrap cannot create its sandbox under strict confinement (user namespaces refused; checked in two installed strict snaps). With today's code every image would show "could not be shown" in a snap. | **A now, B later:** inside Rappel's own snap only, run the image loader without bubblewrap; the snap's confinement (no network, no home folder) is the boundary, and sizes, timeout and the SVG refusal stay. Canonical's GNOME platform makes the same choice. Then propose a snap mode with seccomp and a memory limit to glycin (B). | A, B, C (Flatpak first, snap to 3c) or D (snap without images, not recommended). Residual risk of A: a decoder exploit in a hostile imported image could reach Rappel's own data and backups. |
| 2 | **A `dbus` slot** for `io.github.rappelapp.Rappel`, outside the brief's plug list. Without it the app cannot own its name and so loses single-instance behaviour, which backups and restore rely on. | Allow it (a slot grants no access; GNOME snaps declare it). Read from snapd's policy, not run. | Yes / no. |
| 3 | **Where the snap keeps the collection.** By default per revision (`~/snap/rappel/<revision>/…`): a revert restores pre-migration data, but hides work done since and copies the media per kept revision. Alternative: one shared folder, but a revert then meets a too-new database. | Per revision (the default), with the README explaining reverts and disk use. | Per revision / shared. |
| 4 | **The snap's grade.** | `grade: stable` (the beta channel says "beta"; stable later is just a channel change). | Stable / devel. |
| 5 | **The backup removes its own leftover `.partial` files**, which extends the letter of the pre-approved deletion rule. Only hidden regular files of its exact name (with 64 random bits), untouched for 24 h, after a successful backup. | Keep it (it is in #141). The alternative is to remove `remove_stale_partials` and document hand deletion. | Keep / remove. |

### Notable batch decisions (full lists in each PR's **Decisions**)
- **SQLite security bump in slice 1 (#132):** `rusqlite` 0.40 → 0.41, bundled SQLite 3.54.1, for RUSTSEC-2026-0459 (stale memory with a crafted `-wal` file), because `cargo deny` refused the old one. Same MIT licence, no migration.
- **New dependency `rustix` (#137)** in `rappel-core`, `fs` feature, already in the tree through glycin; licences allowed. Std has no `openat`/`unlinkat`, and `unsafe` is forbidden. The only new direct dependency.
- **Shared tags in a fixed header above the list (#134)**, not a list section header, which the first row's focus scrolled out of sight.
- **Due words count days as 24 h (#136):** around a daylight-saving change one word can be a day off (in ADR 0079).
- **Forecast starts tomorrow (#135):** today's and overdue cards are no longer in the chart, and the forecast does not page. The Memory list always shows the current 30 days. The mockup's "Average" figures were left out.
- **Backups (#137):** "changed since the last backup" is a fingerprint of what an export would hold (SHA-256 of `cards.json` without its time, plus the media records), so no migration; closing never waits; one backup per local calendar day; a backup over 2 GiB fails rather than silently leaving out media; the setting lives in `auto-backup.ini` beside `window-state.ini`.
- **S starts on release (#138)**, and only with no modifier and no text field focused; 3 and 4 are bound like 2 (same settle delay; AZERTY as GTK maps it). Under a non-Latin layout S does not start a session (known limit).
- **Store listing (#140):** light screenshots only (AppStream has no dark variant); release notes were held in a comment until the version bump, because the gate requires the metainfo to match the CHANGELOG.
- **No lazy formula layout (#141):** 0.3 s for 10,000 maths cards in release was judged acceptable.

## Process notes
- **Usage-limit pause, 9–10 November:** the run stopped on a usage limit during slices 9–10 and resumed on the same branches; nothing was lost.
- **CI failed three times on #136,** each time the same test, `the_first_groups_header_stays_in_sight`, which waited for a number of frames (5 runs, 3 red). The fix was redesigned to be timing-free: it waits for the window's next layout, not frames, and slice 9 used the same approach for the bulk-action case.
- **The two flaky export-size tests are now exact (#141):** the export holds its own time, which changed the compressed size by a few bytes; tests now pass the time in.
- **The pre-commit hook needs a display.** From a Wayland session, commit with `env -u WAYLAND_DISPLAY -u DISPLAY xvfb-run -a git commit`. The ink-box test now also passes in a Wayland session (it waits for sizes).
- **The guard hook blocks compound Bash commands that contain merge or push;** run `gh pr merge` alone.
- **A docs-only PR's skipped `check` counts as passing.** #139 (docs only) showed `check` "skipping"; `gh pr checks --watch` returned success, the merge state was clean and the ruleset let it merge. This answers the last open point of Phase 3a question 7. (#131 was not docs-only, as it touched `CHANGELOG.md`, so it ran the gate.)
- **No `dbus-run-session` on this machine,** and a stale Rappel process could hold the app's D-Bus name, so a later app check would open in that process instead. App checks used a private D-Bus address instead. If Rappel seems not to start after a test, look for a leftover `rappel` process (`pgrep -a rappel`).

## Things not verified
The implementers never saw a window: every screen was checked only through Xvfb screenshots. Unverified: GL and Vulkan renderers (#120), HiDPI, touch and touchpad scrolling, the real accent colour, Orca, French wording (to proofread in `po/fr.po`), and the backup's hold and 5-minute cap on a real close. **The snap was never built:** the portal folder's persistence, the `dbus` slot and M4A playback in a snap are read from sources only.

## Screenshots
Local and git-ignored, in `/home/architect/dev/rappel/docs/briefs/screenshots/`, never committed. Slices 0, 2, 7 and 10 have none.
- **Slice 1, dots:** before/after `f120-{light-1000,light-360,dark-360}-{1,2}{f,b}.png`; evidence runs (cards 1–6, front `f` and back `b`): `w120-{rev,full,diff,ngl}-*`, `x120-{cairo,cairofull,cairo-unfixed2,cairo-fix,cairo-fixfull,gl,glfull,vulkan,vulkanfull}-*`, `y120-{unfixed,unfixedfull,fix,fixfull}-*`, `w120-deck.png`.
- **Slice 3, card lists:** `p3b-s3-{light,dark}-{1000,360x640}-{deck,allcards-deck}.png`, `p3b-s3-fr-360x640-{deck,allcards-deck}.png`, `p3b-s3-allcards-light-1000.png`, 10,000 cards `p3b-s3-10k-light-1000-{deck,allcards}-0.5s.png`.
- **Slice 3b, grouped deck page:** `s3b-fronts-{light,dark,dark-keys,360-fr,360-fr-end}.png`, `s3b-backs-{light,360-dark}.png`, `s3b-tagsets-{light,360-fr-dark}.png`, `s3b-allcards-{light,360-fr-dark}.png`.
- **Slice 4, statistics:** `p3b-s4-{light,dark}-{1000,360x640}-{reviews,time,forecast}.png`, `p3b-s4-{light-1000,light-360x640,dark-1000}-{selected,previous}.png`, `p3b-s4-light-1000-previous{2,3}.png`, `p3b-s4-fr-360x640-{reviews,time,forecast,selected,previous}.png`.
- **Slice 5, backups:** `s5-prefs-{light,dark}-{1000,360}.png`, `s5-prefs-fr-360.png`, `s5-prefs-off-light-360.png`, `s5-failtoast-{light-1000,dark-1000,light-360,fr-360}.png` (and `-x` with Details), `s5-v2-prefs-{light,fr}-360.png`, `s5-v2-failtoast-light-1000.png`; probes `s5-prefs-probe-*.png`, `s5-e2e-before.png`.
- **Slice 6, study keys:** `study-keys-review.png`, `study-keys-shortcuts.png`.
- **Slice 8, store work shots:** `st-{review,decks,deck,deck3,allcards,statistics,editor,ed0,phone,menu,q}.png`.
- **Slice 9:** "Folder not allowed" toast `s9-toast-{light,dark,fr,fr-details,360,360-details}.png`.
- `probe-0.png` is a layout probe.
- **Committed store screenshots** (in the metainfo, light, 1000 px unless noted): `data/screenshots/{review,decks,deck,all-cards,statistics,editor,phone}.png` (`phone.png` is 360×640).

## Checklist for the native build
Scratch data, in the repository (never your real data folder):
```sh
D=$PWD/target/tmp/p3b-test
cargo run -p rappel-core --example fill_scratch_db -- "$D" 30 --due 5 --tags --suspended 3 --demo --probability --groups --history 75 --math
RAPPEL_DATA_DIR="$D" cargo run -p rappel
```
French: once `mkdir -p target/tmp/locales && localedef -i fr_FR -f UTF-8 target/tmp/locales/fr_FR.UTF-8`, then prefix `LOCPATH=$PWD/target/tmp/locales LC_ALL=fr_FR.UTF-8 LANGUAGE=fr`. Each PR's **Try it** has more detail.

1. **Look at the screenshots** above and tell me what looks wrong.
2. **Dots (#120):** add "Tribu engendrée" / "Mesure $\sigma$-finie" and "Ǻǻ Ŝŷ ĝĥ *σ-algèbre* ÉÂÎ" / "intégrale"; study and move between them, reveal, edit in the preview: no specks, no cut accents. On your normal (GL/Vulkan) session, and once with `GSK_RENDERER=cairo`.
3. **Rename (#130):** About shows "The Rappel Project", links to github.com/rappelapp/rappel; your real data still opens in an installed build (folder unchanged).
4. **Card lists and grouping:** "SIC 3101 – Fronts": "On every card:", "Only the shared tags · N cards", "Also Analyse MAT3101 · N cards", no pills on rows, due words ("Today" in accent). "SIC 3101 – Probabilités": backs dimmed, formulas drawn. "Tag sets": no groups, pills with "+N". Select Cards keeps headers; after a bulk Suspend the first header stays in view. All Cards (Ctrl+F): flat rows, deck name on the second line, never grouped; filtered by deck, the shared-tags line appears.
5. **Statistics** (F10 → Statistics): three tabs (Alt+R/T/F), rounded bars, failed lighter over a 2 px gap, grid values at right, Previous stops at your first review, Next disabled on the current period, Forecast from tomorrow, Memory list. Large Text: the chart grows.
6. **Backups:** Preferences (Ctrl+,) → Automatic Backups "Off" → Choose… a folder outside the data folder. Edit a card, close: the window closes at once, `Rappel backup <today>.rappel` (mode 0600) appears, no `.partial`. Close again unchanged: no new file. 19 dummy backups plus `notes.txt` and a link named like a backup: only the 14 newest dated files remain, the others untouched. Remove the folder, edit, close: the next start says "Automatic backup failed" with Details. A folder inside the data folder: "Folder not allowed".
7. **Study keys:** deck list, focus a row, S: that deck's session ("Back to Decks"). Deck page: S starts; nothing in selection mode or with nothing due. In a session 3, 4 and keypad pass, 3 before reveal does nothing; hold S: one card suspended. In the editor or search, s, 3, 4 only type. Tooltip "Study (S)"; Ctrl+? lists Start Studying and Pass 2 3 4.
8. **Store listing:** `appstreamcli validate --no-net --strict data/io.github.rappelapp.Rappel.metainfo.xml.in` passes; the seven screenshots in `data/screenshots/` match the app; read the 0.4.0 release notes in the metainfo.
9. **Light and dark** on every screen above.
10. **360×640** (and 640×360): deck page, All Cards, Statistics (bars about 7 px, legend under the figures), Preferences, the backup toast: nothing cut, with minimise and maximise shown (`gsettings set org.gnome.desktop.wm.preferences button-layout 'appmenu:minimize,maximize,close'`, put yours back after).
11. **French** at 360×640: "Sur toutes les cartes :", "Seulement les étiquettes communes", due words with plurals, "Dossier refusé", "Échec de la sauvegarde", the Statistics labels. Proofread with `po/fr.po`.
12. **Keyboard only:** every step without the mouse: rows and their ⋮, the first header visible on open and after Up to the first row, Left/Right/Home/End on the chart, Tab to Previous/Next, Choose… and the ✕ (focus goes back to Choose…), the toasts' Details.
13. **Orca:** row labels (tags, due state), group headers read once, "On every card:" and its "+N" pill; the chart summary and the selected day with the failed count; Previous/Next; "Turn Off Automatic Backups" and the folder row's path. #48 (break characters) still waits for your answer from 3a.
14. **Touch, if you have it:** a tap on the chart selects the nearest day; a vertical swipe scrolls without selecting.

## The snap: none to install yet
There is no snap to install for 0.4.0. When packaging resumes (after question 1), these are the checks ADR 0082 leaves for a real snap:
1. `sudo snap install --dangerous ./rappel_<version>_amd64.snap`; Rappel starts from the app grid with its icon and name.
2. Images show in a card (question 1's choice in effect) and the log says which sandbox was used; an SVG is still refused.
3. MP3, Ogg and WAV play; M4A plays only if `faad` was staged.
4. A second launch opens a window in the same process (the `dbus` slot).
5. French appears with `LANGUAGE=fr` (`RAPPEL_LOCALEDIR` set).
6. Export… and Import… go through the portal; choose a backup folder, close, log out and back in, edit, close: the backup is written to the same folder (portal persistence; ADR 0080's path survives a restart).
7. No access to `~` outside the portal (no `home` plug): `snap connections rappel` lists only the desktop set, `audio-playback` and the `dbus` slot.
8. The data is in `~/snap/rappel/current/.local/share/rappel`; after a `snap refresh` the collection is there; read the README's revert note (question 3).
9. Light, dark, 360 px and Orca as above, once in the snap.

**Copying your data into a snap**, once it exists (the snap cannot read `~/.local/share/rappel`):
```sh
# 1. Quit Rappel (native). 2. Start the snap once and close it, so the folder exists.
snap run rappel   # then close the window
# 3. With both closed:
mkdir -p ~/snap/rappel/current/.local/share
cp -a ~/.local/share/rappel ~/snap/rappel/current/.local/share/
```
`current` points at the installed revision. With the per-revision default (question 3), snapd copies this folder at each refresh; after a `snap revert`, work done since the refresh stays only in the newer revision's folder until snapd drops it, so move it back with Export / Import. Export… in the native app and Import… in the snap is the alternative to `cp -a`.

**Old desktop file and icons after the rename** (only if you installed them by hand under the old id):
```sh
rm -f ~/.local/share/applications/io.github.rsmirke.Rappel.desktop
rm -f ~/.local/share/icons/hicolor/scalable/apps/io.github.rsmirke.Rappel.svg
rm -f ~/.local/share/icons/hicolor/symbolic/apps/io.github.rsmirke.Rappel-symbolic.svg
rm -f ~/.local/share/metainfo/io.github.rsmirke.Rappel.metainfo.xml
```

## After the batch
1. **Answer QUESTIONS.md, Phase 3b**, questions 1–5 (question 5 decides whether #141's `.partial` cleanup stays in 0.4.0).
2. **Merge the draft PR `phase-3b` → `main`** (squash; merge commits are refused), after the checklist above:
   ```sh
   gh pr merge <draft PR number> --squash
   ```
3. **Tag `v0.4.0` on that commit and create the release:**
   ```sh
   git checkout main && git pull --ff-only
   git log -1 --oneline        # the phase 3b squash commit
   git tag -a v0.4.0 -m "Rappel 0.4.0"
   git push origin v0.4.0
   sed -n '/^## \[0.4.0\]/,/^## \[0.4.0-beta.1\]/p' CHANGELOG.md | sed '1d;$d' > /tmp/rappel-0.4.0-notes.md
   gh release create v0.4.0 --title "Rappel 0.4.0" --notes-file /tmp/rappel-0.4.0-notes.md
   ```
   The metainfo's screenshot URLs and the CHANGELOG's compare link point at `v0.4.0`, so they give 404 until the tag is pushed.
4. **Decide the snap path** (question 1: A/B, C or D). With A or B, a later slice packages it with the plan already in ADR 0082 (core26, `gnome` extension, `scripts/snap-install.sh`, a tag-triggered workflow that only builds). Only then: install from CI, run the snap checklist, and `snapcraft upload --release=beta rappel_<version>_amd64.snap` (0.4.x goes to beta).
5. Close #120, #127, #128, #129 and #130 after the merge (still open now); #48 closes after your Orca answer.
