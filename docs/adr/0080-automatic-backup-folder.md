# 80. Automatic backups to a folder the user chooses

- Status: Accepted
- Date: 2026-11-09
- Refers to: #127; Phase 3b brief, slice 5 (its pre-approved setting and deletion rule); SPEC §4.1 (the export), §4.8 (Preferences); ADR 0017 (the export is deterministic), 0026 (the database's own backups), 0027 (Preferences), 0039 (`window-state.ini`), 0050 (row 33: untrusted key files), 0058 (toast titles at 360 px), 0071 (the 2 GiB an import reads)

## Context

Rappel already backs up its database inside the data directory (ADR 0026). That copy dies with the disk. #127 asks for a copy somewhere the user picks, such as a synced folder or a USB disk, made without thinking about it. The brief fixes the shape:
- one setting, off by default, with the folder chosen through `GtkFileDialog::select_folder`;
- at close, if the collection changed since the last backup and none was made today, a normal export named `Rappel backup YYYY-MM-DD.rappel`;
- the 14 newest files matching that pattern are kept, and older ones are deleted;
- failures never block closing or lose data, and the next start says what went wrong.

Two things are pre-approved: the setting lives in a key file in the data directory, and the job may delete regular files (not links) of its own exact pattern beyond the 14 newest. Nothing else may be deleted, and no migration or format change is allowed.

## Decision

**What a backup is.** A normal export: format version 2, with media, the same bytes `Export…` would write at that moment (ADR 0017). It is named `Rappel backup 2026-11-09.rappel` from the local calendar date when it is made. The local date is used, not UTC, because that is the date the user reads. It is a calendar day, not the study day of ADR 0027: a file name is about when, not about studying. The file is created with mode 0600, like the database.

An export that would be over 2 GiB fails as "too large", and no backup without media is made in its place. A smaller file holding something else, without saying so, would surprise the user more than a toast that says to use Export… without media.

**Where the setting lives.** `<data dir>/auto-backup.ini` is a `GKeyFile`, read and written like `window-state.ini` (ADR 0039, 0050 row 33):
- read only if it is a regular file of at most 16 KiB, opened with `O_NOFOLLOW | O_NONBLOCK`;
- written in one atomic step with mode 0600.

It has four groups:
- `[settings] folder`: the folder;
- `[last-backup] date, folder, fingerprint`: the last backup made;
- `[failure] reason, detail`: a failure to report at the next start;
- `[running] date`: a backup that started and has not ended.

The shared reading and writing moved from `window_state.rs` to `key_file.rs`. Each value is checked when read: the folder must be an absolute path; the date must be a real `YYYY-MM-DD`; the fingerprint must be 64 lowercase hex digits. An unknown reason reads as "other", so a failure is never lost. A broken file means backups are off, which is the default. The file is not in the database, so it stays out of exports, imports and database restores. The folder belongs to this computer.

**"Changed since the last backup".** `Store::collection_fingerprint` is the SHA-256 of `cards.json` as an export with media would write it, with `exported_at` set to 0, followed by the hashes of the media files present in the media folder (a regular file of the recorded size; looked at, not read). It hashes the export's own reading, so it changes when a backup would hold something different. A media file damaged in place at its recorded size is the one change it does not see. That covers every kind of change: edits, tags, suspension, moves, archiving, settings, reviews and media records. A timestamp would miss most of these: tags, suspension and moves do not touch `updated_at`, and there is no revision counter. Nothing is stored in the database and no migration is needed. It costs one read of every row and one `stat` per media file. It runs on the worker, so the close never waits for it. It runs before the folder is opened: on a day nothing changed, an unplugged disk says nothing, and an unchanged check sets the last backup's date to today (never keeping a later date), so the next close that day does not read everything again.

A backup that had to leave out a missing media file keeps its fingerprint, which has that file absent. The next day's backup is skipped while nothing else changes, so no identical incomplete backup is written each day; once the file is back, the fingerprint differs and the file gets into a backup.

**Once a day.** A pure function `due(last, folder, today)` decides:
- a backup is due when the last one went to another folder or is dated before `today`;
- a last backup dated one or two days after `today` counts as today's: the local dates of one instant differ by at most two days (UTC+14 and UTC−12 are 26 hours apart, `MAX_ZONE_SPREAD_DAYS`), so this is the user who flew west;
- a last backup dated three days or more ahead was made with a clock set wrong. It is due, so one bad clock never stops backups until that date; the next backup records today's date;
- so travelling never gives two backups with one date, and a new folder gets its first backup at once.

