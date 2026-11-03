# Phase 3a: final report

This report covers batch mode's run through `docs/briefs/phase-3a.md`, and it is written for the maintainer.

**How each slice was done:**
- I planned it here, and a fresh sub-agent implemented it.
- **Models.** The brief's model line says Opus for planning, implementation and review. At the start of the run you overrode it in chat: Sonnet for slices 0, 1, 7 and 9, Opus for slices 2–6 and 8, and the reviewer always on Opus; a Sonnet slice that needed more than two review rounds was to be redone on Opus. Slices 0 and 1 ran on Sonnet. Slice 7 ran on Sonnet, needed a third round and was redone on Opus. After slice 7 you withdrew the chat instruction ("no more Sonnet, everything on Opus") for the rest of the batch, so slice 7b and slices 8 and 9 ran on Opus.
- It was checked with `scripts/check.sh` under Xvfb.
- The `reviewer` agent (Opus) reviewed it until it had no blocking findings. Usually the reviewer also mutated the code: it removed a fix or a guard in a scratch copy and confirmed that a test then fails.
- It then went through a PR into `phase-3a`, waited for a green CI `check`, and was squash-merged.
- From mid-run your CI-budget rule applied (#111): a PR was opened only after the local gate and the review had passed, and fixes went up in one push.
- Scratch builds: the reviewers' mutation builds once filled `/tmp` (a RAM-backed tmpfs); after that they went under the git-ignored `target/`. You cleaned up both `/tmp` and the `target/scratch-*` folders.
- Nothing was merged into, pushed to or aimed at `main`, except the draft PR `phase-3a` → `main` that follows this report.

**Version:** **0.4.0-beta.1** in `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` and the AppStream metainfo (`0.4.0~beta.1`). It is **not tagged**.

## What changed, per slice

| Slice | PR | ADR | In short |
|---|---|---|---|
| — | #107, #108 | — | #107 your brief; #108 `pulldown-cmark` 0.13.6 for two advisories that blocked the gate (Rappel never enabled the affected wikilinks). |
| 0. Housekeeping | #109 | — | SPEC roadmap (3a ends in 0.4.0, 3b in 1.0), CLAUDE.md's batch names, Phase 2 question 1 answered. |
| — | #111, #112 | 0003 (amended) | CI skips the gate for docs-only PRs (yours), then made fail-safe (two ways it could skip the gate on code). |
| 1. Small polish | #110 | 0064 | Key hints in tooltips ("Pass (2)"), Ctrl+A on All Cards, Move to Deck… per card and during review, a table's border hugs its columns (redo of #32). |
| 2. Selection on the deck page | #113 | 0065 | One shared selection mode (`ui::selection`) for All Cards and the deck page, with the same action bar and Undo. |
| 3. Merge decks | #114 | 0066, 0067 | Merge Into… with one Undo, a property test that merge then Undo is the identity; the **import now keeps a card's own deck**. |
| 4. Archive a deck | #115 | 0068 | Migration 0012, `archived_at` in exports, Archived Decks page, All Cards' archived filter; **the test-harness SIGSEGV found and fixed**. |
| 5. Study by tag | #116 | 0069 | A normal session over a tag's cards in every unarchived deck, each deck's daily limits shared. |
| 6. Practice | #118 | 0070 | Practise a deck or a tag; nothing is written, proved byte for byte. |
| 7. Phase 2 leftovers, and #117 | #121 | 0071 | Nothing cut at 360 px in French, Add Tags keeps its text, an import check with Cancel, exports over 2 GiB of media asked first; the editor's key hint removed (#117). **Redone on Opus.** |
| 7b. Same-front note (#119) | #122 | 0072 | The card editor says when another card has the same front, with Show Card. |
| 8. Clean-up and hardening 4 | #123 | 0073 | 15 attacks on the new paths (rows 74–88), four fixes, among them a deck page freeze at 10,000 cards (234.6 s of CPU → 0.17 s). |
| 9. Beta preparation | this PR | — | 0.4.0-beta.1, QUESTIONS.md, this report. The draft PR `phase-3a` → `main` follows it. |

Slice 7b and #117 are not in the brief: you added them during the run (QUESTIONS.md, Phase 3a, note 5). #117 is done by #121 and #119 by #122. After the merge, these issues can be closed: #102, #103, #104, #105, #106, #117 and #119; #48 closes after your Orca answer (checklist 18). PR #32 was already closed on 2026-11-02 (slice 1 redid it). #120 (white dots next to inline math) is planned after this phase.

Each PR's description has the full **What**, **Decisions**, **Reviewer** and **Try it** sections. Below are the parts you most need.

### Highlights
- **Organising.**
  - The deck page has the same selection mode as All Cards: Select Cards, Ctrl+A or a long press, then Move, Tags, Suspend, Reset or Delete, each with Undo. Both pages use one component, so they cannot drift.
  - **Move to Deck…** for one card, from its menu or from the review page's Card Menu (the card leaves the session as with Suspend).
  - **Merge Into…** moves every card of a deck, with its history, into another and removes the emptied deck. One Undo restores both exactly; a property test checks the whole database is identical after merge then Undo.
  - **Archive** takes a deck out of the deck list, the day's study and the forecast, and keeps its cards, history and exports. Archived Decks lists them with Unarchive.
- **Studying.**
  - **Study by tag**: a normal, recorded session over the due and new cards that carry a tag, in every unarchived deck. Each deck gives at most what is left of its own daily limits, so studying by tag and by deck share them.
  - **Practice**: goes over a deck's or a tag's cards, weakest first, at most 50, with the same Fail and Pass. Nothing is written. A test compares the database files byte for byte, the logical dump and SQLite's change counters before and after; the practice reads run with `PRAGMA query_only`, so a write there would fail.
- **The card editor** notes "A card in “Spanish” has the same front", ignoring case, accents, spacing and Markdown, without ever blocking Save.
- **Imports and exports.**
  - An export made before a Move to Deck or a merge imports again (ADR 0067; question 1).
  - An export whose media would make it too large to import again asks "Export Without Media?" first, and only when that is certain (a lower bound).
  - Checking a `.rappel` file can be cancelled, and an oversized `cards.json` is refused at once.
- **Phone and French:** All Cards' filters wrap instead of being cut, and the deck page and review headers fit at 360 px with minimise and maximise shown.
- **Hardening:** ADR 0073 lists 15 more attacks (rows 74–88), each with its test: odd `archived_at` values, a merge of 20,000 cards, a practice killed with SIGKILL, Ctrl+A on 10,000 cards, a tag across 100 decks, and more.

### Bugs found and fixed along the way
- **The test harness crashed, and drew on your desktop** (#115). The app's test binary sometimes died with SIGSEGV or SIGABRT; it had happened in phase 2 (#93, #96) and in slice 3. Cause: `xvfb-run` keeps `WAYLAND_DISPLAY`, and GTK prefers Wayland, so the GTK tests ran on your real Wayland session with IBus, not on Xvfb. Their windows appeared on your screen, and late input-method events reached windows a test had already destroyed. The harness now uses GTK's own input method and X11 whenever `DISPLAY` is set (ADR 0024 amended). The app is unchanged.
- **The docs-only CI check could skip the gate on code** (#112, after #111 had merged before its review finished). Under `pipefail`, `grep -q` stopped reading early and its writer died of SIGPIPE, which the check read as "no code file": from about 400 changed files the gate was sometimes skipped, from 2,000 always. A source file renamed into `docs/` also looked docs-only. Both fixed, and a real `grep` error now runs the gate.
- **A card looked unticked but was still chosen** (#113, blocking in review). After Ctrl+A, the Undo of a card menu's Suspend replaced the row, which showed unticked while its card stayed chosen, so a bulk Delete would have deleted a card you could not see was selected. The shared controller now follows the list's changes itself.
- **A tag name went unescaped into a status page** (#116, blocking). "No cards tagged … are left to study now." is Pango markup; a tag such as `a&b <i>` broke it. It is escaped now, with a test.
- **The import's test model still had the old rule** (#114, blocking). The independent model of the import (`oracle.rs`) and the random histories did not move cards or merge decks, so the new rule was untested by them. Both now do, and a new property test imports an old export after random reorganisations.
- **An export could become impossible to retry, or wrongly refused** (#121). Round 1: an export whose media plus text went past 2 GiB was refused every time, with no way out. Round 2's fix overestimated `cards.json` 5 to 60 times, so a user with 1.5–2 GiB of media would only have been offered "Export Without Media" for an export that imports fine. The Opus redo made the pre-check a true lower bound, and leaves the rest to a check after writing that offers the retry without media.
- **French counts were cut on a phone** (#121, blocking in the Opus review): "5 à revoir · 20 nouvelles" beside the Study pill ran off the window at 360 px. They now sit under it.
- **The same-front property test never matched** (#122, blocking): 0 of 65 cases generated a duplicate, so it could not catch a missed one. It now builds near-copies from a tiny alphabet against an independent key, with at least half the cases matching, and mutations make it fail.
- **The deck page froze for minutes at 10,000 cards** (#123): after a reset of many cards, each changed new-card flag walked the whole list. With a card chosen: 234.6 s of CPU, now 0.17 s. With nothing chosen, the new test's 1,000 row changes took 8.6 s, now 19 ms.
- **Smaller ones** (#123): practice's `query_only` was turned off at the end of a nested read; the same-front lookup copied every whole front into SQLite's sorter; a toast title was cut inside a deck name with a colon ("“Spanish"); All Cards could not find a word starting with a private-use character (#122 found it in the same-front lookup).
- **Flaky tests:** `deck_page::…after_a_bulk_removal_the_focus_follows_the_list` waited a fixed 100 ms (fixed in #115). The drop-down test `ui::dropdown::tests::the_factories_survive_new_labels_and_the_popup_stays_narrow` failed now and then (#108, #113). Its root cause was not found; its recorded failures all predate the harness fix, it passed 40 of 40 runs under load since, and the harness now prints the failing assertion (#123).

## Open questions (`docs/briefs/QUESTIONS.md`, Phase 3a)
None was a stop condition: the brief pre-approved migration 0012, the export field and the merged deck's soft deletion. Each is a decision you may want to reverse.
1. **The import keeps a card's own deck** (ADR 0067). A reorganisation on machine A is not carried to machine B by import. Keep it?
2. **0.3.0-beta.1 refuses every export from this version** (`archived_at` is always written). Acceptable for a beta?
3. **Suspend, Move and Delete stay available during practice.** Keep, or hide them?
4. **Slice 7's redo on Opus** took over the branch and kept the verified work, rather than rewriting. Is that how a redo should work?
5. **The brief does not list slice 7b (#119) or #117.** Amend it for the record?
6. **Under 400 px, selection mode's title is just the count** ("30"). Keep, or find shorter words?
7. **Two points of CLAUDE.md's CI budget** from #112: the "no docs after CI has passed" wording, and `.claude/` counted as docs.

## Things to know
- **French needs you**, as in phase 2: every new string has a French translation of mine ("S’entraîner", "Entraînement", "Cartes travaillées", "replanifié", "Fusionner dans…", "Désarchiver", "Afficher la carte"…), to proofread with the rest of `po/fr.po`.
- **The store screenshots in `data/screenshots/` are old.** `review.png` still shows the Space / 1 / 2 hints and `editor.png` the "Ctrl ⏎ to save" hint. Retake them before a release (`fill_scratch_db --demo`, on GNOME for the window shadows).
- **The ruleset:** `phase-3a` had no ruleset at first (the "phase-1" ruleset targeted `refs/heads/target-*`). You fixed it on 2026-11-03 ("phase branches", `refs/heads/phase-*`, requires `check`); I checked it with `gh api`. Batch mode had waited for a green `check` before every merge anyway.
- **Lockfile bumps:** `pulldown-cmark` 0.13.6 (#108), and `async-executor` 1.14.0 → 1.14.1 (#115), a glycin/zbus dependency yanked between the local gate and CI.
- **Disk:**
  - The reviewers' mutation builds once filled `/tmp`, which is a RAM-backed tmpfs (about 5.8 GB); you deleted them. Since then every scratch build goes under the git-ignored `target/`, and you have deleted the `target/scratch-*` folders too.
- **Known limits from the PRs:**
  - **Files moved between machines:** a Move or a merge on machine A is not carried to machine B by import, and a merged deck comes back empty when an older backup is imported (question 1).
  - **0.3.0-beta.1 cannot import a file from this version** (question 2).
  - **Export over 2 GiB:** an export whose media are just under 2 GiB is written in full first, then found too large because of its text, and only then asked about. That is the price of never refusing an export that would import (#121).
  - Escape cancels an import check only when nothing else uses it: not with a dialog open, nor in a focused search field (#121).
  - The Anki early size check covers only the legacy containers (ADR 0071).
  - Undoing a merge reloads the deck list, deck pages and All Cards, but not an open review or Statistics page of the target deck; answers there still go to the right cards (#114).
  - Undo of "Deck unarchived" during a session of that deck ends the session and leaves an open card editor behind; every grade is already saved (#115).
  - An archived deck cannot be renamed from the UI (it has no row or page), though core allows it (#115).
  - At 640×360 with every deck archived, a faint scroll edge shows above the empty state (#115).
  - The same-front note never shows for a front made only of punctuation or symbols ("?", "+", or `ⓐ`), nor for a front over 64 KiB. Show Card opens only the first match. A card added elsewhere while the editor stays open on an unchanged front is not noticed (#122).
  - The same-front lookup takes 0.5–0.9 s (debug) against a crafted import of 256 MiB of repeated fronts; accepted (ADR 0073, row 84).
  - The deck name in the deck page's header is still shortened at 360 px with minimise and maximise shown ("Scratch (30 …"): it cannot fit beside three window buttons and four header buttons (#121).
  - The review header's Card Menu lists Edit Card at every width, a duplicate of the Edit button on wide windows (#121).
  - All Cards' Study button is an icon (play) only; a word does not fit at 360 px (#116).
  - There is no Practice entry on All Cards: a tag's practice starts from its session's summary (#118).
  - Move to Deck…'s chooser opens with the drop-down focused, so Enter opens the list and Tab reaches Move, as All Cards' Move always did (#110).
  - Move to Deck… on All Cards' rows was not asked for; it comes with the shared row (#110).
  - Practice's queue is sorted in Rust over the whole scope: 14 ms at 10,000 cards in debug (#118).
  - With a `DISPLAY` that names a dead X server, the GTK tests now fail rather than fall back to Wayland; the message says to use `xvfb-run` (#115).
- **Unchecked here:** everything Orca says (the checklist below), a real on-screen keyboard (Phosh), hovering for tooltips (the harness cannot hover), the "Checking the file…" toast (the file chooser is a portal, out of the harness's reach), and popovers at 640×360 on a real session (the card menu reaches the window's bottom edge under Xvfb, #116).

## New dependencies in this phase
None. Every licence is as before.
- Lockfile only: `pulldown-cmark` 0.13.4 → 0.13.6 (#108) and `async-executor` 1.14.0 → 1.14.1 (#115).
- `rusqlite` was already a dev-dependency of the app; #123 also uses it to write 10,000 test cards in one transaction.
- `serde_json`, already a dependency of `rappel-core`, binds the merge Undo's ids through SQLite's `json_each` (#114).

## Screenshots
All are local, in `docs/briefs/screenshots/` (git-ignored); each PR lists its own. Slices 0, 8 and 9 have none (slice 8 changed only an error toast's text).
- **Slice 1, small polish** (`p3a-s1-*`):
  - review page without hints: `p3a-s1-review-{1000,360,640x360}-{light,dark}-{front,back}.png`, French `p3a-s1-review-360-fr-{front,back,menu,chooser,toast}.png`;
  - Move to Deck…: `p3a-s1-review-1000-light-{menu,chooser,moved}.png`, `p3a-s1-deck-1000-light-{menu,chooser,toast,undone}.png`, `p3a-s1-deck-1000-dark-{menu,chooser,toast}.png`, `p3a-s1-deck-360-light-{menu,chooser,toast}.png`, `p3a-s1-deck-640x360-light-menu.png`, `p3a-s1-deck-360-fr-{menu,chooser,toast}.png`;
  - Ctrl+A: `p3a-s1-allcards-1000-light-ctrl-a-in-search.png`, `p3a-s1-allcards-1000-light-selected.png`;
  - tables: `p3a-s1-table-{narrow,wide}-1000-front.png`, `p3a-s1-table-wide-360-front.png`, `p3a-s1-table-360-{c7,c7back,c8,c8back}.png`.
- **Slice 2, selection on the deck page** (`p3a-s2-*`): `p3a-s2-deck-{light,dark}-{1000,360}-{normal,selecting}.png`, `p3a-s2-deck-light-640x360-{normal,selecting}.png`, `p3a-s2-deck-fr-360-{normal,selecting}.png`, `p3a-s2-deck-light-360-close-normal.png` (close-only buttons), All Cards after the refactor `p3a-s2-allcards-light-1000-{normal,selecting}.png`, `p3a-s2-allcards-dark-360-{normal,selecting}.png`.
- **Slice 3, merge** (`p3a-s3-*`): menu, dialog and toast `p3a-s3-{light,dark}-{1000,360}-{menu,dialog,toast}.png`, `p3a-s3-light-640x360-{menu,dialog,toast}.png`, `p3a-s3-fr-360-{menu,dialog}.png`; from the deck page `p3a-s3-deckpage-light-1000-{menu,dialog,after}.png`, `p3a-s3-deckpage-dark-360-{menu,dialog,popup}.png`. (`p3a-s3-probe*` are layout probes.)
- **Slice 4, archive** (`p3a-s4-*`): for each of `light`, `dark` at `1000`, `360` and `640x360`, and `fr-360`: `-list`, `-menu`, `-toast`, `-archived`, `-allarchived`, `-allcards`, `-archivedpopup`; also `-unarchived`, `-allcardsshown` (1000 and 360), `-onlyarchived` (360), `-deckpopup` (1000), `-archivedrow` (640x360), and `p3a-s4-light-360x640-list.png`.
- **Slice 5, study by tag** (`p3a-s5-*`): `p3a-s5-{1000,360x640,640x360}-{light,dark}-{deckmenu,review-from-deck,allcards,review,empty}.png`, French `p3a-s5-fr-360x640-light-{deckmenu,review-from-deck,allcards,empty}.png`, the Deck Menu's submenu `p3a-s5-{1000x700,360x640}-light-deck-menu.png`, and `p3a-s5-360x640-phone-allcards.png`.
- **Slice 6, practice** (`p3a-s6-*`): `p3a-s6-{1000,360x640,640x360}-{light,dark}-{deckmenu,summary,practice,answer,done}.png`, French `p3a-s6-fr-360x640-light-*.png`, the deck row's menu `p3a-s6-decklist-{list,rowmenu}.png`.
- **Slice 7, leftovers** (`p3a-s7-*`):
  - All Cards' filters, before and after: `p3a-s7-before-ac-{en,fr}-360-a.png`, `p3a-s7-before-ac2-en-360-ac.png`, `p3a-s7-after-ac-{en,fr}-360-a.png`, `p3a-s7-after-ac-fr-1000-a.png`, `p3a-s7-after-ac-dark-640x360-a.png`;
  - **the deck page, final** (counts under the Study pill; `mm` = minimise and maximise shown, `c` = close only): `p3a-s7-redo-deck-{en,fr}-{light,dark}-360-{mm,c}-deck.png`, `p3a-s7-redo-deck-{en,fr}-light-640x360-deck.png`, `p3a-s7-redo-deck-fr-dark-640x360-deck.png`;
  - headers before the Opus review (older than the final deck shots): `p3a-s7-before-deck-{en,fr}-360-{deck,study}.png`, `p3a-s7-after-deck-{en,fr}-360-deck.png`, `p3a-s7-r2-deck-fr-360-deck.png`, selection titles `p3a-s7-after-deck-fr-{360,1000}-selecting.png`, `p3a-s7-after-ac-fr-360-selecting.png`, review and practice `p3a-s7-hdr-fr-360-{practice,study,menu,dmenu,stats,archived}.png`, `p3a-s7-after-practice-fr-360-a.png`;
  - Add Tags: `p3a-s7-tags-before-{typed,small,menu,sel}.png`, `p3a-s7-tags-after-small.png`;
  - export question, final: `p3a-s7-redo-export-fr-360-{alert,menu}.png`, `p3a-s7-redo-export-dark-1000-alert.png`; before the redo (old body): `p3a-s7-export-{light,dark}-1000-alert.png`, `p3a-s7-export-light-360-{alert,menu}.png`, `p3a-s7-export-fr-360-{alert,menu}.png`, `p3a-s7-export-dark-640x360-alert.png`, `p3a-s7-export-en-1000-menu.png`;
  - #117, the editor: `p3a-s7-editor-before-{light,dark}-1000-a.png`, `p3a-s7-editor-before-{light-360-a,light-360-b,dark-360-a,fr-360-a}.png`, `p3a-s7-editor-after-{light,dark}-{1000,360}-a.png`, `p3a-s7-editor-after-fr-360-a.png`.
  - `p3a-s7-dbg-s.png` and `p3a-s7-tags-dbg-small.png` are debugging shots.
- **Slice 7b, same-front note** (`p3a-s7b-*`): `p3a-s7b-{1000,360x640,640x360}-{light,dark}-{one,several,archived,info,back}.png`, French `p3a-s7b-fr-360x640-light-{one,several,archived,info,back}.png`.

## Checklist for the test week
Scratch data, in the repository (never your real data folder):
```sh
D=$PWD/target/tmp/p3a-test
cargo run -p rappel-core --example fill_scratch_db -- "$D" 30 --due 5 --tags --suspended 3 --demo --media --archived 1
RAPPEL_DATA_DIR="$D" cargo run -p rappel
```
This gives the "Scratch (30 cards)" deck (5 due, tags, 3 suspended), six demo decks with a month of history (the tag "geography" on World Capitals and Flags), a Media deck, and one archived deck (Art History, the first in the list). For French, once: `mkdir -p target/tmp/locales && localedef -i fr_FR -f UTF-8 target/tmp/locales/fr_FR.UTF-8`, then add `LOCPATH=$PWD/target/tmp/locales LC_ALL=fr_FR.UTF-8 LANGUAGE=fr` before `cargo run` (README → Translations). For the narrow checks with minimise and maximise shown: `gsettings set org.gnome.desktop.wm.preferences button-layout 'appmenu:minimize,maximize,close'`, and put your own value back afterwards (`gsettings get …` first). Each PR's **Try it** has more detail.

1. **Look at the screenshots** above and tell me what looks wrong.
2. **Review page hints (#105, #117):** Study a deck. No "Space", "1", "2" under the buttons. Hover Show Answer, Fail, Pass, Undo and Edit: "Show Answer (Space)", "Fail (1)", "Pass (2)", and the Undo and Edit keys. Open the card editor (+ on a deck page): no "Ctrl ⏎ to save"; Save's tooltip is "Save (Ctrl+Enter)".
3. **Move to Deck… (#102):** on the deck page, a card's ⋮ → Move to Deck… (above Delete): the chooser never lists the card's own deck; the card leaves, "Card moved", Undo puts it back in its place. In review, Card Menu → Move to Deck…: the next card shows, and Undo brings the moved one back as the next card. With a single deck, the entry is greyed out.
4. **Ctrl+A (#106):** on All Cards, Ctrl+A from the list, a filter or the Select button chooses every listed card; in the search field it selects the text only.
5. **Tables:** in a second scratch folder,
   ```sh
   cargo run -p rappel-core --example fill_scratch_db -- $PWD/target/tmp/p3a-tables 30 --markdown
   RAPPEL_DATA_DIR=$PWD/target/tmp/p3a-tables cargo run -p rappel
   ```
   study the Markdown deck: the "A table:" card's border hugs its three columns.
6. **Selection on the deck page (#104):** open "Scratch (30 cards)", Select Cards (between Study and ⋮). Choose two rows: "2 Selected"; each bar action works and its Undo restores the list and the counts. Bug-fix check: Suspend one card from its ⋮ menu, Ctrl+A, then Undo in the "Card suspended" toast: every card stays ticked and the title still says "30 Selected". Escape leaves the mode, a second Escape goes back.
7. **Merge (#103):** on the deck list, French Vocabulary's ⋮ → Merge Into…: the body says how many cards move; Enter on the drop-down opens its list, never merges; archived decks are not offered. Merge into Chemical Elements: the row leaves, "Decks merged", Undo brings the deck back with its cards. From a deck page, Merge Into… goes back to the deck list focused on the target.
8. **The import keeps a card's deck (question 1):** Export… to a file. Move a card to another deck (step 3), then Import… the file: the dialog says "1 card is in another deck in the file and stays in the one it is in now." Import: the card stays in its new deck.
9. **Archive:** "Archived Decks" (with a 1) ends the deck list. Archive Calculus from its ⋮ menu: "Deck archived", Undo. On the Archived Decks page, Unarchive Calculus only (Art History stays archived, for step 12). On All Cards, the fourth drop-down "Archived Hidden" / "Archived Shown"; Escape or Clear Filters leaves it as it is. Statistics: the forecast leaves archived decks out, reviews per day still count them.
10. **Study by tag:** World Capitals → a card's ⋮ → "Study Cards Tagged “geography”": the title is "Tagged “geography”", with due cards from World Capitals and Flags. The Deck Menu has a "Study Cards Tagged" submenu. On All Cards, choose "geography" in the tag filter: a play button appears beside Select Cards. Archive Flags, study the tag again: no Flags card.
11. **Practice:** finish a session: "Practice" (grey) sits beside "Back to Deck", which has the focus. Practise: the banner "Practice — nothing is rescheduled"; a failed card comes back after three others; Ctrl+Z steps back. The summary says "Practice: nothing was rescheduled". Back goes to the deck page, and its counts are unchanged. Practice is also first in the Deck Menu and in each deck row's ⋮ menu (that one ends with "Back to Decks").
12. **Same front (#119):** French Vocabulary → Ctrl+N, type `THE  KEY`: after a pause, "A card in “French Vocabulary” has the same front" with Show Card; Save still works. Type `Who sculpted *The Thinker*?`: the note names the archived deck. Show Card opens Card Information; Escape returns with the text intact.
13. **Phase 2 leftovers:**
    - All Cards at 360 px, English and French: no filter ends in "…"; they wrap.
    - With minimise and maximise shown, 360 px: the deck page has Select Cards at the left of the bottom bar, a one-count subtitle and the full counts under the Study pill, whole in French.
    - Add Tags: type `zebra` (no comma) and make the window much shorter: `zebra` stays in the field, no pill.
    - Exports over 2 GiB: follow #121's Try it (a scratch copy with the media sizes raised): "Export Without Media?" with Cancel focused; with the real sizes, no question.
14. **Light and dark** on every screen above.
15. **Sizes 360×640 and 640×360** (drag the window, or a phone): the deck page, All Cards, the merge dialog, Archived Decks, a tag session, a practice and its summary, and the editor's same-front note. Nothing is cut, and Back to Deck and Practice both show on a session's summary.
16. **French** at 360×640: the new texts ("Fusionner dans…", "Archiver", "Désarchiver", "Étudier les cartes étiquetées « … »", "S’entraîner", "Une carte de « … » a le même recto") read well and are not cut. Proofread them with the rest of `po/fr.po`.
17. **Keyboard only:** every step above without the mouse. In particular: Tab to a row's ⋮, Space, arrows, Enter; Ctrl+A, then Tab to the action bar; the toasts' Undo and Cancel by Tab; Tab from the front field to Show Card; Tab from Back to Deck to Practice.
18. **Orca, #48: the inserted break characters and the long-card note.** Since #47, a run of more than 64 characters without a space gets an invisible zero-width space every 64 characters, and a side over 32 KiB shows only its start with a note under the card. Check that Orca never says these characters and reads the note once.
    1. Make two texts:
       ```sh
       python3 -c "print('Before ' + 'abcdefghij' * 20 + ' after')" > target/tmp/run.txt
       python3 -c "print('word ' * 7000)" > target/tmp/long.txt
       ```
    2. Start Orca: Super+Alt+S, or `orca &` in a terminal. (The Orca key is Insert on the Desktop layout, Caps Lock on the Laptop layout.)
    3. Start Rappel with the scratch data. On the deck list, New Deck… "Orca", open it, press Ctrl+N. Open `target/tmp/run.txt` in Text Editor, Ctrl+A, Ctrl+C, and paste it into the front with Ctrl+V; type `run` in the back; Ctrl+Shift+Enter (save and add another). Paste `long.txt` the same way into the next front, type `long` in the back, Ctrl+Enter.
    4. Press Study on the "Orca" deck: the session has only these two new cards.
    5. **The 200-letter run:** when the card shows, Orca reads the focused card. Listen: "Before", the run, "after". It must **not** say "zero width space", "space", or pause oddly every 64 letters. Press Where Am I (Keypad Enter on the Desktop layout, Orca+Enter on the Laptop layout) to hear it again. Press Space: the back "run" is read.
    6. **The long card:** when it shows, Orca starts reading "word word…"; press Ctrl to stop it. Under the card the note says "This card is long: only its start is shown. Edit it to see all of it." Use Where Am I, then Tab once and Shift+Tab back. The note must be heard **once** each time, not as part of the card's text and again as its own label. Press Space to reveal: the back reads "long" with no note.
    7. In the editor (E on the long card): on a narrow window, switch to Preview. Under the preview the editor's own note says "This card is long: only its start is previewed." It must be heard once.
    8. **Flat review across the run.** The screen-reader name of a card's label has the breaks removed, but the label's text (AT-SPI Text interface) still holds the U+200B characters (`crates/app/src/render/mod.rs`, `name_without_breaks`), and flat review reads that text. Back on the review page with the 200-letter card, enter flat review and move through it: on the Desktop layout, KP_7 / KP_8 / KP_9 (previous, current, next line) and KP_4 / KP_5 / KP_6 (previous, current, next word); on the Laptop layout, Orca+U / Orca+I / Orca+O for lines and Orca+J / Orca+K / Orca+L for words. Orca must never say "zero width space", and the run must not be split into odd pieces every 64 letters.
    9. Write down what Orca said in each case; #48 is closed by your answer.
19. **Orca, the other new items:**
    - **Selection title (#121):** with minimise and maximise shown and the window at 360 px, a deck page → Select Cards (bottom bar) → choose 30 cards with Ctrl+A. The title shows "30"; Orca must read "30 Selected", not "30". At 1000 px it reads "30 Selected" too. Each row reads "…, selected" or "…, not selected"; the bar buttons read "Move selected cards to a deck", "Delete selected cards" and so on.
    - **Same-front note (#122):** in the card editor, type `the key` and stop: Orca reads "A card in “French Vocabulary” has the same front" **once**, politely (after what it was saying, not interrupting), and not again at each key you type later. On the front field, Where Am I includes the note as its description. Tab from the front: Orca reads "Show Card" with the note. Enter opens Card Information; Escape comes back to Show Card.
    - **Practice banner (#118):** start a practice. Listen whether Orca announces "Practice — nothing is rescheduled" or only the card when the page opens; the page's name is "Practice". If the banner is only heard when you browse to it (Orca's flat review: the Orca key + the keypad, or Tab), tell me: it may need an announcement.
    - **Import Cancel toast (#121):** make a large export to import: `cargo run -p rappel-core --example fill_scratch_db -- $PWD/target/tmp/p3a-big 20000 --due 100` (filling 50,000 cards takes about 10 minutes; 20,000 is usually enough to listen), run Rappel on that folder (`RAPPEL_DATA_DIR=$PWD/target/tmp/p3a-big cargo run -p rappel`), Export…, quit; then in the test folder Import… that file. While "Checking the file…" shows, Tab reaches **Cancel** and Orca reads the toast's text and "Cancel"; Escape cancels too: "Import cancelled", nothing imported. (If the check ends before you can listen, use more cards.)
    - Also: All Cards' Study button reads "Study Cards Tagged geography in Every Deck"; the merge drop-down reads "Merge Into"; each Unarchive button reads "Unarchive “…”"; the review page's title in a tag session reads "Tagged geography".
20. **Answer QUESTIONS.md, Phase 3a**, notes 1 to 7.
21. **Merge the draft PR `phase-3a` → `main`** when you are satisfied. Tag `v0.4.0-beta.1` yourself if you want a release; I did not tag. The CHANGELOG's compare links point at that tag, so they give a 404 until it exists. Then close issues #102, #103, #104, #105, #106, #117 and #119; close #48 once you have answered it (step 18).

## Decisions, with reasons
The full list is in each PR's **Decisions** section and its ADR. The ones that change behaviour you might not expect:
- **#110:**
  - a tooltip's key text comes from the page's own key table through GTK, so it is in your language and cannot name a key the page does not bind;
  - Ctrl+A is caught before the list and the text field, and leaves a focused text field alone by an explicit check;
  - Move to Deck… has no key of its own (a stray letter should not open a choice), and it is also on All Cards' rows, which share the deck page's row.
- **#113:**
  - one controller (`ui::selection`) plus two small shared widgets for the header and the bar, because the header and the bar are separate bars of the page;
  - sharing it changed All Cards slightly: Move… is disabled with one deck, and an emptied list ends selection mode.
- **#114:**
  - deleted cards move too and keep their own deletion time, so a later Delete Deck and its Undo never revive them;
  - the dialog counts live cards only, suspended ones included: what you see in the deck;
  - merging from a deck page goes back to the deck list, because the deck's page has nothing left to show;
  - Merge is the suggested, not destructive, answer, since Undo restores everything; Cancel is the default;
  - **the import rule of ADR 0067** (question 1).
- **#115:**
  - an archived deck is live: it keeps its name and nothing but the flag changes, so Archive then Unarchive leaves the database exactly as it was;
  - archived decks still count in the global activity (it is history) and their cards in tags and exports;
  - an import never archives or unarchives, as with `suspended`;
  - All Cards' archived drop-down is a view setting, not a filter: Escape and Clear Filters leave it, and it shows only while a deck is archived.
- **#116:**
  - a tag session takes from each deck at most what is left of that deck's own limits, rather than one allowance for the tag, which could exceed a deck's day or starve it;
  - the deck page's entry is in each card's ⋮ menu, not on the tag pills (a pill per tag per row would add a Tab stop each);
  - a deck archived during a tag session leaves it, and Undo never brings its cards back.
- **#118:**
  - practice writes nothing, enforced by `PRAGMA query_only` on its reads (ADR 0070: FSRS would learn from a recall at t ≈ 0, and the log is append-only);
  - at most 50 cards, weakest first; a Fail comes back after 3 other cards;
  - Practice replaces the finished session, so Back does not land on it;
  - a banner, not a changed title or colours, says it is practice (HIG: banners show a lasting state of a view);
  - Suspend, Move and Delete stay available (question 3).
- **#121:**
  - All Cards' filters wrap (`AdwWrapBox`) rather than being cut or hidden in a popover;
  - the export pre-check is a lower bound, so it never refuses an export that would import; a file still too large after writing is not kept and the question is asked then;
  - Escape cancels an import check only when no dialog is open;
  - "Enter" rather than GTK's "Return" in "Save (Ctrl+Enter)".
- **#122:**
  - "the same front" means the same search text: case, accents, whitespace and Markdown ignored, punctuation kept;
  - one indexed query anchored to the front's start, then an exact comparison; fronts over 64 KiB are not looked up;
  - Show Card opens Card Information over the editor, which keeps your text, rather than a second editor.
- **#123:**
  - one ADR per hardening round (0073), as before;
  - the same-front lookup's remaining cost against crafted imports is accepted, since dropping the anchor would slow every normal lookup;
  - the drop-down test was kept; it now says which assertion failed.
- **Process:**
  - slice 7 was redone on Opus by taking over its branch (question 4);
  - from mid-run, the CI budget: PRs opened only after the gate and the review, one push of fixes;
  - this report and its draft PR are the only things aimed at `main`, and nothing is tagged.