`changed(last, folder, fingerprint)` then compares the fingerprints. An unknown fingerprint counts as changed. Core tests walk closes through Paris, New York, Tokyo, UTC+14 and UTC−12, and across a daylight-saving change, with `LocalDate::from_utc_ms`. The app takes today from `g_date_time_new_now_local`.

**Writing safely, with one handle on the folder.** Core's `auto_backup::run` opens the folder once, with `O_DIRECTORY | O_NOFOLLOW`. Everything else is relative to that handle, through `rustix`'s safe `openat`, `renameat2`, `unlinkat` and `fstatat`:
- The export goes to `.Rappel backup 2026-11-09.rappel.<16 random hex>.partial`. It is a new file (`O_CREAT | O_EXCL | O_NOFOLLOW`, mode 0600) whose name never matches the pattern: the leading dot and the suffix see to that, and a test checks it.
- It is synced, then renamed with `RENAME_NOREPLACE`, then the folder is synced. A half-written backup therefore never has a backup's name.
- On a file system without `RENAME_NOREPLACE` (`EINVAL`, `ENOTSUP`, `ENOSYS`), a check that the name is free comes first, then a plain rename.
- On error, the partial file this call created is removed.
- If something already has today's name, nothing is written and it is kept. A regular file there is today's backup already (its record was lost, or the folder was used again the same day): the record keeps the date with an unknown fingerprint. Anything else (a link, a folder) is not Rappel's: the backup fails as "name taken", so the user is told instead of believing a backup was made.
- The export stops at its next write once a cancel flag is set (`Store::export_into` wraps the file). The partial file is then removed like on any other error.
- An IO error says why ("not found", "disk full"…) only when it is about a file in the backup folder. Any other error, such as a media file that disappeared during the export, is "something unexpected" with the error itself, so the toast never blames the folder for it.
- A folder sync that the file system does not support (`EINVAL`, `ENOTSUP`) after the rename counts as done: the backup is there.

Because everything goes through the handle, a folder that is renamed, removed or replaced while the backup runs gets nothing written elsewhere. A removed folder fails with "not found" on the handle (tested).

**Deleting.** Only regular files whose names match `Rappel backup YYYY-MM-DD.rappel` exactly are considered:
- the check is a literal prefix and suffix, four, two and two ASCII digits, and a date that exists;
- each file is checked with `fstatat(AT_SYMLINK_NOFOLLOW)`, since a directory entry's type may be unknown.

The pure `to_delete` keeps the 14 newest by the date in the name, not by the file's time, so a sync tool that touches the files changes nothing. The file just written always stays: if a clock that ran ahead left 14 later dates, the oldest of those goes instead. Backups dated far ahead are not ranked below today's: the pre-approved rule allows deleting only files beyond the 14 newest by date, and a future-dated file is among the newest by date. So a wrong clock that left several future-dated files makes them take slots until they are older than 14 later backups, and they are never deleted early; the review suggested ranking them lower, and this rule is why it was not done. Each file is checked again just before `unlinkat`, which never follows a link anyway, so nothing outside the folder can be deleted. One gap remains: a regular file swapped for a link in the instant between that check and `unlinkat` would have the link removed (never its target). Linux has no "unlink only if regular", so this cannot be closed; only a process racing on purpose in the user's own folder could hit it. A test fills a folder with old backups, look-alike names, links named like backups (to a file outside, to a backup inside, dangling) and a folder named like one. Only the old regular backups go, and every link target is untouched. A failed deletion is logged and does not fail the backup.

**The folder as chosen.** Choose… opens `GtkFileDialog::select_folder`. Under a sandbox (the snap, a Flatpak) it goes through the desktop portal's file chooser. The path GTK returns (`g_file_get_path`) is stored with its links resolved (`canonicalize`), so a link the user went through is followed once, when chosen. If it cannot be resolved, the choice is refused with a toast, rather than saving a setting whose every backup would fail. A link found in the folder's place later was put there since, so it is refused ("replaced by a link; choose it again"). A link higher up the path is the user's own layout and is followed; a test covers it.

Under the portal the path may be a document-portal path, `/run/user/<uid>/doc/<id>/<name>`. It keeps working after a restart as long as the document portal still grants the folder to the app: the grant is kept in the permission store under the app ID. If the grant is gone, the backup fails as "not found" at the next close, the next start says so, and choosing the folder again fixes it. The row shows only the folder's name (its last component, which is the real folder name even under the portal); the full path is in the row's tooltip. Slice 7, the snap, should check this on a real strict snap.

**Closing.** Closing must never wait. `close_request` runs `save_state`, then `start_auto_backup`, then lets the window close:
1. If backups are on and one is due, `[running]` is written.
2. The application is held (`g_application_hold`), so the process outlives its window.
3. The job runs on GIO's thread pool, on its own database connection, which it only reads through.
4. When it ends, the result is recorded and the hold is released. The process then exits as usual.

A cap of 5 minutes (`TIME_CAP`) bounds this. If the job still runs then:
1. its cancel flag is set, so its next write fails and it removes its own partial file;
2. the failure is recorded as "took too long";
3. after a grace of 30 seconds (`CANCEL_GRACE`, for a disk stuck inside one write), the backup's own hold is released even if the worker has not returned.

The application is never told to quit. `quit` would also end other work that holds it through `ui::worker::run`, such as an import, an Anki import, Clean Up Media or the retention recalculation, which closing the window has always let finish. With no window and no other hold left, `app.run` returns and the process ends; a worker still stuck in the disk ends with it.

A launch while the job runs reaches the same process (single instance) and opens a new window as usual. The job then finishes in the background, and that window does not report the running job as unfinished.

Only the main page backs up. A recovery or error page has no database open.

**Failures.** On failure the last-backup record is not touched, so the next close tries again. `[failure]` keeps a reason and the English error. At the next start, `report_at_start` shows a toast through `show_error` and forgets it: "Automatic backup failed: the disk is full". The title before ":" fits beside Details at 360 px (ADR 0058); in French it is "Échec de la sauvegarde". The reasons:
- not found ("is its disk connected?");
- not a folder;
- replaced by a link;
- disk or quota full;
- read-only;
- not allowed;
- too large;
- took too long;
- stopped before it finished (a `[running]` left from a shutdown);
- anything else, with the error itself.

The mapping from `errno` is a pure function, tested.

**Preferences.** A new group, "Automatic Backups", sits after Data. Its description is "Each day Rappel closes after a change, it saves an export to this folder and keeps the 14 newest". It has one `AdwActionRow`, "Folder", whose subtitle is "Off" or the folder's name, with two suffixes:
- **Choose…**, the row's activatable widget, which opens the chooser;
- a flat button with the `edit-clear-symbolic` icon, "Turn Off Automatic Backups" (tooltip and accessible label), shown only while a folder is set.

A folder inside the data directory (the data directory itself, its `backups/` or `media/`, with links resolved) is refused when chosen ("Choose a folder outside Rappel’s data folder"): a backup there would be lost with what it backs up. An `auto-backup.ini` that is there but cannot be read means backups are off; each start says so ("Automatic backups off: their setting could not be read. Choose the folder again in Preferences."), and choosing a folder writes the file anew.

Both save at once; a failed write shows the dialog's error toast. Turn Off hidden under the focus hands the focus to Choose….

**Dependency.** `rustix` (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; already in the tree through glycin's `async-io`) is now a direct dependency of `rappel-core`, with the `fs` feature. Std has no `*at` calls, and the workspace forbids `unsafe`.

## Consequences

- The export gains `Store::export_into` (an open file), used by `export_to_with` and the backup, and `collection_fingerprint`. The format is unchanged.
- A backup is skipped on a day nothing changed, so a quiet week leaves fewer than 7 files. The 14 kept are the 14 most recent changes.
- A backup cut off by the cap removes its own partial file. Only one cut off by a shutdown or a crash, or by a disk stuck past the grace period, leaves a hidden `.partial` file; the deletion rule does not allow removing it later, so it is left for the user.
- On a disk too slow for the cap (a large media collection on a USB 2 stick), every close that day tries again and is cancelled again; each start says it took too long. Export… has no cap for such a case.
- A day's later changes go into the next day's backup, not today's. That follows from "once a day".
- The record is per data directory, so a debug build (`rappel-dev`) has its own setting and never writes into the release folder's record.
- In French, the Folder row's "Off" is "Aucun" (no folder), which agrees with "Dossier".
- The full path is the row's tooltip and its accessible description.
- Not covered by tests: the GTK hold and cap at close, which the maintainer checks by closing the app (Try it). The portal's persistence under a strict snap is for slice 7 to confirm.
