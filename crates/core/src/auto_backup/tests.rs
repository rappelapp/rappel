use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use std::sync::atomic::AtomicBool;

use super::*;
use crate::domain::Grade;
use crate::test_support::{T0, answer};

fn date(text: &str) -> LocalDate {
    LocalDate::parse(text).unwrap()
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// `YYYY-MM-DDTHH:MM` UTC in milliseconds.
fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    utc_ms(UtcParts {
        year,
        month,
        day,
        hour,
        minute,
        second: 0,
    })
    .unwrap()
}

#[test]
fn a_backup_is_named_for_its_local_date() {
    assert_eq!(
        file_name(date("2026-11-09")),
        "Rappel backup 2026-11-09.rappel"
    );
    assert_eq!(
        file_name(LocalDate::new(7, 1, 2).unwrap()),
        "Rappel backup 0007-01-02.rappel"
    );
    assert_eq!(date("2026-11-09").to_string(), "2026-11-09");
    assert_eq!(
        parse_file_name("Rappel backup 2026-11-09.rappel"),
        Some(date("2026-11-09"))
    );
    assert_eq!(
        parse_file_name("Rappel backup 2024-02-29.rappel"),
        Some(date("2024-02-29"))
    );
}

/// Only the exact pattern is a backup: anything that merely looks like one
/// is never deleted.
#[test]
fn a_look_alike_name_is_not_a_backup() {
    for name in [
        "",
        "Rappel backup .rappel",
        "Rappel backup 2026-11-09.rappel.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef.partial",
        "Rappel backup 2026-11-09.rappel~",
        "Rappel backup 2026-11-09.Rappel",
        "Rappel backup 2026-11-09 (1).rappel",
        "Rappel backup 2026-11-09.cards",
        "rappel backup 2026-11-09.rappel",
        "Rappel Backup 2026-11-09.rappel",
        "Rappel  backup 2026-11-09.rappel",
        " Rappel backup 2026-11-09.rappel",
        "Rappel backup 2026-11-09.rappel ",
        "Copy of Rappel backup 2026-11-09.rappel",
        "Rappel backup 2026-11-9.rappel",
        "Rappel backup 26-11-09.rappel",
        "Rappel backup 2026_11_09.rappel",
        "Rappel backup 20261109.rappel",
        "Rappel backup 2026-11-09-2.rappel",
        "Rappel backup 2026-13-01.rappel",
        "Rappel backup 2026-00-10.rappel",
        "Rappel backup 2026-04-31.rappel",
        "Rappel backup 2025-02-29.rappel",
        "Rappel backup 0000-01-01.rappel",
        "Rappel backup +202-11-09.rappel",
        "Rappel backup -202-11-09.rappel",
        "Rappel backup 2026-1-109.rappel",
        "Rappel backup २०२६-११-०९.rappel",
        "Rappel backup 2026\u{2010}11\u{2010}09.rappel",
        "Rappel backup 2026-11-09.rappel/",
        "Rappel backup 2026-11-09.rappel\0",
        "rappel-20261109-120000.sqlite3",
    ] {
        assert_eq!(parse_file_name(name), None, "{name:?}");
    }
    // The hidden partial file is never a backup, whatever its random part.
    for _ in 0..20 {
        let partial = partial_name(&file_name(date("2026-11-09"))).unwrap();
        assert!(partial.starts_with(".Rappel backup 2026-11-09.rappel."));
        assert!(partial.ends_with(".partial"));
        assert_eq!(parse_file_name(&partial), None, "{partial}");
    }
}

#[test]
fn dates_are_checked() {
    assert_eq!(LocalDate::new(2026, 2, 29), None);
    assert_eq!(LocalDate::new(2026, 4, 31), None);
    assert_eq!(LocalDate::new(0, 1, 1), None);
    assert_eq!(LocalDate::new(10_000, 1, 1), None);
    assert_eq!(LocalDate::new(2026, 0, 1), None);
    assert_eq!(LocalDate::new(2026, 1, 0), None);
    assert_eq!(LocalDate::new(-1, 1, 1), None);
    assert!(LocalDate::new(9999, 12, 31).is_some());
    assert_eq!(LocalDate::parse("2026-11-09 "), None);
    assert_eq!(LocalDate::parse("2026/11/09"), None);
    assert!(date("2026-11-09") < date("2026-11-10"));
    assert!(date("2026-12-31") < date("2027-01-01"));
}

/// Twenty backups, one a day: the fourteen newest stay.
#[test]
fn the_fourteen_newest_stay() {
    let found: Vec<(String, LocalDate)> = (1..=20)
        .map(|day| {
            let d = LocalDate::new(2026, 10, day).unwrap();
            (file_name(d), d)
        })
        .collect();
    let newest = file_name(date("2026-10-20"));
    let mut gone = to_delete(&found, &newest);
    gone.sort();
    let expected: Vec<String> = (1..=6)
        .map(|day| file_name(LocalDate::new(2026, 10, day).unwrap()))
        .collect();
    assert_eq!(gone, expected);
    // In any order.
    let mut reversed = found.clone();
    reversed.reverse();
    let mut gone_reversed = to_delete(&reversed, &newest);
    gone_reversed.sort();
    assert_eq!(gone_reversed, expected);
    // Fourteen or fewer: nothing goes.
    assert_eq!(to_delete(&found[6..], &newest), Vec::<String>::new());
    assert_eq!(to_delete(&found[..3], &newest), Vec::<String>::new());
    assert_eq!(to_delete(&[], &newest), Vec::<String>::new());
}

/// The backup just written stays even if a clock that ran ahead left
/// fourteen backups dated after it: the oldest of those goes instead.
#[test]
fn the_backup_just_written_always_stays() {
    let mut found: Vec<(String, LocalDate)> = (1..=14)
        .map(|day| {
            let d = LocalDate::new(2030, 1, day).unwrap();
            (file_name(d), d)
        })
        .collect();
    let today = date("2026-11-09");
    found.push((file_name(today), today));
    assert_eq!(
        to_delete(&found, &file_name(today)),
        vec![file_name(date("2030-01-01"))]
    );
    // Not in the list (never the case after a write): the fourteen newest.
    let gone = to_delete(&found, "something else");
    assert_eq!(gone, vec![file_name(today)]);
}

#[test]
fn a_backup_is_due_once_a_day_and_per_folder() {
    let folder = Path::new("/backups");
    let last = LastBackup {
        date: date("2026-11-09"),
        folder: folder.to_path_buf(),
        fingerprint: Some("abc".to_owned()),
    };
    assert!(due(None, folder, date("2026-11-09")));
    assert!(!due(Some(&last), folder, date("2026-11-09")));
    assert!(due(Some(&last), folder, date("2026-11-10")));
    assert!(due(Some(&last), folder, date("2027-01-01")));
    // Dated up to two days after today (flown west): not twice.
    assert!(!due(Some(&last), folder, date("2026-11-08")));
    assert!(!due(Some(&last), folder, date("2026-11-07")));
    // Dated three days or more ahead: a clock set wrong once, which must
    // not stop backups until then.
    assert!(due(Some(&last), folder, date("2026-11-06")));
    assert!(due(Some(&last), folder, date("2025-11-09")));
    let far = LastBackup {
        date: date("2030-01-01"),
        ..last.clone()
    };
    assert!(due(Some(&far), folder, date("2026-11-09")));
    // Across a year end.
    let new_year = LastBackup {
        date: date("2027-01-02"),
        ..last.clone()
    };
    assert!(!due(Some(&new_year), folder, date("2026-12-31")));
    assert!(due(Some(&new_year), folder, date("2026-12-30")));
    // Another folder gets its first backup at once.
    assert!(due(
        Some(&last),
        Path::new("/elsewhere"),
        date("2026-11-09")
    ));
}

#[test]
fn a_backup_is_made_only_when_the_collection_changed() {
    let folder = Path::new("/backups");
    let last = LastBackup {
        date: date("2026-11-09"),
        folder: folder.to_path_buf(),
        fingerprint: Some("abc".to_owned()),
    };
    assert!(changed(None, folder, "abc"));
    assert!(!changed(Some(&last), folder, "abc"));
    assert!(changed(Some(&last), folder, "abd"));
    assert!(changed(Some(&last), Path::new("/elsewhere"), "abc"));
    let unknown = LastBackup {
        fingerprint: None,
        ..last
    };
    assert!(changed(Some(&unknown), folder, "abc"));
}

/// Closing Rappel at many times, in many time zones, travelling between
/// them: never two backups on one local date, and one on each new local
/// date (when something changed).
#[test]
fn once_a_day_across_time_zones() {
    let folder = Path::new("/backups");
    // (UTC instant, offset in seconds of the zone the computer is set to)
    let paris_winter = 3600;
    let paris_summer = 7200;
    let new_york = -5 * 3600;
    let tokyo = 9 * 3600;
    let kiribati = 14 * 3600;
    let baker = -12 * 3600;
    let closes = [
        // Paris, late on the 9th, then just after midnight: the 10th.
        (utc(2026, 11, 9, 22, 30), paris_winter, Some("2026-11-09")),
        (utc(2026, 11, 9, 22, 50), paris_winter, None),
        (utc(2026, 11, 9, 23, 10), paris_winter, Some("2026-11-10")),
        // Flown to New York: still the 9th there. Not again.
        (utc(2026, 11, 10, 3, 0), new_york, None),
        (utc(2026, 11, 10, 4, 59), new_york, None),
        // The 10th in New York: the 10th's backup was made in Paris.
        (utc(2026, 11, 10, 5, 0), new_york, None),
        (utc(2026, 11, 10, 20, 0), new_york, None),
        // The 11th in New York.
        (utc(2026, 11, 11, 5, 1), new_york, Some("2026-11-11")),
        // Flown to Tokyo: the 12th there already.
        (utc(2026, 11, 11, 16, 0), tokyo, Some("2026-11-12")),
        (utc(2026, 11, 12, 14, 0), tokyo, None),
        (utc(2026, 11, 12, 15, 0), tokyo, Some("2026-11-13")),
        // The far ends of the clock: +14 is the 14th from 10:00 UTC on
        // the 13th; -12 is still the 13th until 12:00 UTC on the 14th.
        (utc(2026, 11, 13, 10, 0), kiribati, Some("2026-11-14")),
        (utc(2026, 11, 14, 11, 59), baker, None),
        (utc(2026, 11, 14, 12, 0), baker, None),
        (utc(2026, 11, 15, 12, 0), baker, Some("2026-11-15")),
    ];
    let mut last: Option<LastBackup> = None;
    let mut made = Vec::new();
    for (i, (instant, offset, expected)) in closes.into_iter().enumerate() {
        let today = LocalDate::from_utc_ms(instant, offset).unwrap();
        // Something changed before every close.
        let fingerprint = format!("state {i}");
        let backed_up =
            due(last.as_ref(), folder, today) && changed(last.as_ref(), folder, &fingerprint);
        assert_eq!(
            backed_up.then(|| today.to_string()).as_deref(),
            expected,
            "close {i} at {today}"
        );
        if backed_up {
            made.push(today);
            last = Some(LastBackup {
                date: today,
                folder: folder.to_path_buf(),
                fingerprint: Some(fingerprint),
            });
        }
    }
    let mut unique = made.clone();
    unique.dedup();
    assert_eq!(unique, made, "never two backups with one date");

    // Daylight saving: the same local day before and after the change
    // (Paris, 25 October 2026, 03:00 summer time becomes 02:00 winter
    // time) is one day.
    let before = LocalDate::from_utc_ms(utc(2026, 10, 25, 0, 30), paris_summer).unwrap();
    let after = LocalDate::from_utc_ms(utc(2026, 10, 25, 1, 30), paris_winter).unwrap();
    assert_eq!(before, after);
    let last = LastBackup {
        date: before,
        folder: folder.to_path_buf(),
        fingerprint: None,
    };
    assert!(!due(Some(&last), folder, after));
}

#[test]
fn local_dates_from_instants() {
    let noon = utc(2026, 11, 9, 12, 0);
    assert_eq!(LocalDate::from_utc_ms(noon, 0), Some(date("2026-11-09")));
    assert_eq!(
        LocalDate::from_utc_ms(noon + 12 * HOUR_MS, 0),
        Some(date("2026-11-10"))
    );
    assert_eq!(
        LocalDate::from_utc_ms(noon, 12 * 3600),
        Some(date("2026-11-10"))
    );
    assert_eq!(
        LocalDate::from_utc_ms(noon - 1, -12 * 3600),
        Some(date("2026-11-08"))
    );
    assert_eq!(LocalDate::from_utc_ms(i64::MAX, 3600), None);
    assert_eq!(LocalDate::from_utc_ms(i64::MIN, 0), None);
}

/// A store with a deck and a card, in a temporary data directory, and a
/// backup folder beside it.
struct Env {
    tmp: tempfile::TempDir,
    store: Store,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&tmp.path().join("data")).unwrap();
        let deck = store.create_deck("Maths").unwrap();
        store.add_card(&deck.id, "2 + 2", "4").unwrap();
        fs::create_dir(tmp.path().join("backups")).unwrap();
        Self { tmp, store }
    }

    fn data(&self) -> PathBuf {
        self.tmp.path().join("data")
    }

    fn folder(&self) -> PathBuf {
        self.tmp.path().join("backups")
    }

    fn run(&self, today: &str, last: Option<&LastBackup>) -> Result<Outcome, Failure> {
        run(
            &self.data(),
            &self.folder(),
            date(today),
            last,
            &AtomicBool::new(false),
        )
    }

    /// The names in the backup folder, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.folder())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn fingerprint(&self) -> String {
        self.store.collection_fingerprint().unwrap()
    }
}

/// The fingerprint sees every kind of change an export would hold, and
/// nothing else.
#[test]
fn the_fingerprint_changes_with_the_collection() {
    let mut env = Env::new();
    let deck = env.store.create_deck("Words").unwrap();
    let card = env.store.add_card(&deck.id, "chat", "cat").unwrap();
    let mut seen = vec![env.fingerprint()];
    assert_eq!(env.fingerprint(), seen[0], "the same when nothing changed");
    assert_eq!(seen[0].len(), 64);
    let mut step = |env: &Env, what: &str| {
        let now = env.fingerprint();
        assert!(!seen.contains(&now), "{what} changed nothing");
        seen.push(now);
    };
    env.store.edit_card(&card.id, "chien", "dog").unwrap();
    step(&env, "editing a card");
    env.store
        .add_tags_to_cards(std::slice::from_ref(&card.id), &["animal".to_owned()], 20)
        .unwrap();
    step(&env, "a tag");
    env.store
        .set_suspended(std::slice::from_ref(&card.id), true)
        .unwrap();
    step(&env, "suspending");
    let other = env.store.create_deck("Other").unwrap();
    step(&env, "a deck");
    env.store
        .move_cards(std::slice::from_ref(&card.id), &other.id)
        .unwrap();
    step(&env, "moving a card");
    env.store.archive_deck(&deck.id).unwrap();
    step(&env, "archiving");
    env.store.set_day_start_hour(6).unwrap();
    step(&env, "a setting");
    let first = env.store.list_cards(&other.id).unwrap()[0].id.clone();
    env.store
        .set_suspended(std::slice::from_ref(&first), false)
        .unwrap();
    step(&env, "unsuspending");
    answer(&mut env.store, &first, Grade::Pass, T0);
    step(&env, "a review");
    let media = env
        .store
        .add_media_bytes(&crate::test_support::png_bytes(3))
        .unwrap();
    step(&env, "a media file");
    // The file going missing, and coming back, are changes too: a backup
    // that had to leave it out is followed by one that has it.
    let path = env.store.media_path(&media).unwrap();
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    let missing = env.fingerprint();
    assert!(
        !seen.contains(&missing),
        "a missing media file changed nothing"
    );
    fs::write(&path, &bytes).unwrap();
    assert_eq!(&env.fingerprint(), seen.last().unwrap(), "back as it was");
    // Reading and time passing change nothing.
    let now = env.fingerprint();
    env.store.list_decks(T0 + DAY_MS, T0).unwrap();
    assert_eq!(env.fingerprint(), now);
}

#[test]
fn a_backup_is_written_then_skipped_until_something_changes() {
    let mut env = Env::new();
    let Ok(Outcome::Written {
        file_name: name,
        fingerprint,
        summary,
        removed,
    }) = env.run("2026-11-09", None)
    else {
        panic!("written");
    };
    assert_eq!(name, "Rappel backup 2026-11-09.rappel");
    assert_eq!((summary.decks, summary.cards, removed), (1, 1, 0));
    assert_eq!(env.names(), vec![name.clone()]);
    let path = env.folder().join(&name);
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    // A normal export: it imports.
    let plan = env.store.plan_import(&path).unwrap();
    assert!(plan.summary().is_noop(), "the same data");

    let last = LastBackup {
        date: date("2026-11-09"),
        folder: env.folder(),
        fingerprint: Some(fingerprint),
    };
    // Nothing changed: nothing is written, the next day either.
    assert_eq!(env.run("2026-11-10", Some(&last)), Ok(Outcome::Unchanged));
    assert_eq!(env.names().len(), 1);

    let deck = env.store.create_deck("New").unwrap();
    env.store.add_card(&deck.id, "a", "b").unwrap();
    let Ok(Outcome::Written { file_name, .. }) = env.run("2026-11-10", Some(&last)) else {
        panic!("written");
    };
    assert_eq!(file_name, "Rappel backup 2026-11-10.rappel");
    assert_eq!(env.names().len(), 2);
}

/// Today's name taken (by an earlier backup whose record was lost, or by
/// anything else): it is kept as it is, never replaced.
#[test]
fn todays_name_taken_is_never_replaced() {
    let env = Env::new();
    let today = env.folder().join("Rappel backup 2026-11-09.rappel");
    fs::write(&today, b"mine").unwrap();
    assert_eq!(env.run("2026-11-09", None), Ok(Outcome::AlreadyThere));
    assert_eq!(fs::read(&today).unwrap(), b"mine");
    fs::remove_file(&today).unwrap();
    // A link of that name: kept, what it points to is untouched, and the
    // user is told (not recorded as backed up).
    let elsewhere = env.tmp.path().join("elsewhere");
    fs::write(&elsewhere, b"precious").unwrap();
    symlink(&elsewhere, &today).unwrap();
    assert_eq!(
        env.run("2026-11-09", None).unwrap_err().kind,
        FailureKind::NameTaken
    );
    assert!(fs::symlink_metadata(&today).unwrap().is_symlink());
    assert_eq!(fs::read(&elsewhere).unwrap(), b"precious");
    assert_eq!(env.names(), vec!["Rappel backup 2026-11-09.rappel"]);
}

/// Old backups beyond the fourteen newest go; a link, a folder, and every
/// file whose name only looks like a backup's stay, and so does whatever a
/// link points to.
#[test]
fn only_old_backups_are_deleted_never_a_link_or_a_look_alike() {
    let env = Env::new();
    let folder = env.folder();
    for day in 1..=20 {
        let name = file_name(LocalDate::new(2026, 10, day).unwrap());
        fs::write(folder.join(name), b"old backup").unwrap();
    }
    let look_alikes = [
        "Rappel backup 2026-09-01.rappel.partial",
        ".Rappel backup 2026-09-01.rappel.0011223344556677.partial",
        "rappel backup 2026-09-01.rappel",
        "Rappel backup 2026-09-01 (1).rappel",
        "Rappel backup 2026-09-31.rappel",
        "Rappel backup 2026-09-01.rappel~",
        "notes.txt",
    ];
    for name in look_alikes {
        fs::write(folder.join(name), b"not a backup").unwrap();
    }
    // Links with a backup's name and an old date, to a file outside and
    // to a backup inside; a folder with a backup's name.
    let outside = env.tmp.path().join("precious.rappel");
    fs::write(&outside, b"precious").unwrap();
    let link_out = "Rappel backup 2026-08-01.rappel";
    symlink(&outside, folder.join(link_out)).unwrap();
    let link_in = "Rappel backup 2026-08-02.rappel";
    symlink(
        folder.join("Rappel backup 2026-10-01.rappel"),
        folder.join(link_in),
    )
    .unwrap();
    let dangling = "Rappel backup 2026-08-03.rappel";
    symlink(env.tmp.path().join("nowhere"), folder.join(dangling)).unwrap();
    let subfolder = "Rappel backup 2026-08-04.rappel";
    fs::create_dir(folder.join(subfolder)).unwrap();
    fs::write(folder.join(subfolder).join("inside"), b"x").unwrap();

    let Ok(Outcome::Written { removed, .. }) = env.run("2026-11-09", None) else {
        panic!("written");
    };
    // 20 old + 1 new = 21 backups; 14 stay, so 7 go: 1 to 7 October.
    assert_eq!(removed, 7);
    let names = env.names();
    let mut expected: Vec<String> = (8..=20)
        .map(|day| file_name(LocalDate::new(2026, 10, day).unwrap()))
        .collect();
    expected.push("Rappel backup 2026-11-09.rappel".to_owned());
    expected.extend(look_alikes.iter().map(|&n| n.to_owned()));
    expected.extend([link_out, link_in, dangling, subfolder].map(str::to_owned));
    expected.sort();
    assert_eq!(names, expected);
    assert_eq!(fs::read(&outside).unwrap(), b"precious");
    assert!(
        fs::symlink_metadata(folder.join(link_out))
            .unwrap()
            .is_symlink()
    );
    assert!(folder.join(subfolder).join("inside").exists());
    // The link inside now dangles (its target was an old backup), and is
    // still not deleted.
    assert!(
        fs::symlink_metadata(folder.join(link_in))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn a_missing_or_strange_folder_fails_safely() {
    let env = Env::new();
    let today = date("2026-11-09");
    let never = AtomicBool::new(false);
    let gone = env.tmp.path().join("gone");
    let failure = run(&env.data(), &gone, today, None, &never).unwrap_err();
    assert_eq!(failure.kind, FailureKind::FolderMissing, "{failure}");

    let file = env.tmp.path().join("a file");
    fs::write(&file, b"x").unwrap();
    let failure = run(&env.data(), &file, today, None, &never).unwrap_err();
    assert_eq!(failure.kind, FailureKind::NotAFolder, "{failure}");

    // A link in place of the folder is refused, and nothing is written
    // where it points.
    let link = env.tmp.path().join("link");
    symlink(env.folder(), &link).unwrap();
    let failure = run(&env.data(), &link, today, None, &never).unwrap_err();
    assert_eq!(failure.kind, FailureKind::FolderIsLink, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
    // A link higher up the path is the user's own layout and is followed.
    let parent_link = env.tmp.path().join("parent link");
    symlink(env.tmp.path(), &parent_link).unwrap();
    assert!(matches!(
        run(
            &env.data(),
            &parent_link.join("backups"),
            today,
            None,
            &never
        ),
        Ok(Outcome::Written { .. })
    ));
}

/// A folder the user may not write to: refused, and nothing is left in it.
/// (Root may write anywhere, so this is checked only when not root.)
#[test]
fn a_read_only_folder_fails_and_leaves_nothing() {
    let env = Env::new();
    let folder = env.folder();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = fs::write(folder.join("probe"), b"x");
    if probe.is_ok() {
        // Running as root: the permission does not apply.
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let failure = env.run("2026-11-09", None).unwrap_err();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(failure.kind, FailureKind::NotAllowed, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
}

/// A folder removed while the backup runs: the open handle still names the
/// removed folder, so nothing is written anywhere else, and the failure
/// says the folder is gone.
#[test]
fn a_folder_removed_before_the_write_fails_safely() {
    let env = Env::new();
    let dir = open_folder(&env.folder()).unwrap();
    fs::remove_dir(env.folder()).unwrap();
    // A new folder of the same name gets nothing.
    fs::create_dir(env.folder()).unwrap();
    let Err(Written::Failed(failure)) = write_backup(
        &dir,
        &env.folder(),
        "Rappel backup 2026-11-09.rappel",
        &env.store,
        &AtomicBool::new(false),
    ) else {
        panic!("refused");
    };
    assert_eq!(failure.kind, FailureKind::FolderMissing, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
}

#[test]
fn errors_say_why() {
    let kind = |errno: Errno| kind_of(&io::Error::from(errno));
    assert_eq!(kind(Errno::NOENT), FailureKind::FolderMissing);
    assert_eq!(kind(Errno::NOTDIR), FailureKind::NotAFolder);
    assert_eq!(kind(Errno::NOSPC), FailureKind::NoSpace);
    assert_eq!(kind(Errno::DQUOT), FailureKind::NoSpace);
    assert_eq!(kind(Errno::ROFS), FailureKind::ReadOnly);
    assert_eq!(kind(Errno::ACCESS), FailureKind::NotAllowed);
    assert_eq!(kind(Errno::PERM), FailureKind::NotAllowed);
    assert_eq!(kind(Errno::LOOP), FailureKind::FolderIsLink);
    assert_eq!(kind(Errno::IO), FailureKind::Other);
    let folder = Path::new("/backups");
    assert_eq!(
        Failure::core(&Error::ExportTooLarge { bytes: 2, limit: 1 }, folder).kind,
        FailureKind::TooLarge
    );
    let io_at = |path: &str, errno: Errno| Error::Io {
        path: PathBuf::from(path),
        source: io::Error::from(errno),
    };
    assert_eq!(
        Failure::core(&io_at("/backups/.partial", Errno::NOSPC), folder).kind,
        FailureKind::NoSpace
    );
    // A media file gone during the export is not the folder's fault.
    let media = Failure::core(&io_at("/data/media/ab.png", Errno::NOENT), folder);
    assert_eq!(media.kind, FailureKind::Other);
    assert!(media.detail.contains("ab.png"), "{media}");
    assert_eq!(
        Failure::core(&io_at("/backups-other/x", Errno::ACCESS), folder).kind,
        FailureKind::Other
    );
    assert_eq!(
        Failure::core(&Error::IdsExhausted, folder).kind,
        FailureKind::Other
    );
}

/// Nothing changed: the folder is not even opened, so an unplugged disk
/// says nothing on a quiet day.
#[test]
fn unchanged_never_opens_the_folder() {
    let env = Env::new();
    let last = LastBackup {
        date: date("2026-11-08"),
        folder: env.tmp.path().join("unplugged"),
        fingerprint: Some(env.fingerprint()),
    };
    assert_eq!(
        run(
            &env.data(),
            &last.folder,
            date("2026-11-09"),
            Some(&last),
            &AtomicBool::new(false)
        ),
        Ok(Outcome::Unchanged)
    );
}

/// A backup cut off by the app's time cap stops, says so, and leaves
/// nothing in the folder: no backup, no partial file.
#[test]
fn a_cancelled_backup_leaves_nothing() {
    let env = Env::new();
    let failure = run(
        &env.data(),
        &env.folder(),
        date("2026-11-09"),
        None,
        &AtomicBool::new(true),
    )
    .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Cancelled, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
}

// Hardening round 5 (ADR 0084): the backup folder attacked.

/// A leap day is a day like any other: one backup on it, one the day
/// after, and "flown west" counts it as a day.
#[test]
fn a_leap_day_is_a_day_like_any_other() {
    let folder = Path::new("/backups");
    let on = |text: &str| LastBackup {
        date: date(text),
        folder: folder.to_path_buf(),
        fingerprint: None,
    };
    assert!(due(Some(&on("2024-02-28")), folder, date("2024-02-29")));
    assert!(!due(Some(&on("2024-02-29")), folder, date("2024-02-29")));
    assert!(due(Some(&on("2024-02-29")), folder, date("2024-03-01")));
    // Two days ahead across the leap day: flown west, not again.
    assert!(!due(Some(&on("2024-03-01")), folder, date("2024-02-28")));
    // Three days ahead across it: a clock set wrong.
    assert!(due(Some(&on("2024-03-02")), folder, date("2024-02-28")));
    // In a common year the same dates are a day closer.
    assert!(!due(Some(&on("2023-03-02")), folder, date("2023-02-28")));
    assert_eq!(
        file_name(date("2024-02-29")),
        "Rappel backup 2024-02-29.rappel"
    );
    assert_eq!(
        LocalDate::from_utc_ms(utc(2024, 2, 28, 23, 30), 3600),
        Some(date("2024-02-29"))
    );
    assert_eq!(
        LocalDate::from_utc_ms(utc(2100, 2, 28, 23, 30), 3600),
        Some(date("2100-03-01")),
        "2100 is not a leap year"
    );
}

/// The job's own partial files are known by their exact pattern: the
/// names it makes, and nothing else.
#[test]
fn a_partial_file_is_known_by_its_exact_name() {
    for _ in 0..50 {
        let name = partial_name(&file_name(date("2024-02-29"))).unwrap();
        assert!(is_partial_name(&name), "{name}");
    }
    assert!(is_partial_name(
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef.partial"
    ));
    for name in [
        "",
        ".partial",
        "notes.partial",
        "Rappel backup 2026-11-09.rappel",
        "Rappel backup 2026-11-09.rappel.partial",
        "Rappel backup 2026-11-09.rappel.0123456789abcdef.partial",
        ".Rappel backup 2026-11-09.rappel.partial",
        ".Rappel backup 2026-11-09.rappel..partial",
        ".Rappel backup 2026-11-09.rappel.0123456789ABCDEF.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcde.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef0.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdeg.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef.partial~",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef.Partial",
        ".Rappel backup 2026-02-30.rappel.0123456789abcdef.partial",
        ".Rappel backup 2026-11-09.cards.0123456789abcdef.partial",
        ".Rappel backup 2026-11-09.0123456789abcdef.partial",
        ".rappel backup 2026-11-09.rappel.0123456789abcdef.partial",
        "..Rappel backup 2026-11-09.rappel.0123456789abcdef.partial",
        ".Rappel backup 2026-11-09.rappel.0123456789abcdef.partial/x",
        ".Rappel backup 2026-11-09 (1).rappel.0123456789abcdef.partial",
        ".Rappel backup ２０２６-11-09.rappel.0123456789abcdef.partial",
    ] {
        assert!(!is_partial_name(name), "{name:?}");
    }
}

#[test]
fn only_partial_files_left_for_a_day_are_stale() {
    let own = ".Rappel backup 2026-11-09.rappel.0123456789abcdef.partial".to_owned();
    let now = 2_000_000_000;
    let at = |modified: i64| stale_partials(&[(own.clone(), modified)], now);
    let none: Vec<String> = Vec::new();
    assert_eq!(at(now - STALE_PARTIAL_SECONDS), vec![own.clone()]);
    assert_eq!(at(now - STALE_PARTIAL_SECONDS - 1), vec![own.clone()]);
    assert_eq!(at(0), vec![own.clone()]);
    assert_eq!(at(i64::MIN), vec![own.clone()], "no overflow");
    assert_eq!(at(now - STALE_PARTIAL_SECONDS + 1), none);
    assert_eq!(at(now), none);
    // A time ahead of the clock (set wrong) is never stale.
    assert_eq!(at(now + 1), none);
    assert_eq!(at(i64::MAX), none);
    // Another name, however old, is never stale.
    assert_eq!(
        stale_partials(&[("notes.partial".to_owned(), 0)], now),
        none
    );
}

/// Sets the modification time of `path` (not following a link) to `secs`
/// after 1970.
fn set_modified(path: &Path, secs: i64) {
    let time = rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: secs,
            tv_nsec: 0,
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: secs,
            tv_nsec: 0,
        },
    };
    rustix::fs::utimensat(rustix::fs::CWD, path, &time, AtFlags::SYMLINK_NOFOLLOW).unwrap();
}

/// A backup killed by a shutdown or a crash leaves its partial file
/// (nothing in the process is left to remove it). Before round 5 these
/// were never removed, so a computer often shut down while a backup ran
/// gathered one (as large as a backup) per day without bound. Now each
/// backup first removes the job's own partial files left for a day:
/// regular files of the exact pattern only, never a link, a folder, a
/// user's file or a partial file of a backup that may still run.
#[test]
fn partial_files_left_by_killed_backups_are_removed_after_a_day() {
    let env = Env::new();
    let folder = env.folder();
    let old = now_seconds() - 2 * STALE_PARTIAL_SECONDS;
    // A thousand killed backups, one a day.
    let mut killed = Vec::new();
    for i in 0..1000 {
        let name = partial_name(&file_name(date("2026-11-09"))).unwrap();
        fs::write(folder.join(&name), format!("killed {i}")).unwrap();
        set_modified(&folder.join(&name), old - i);
        killed.push(name);
    }
    let recent = partial_name(&file_name(date("2026-11-09"))).unwrap();
    fs::write(folder.join(&recent), b"maybe still running").unwrap();
    let mine = "my notes.partial";
    fs::write(folder.join(mine), b"the user's").unwrap();
    set_modified(&folder.join(mine), old);
    let outside = env.tmp.path().join("precious");
    fs::write(&outside, b"precious").unwrap();
    set_modified(&outside, old);
    let link = ".Rappel backup 2026-11-01.rappel.00000000000000aa.partial";
    symlink(&outside, folder.join(link)).unwrap();
    set_modified(&folder.join(link), old);
    let subfolder = ".Rappel backup 2026-11-01.rappel.00000000000000bb.partial";
    fs::create_dir(folder.join(subfolder)).unwrap();
    set_modified(&folder.join(subfolder), old);

    let Ok(Outcome::Written {
        file_name: name, ..
    }) = env.run("2026-11-10", None)
    else {
        panic!("written");
    };
    let mut expected = vec![
        name,
        recent,
        mine.to_owned(),
        link.to_owned(),
        subfolder.to_owned(),
    ];
    expected.sort();
    assert_eq!(env.names(), expected);
    assert_eq!(fs::read(&outside).unwrap(), b"precious");
    // A day later the recent one is old too, and goes at the next backup,
    // even one that finds today's backup already there.
    let recent = expected
        .iter()
        .find(|n| {
            is_partial_name(n) && fs::symlink_metadata(folder.join(n)).is_ok_and(|m| m.is_file())
        })
        .unwrap()
        .clone();
    set_modified(&folder.join(&recent), old);
    assert_eq!(env.run("2026-11-10", None), Ok(Outcome::AlreadyThere));
    assert!(!env.names().contains(&recent));
}

/// The folder inside the data directory, however it is named: refused by
/// the app when chosen, and by the backup itself for a setting saved
/// before that check, a data directory moved since, or a path through
/// links, `.` or `..`. Nothing is written in it.
#[test]
fn a_folder_inside_the_data_directory_is_refused() {
    let env = Env::new();
    let data = fs::canonicalize(env.data()).unwrap();
    let inside = data.join("my backups");
    fs::create_dir(&inside).unwrap();
    let data_link = env.tmp.path().join("data link");
    symlink(&data, &data_link).unwrap();
    let dotted = env.tmp.path().join("backups/../data/./my backups");
    let today = date("2026-11-09");
    let never = AtomicBool::new(false);
    for folder in [
        data.clone(),
        data.join("backups"),
        data.join("media"),
        inside.clone(),
        data_link.join("my backups"),
        dotted.clone(),
    ] {
        assert!(inside_data_dir(&folder, &data), "{}", folder.display());
        assert!(inside_data_dir(&folder, &data_link), "{}", folder.display());
        if folder.is_dir() {
            let failure = run(&data_link, &folder, today, None, &never).unwrap_err();
            assert_eq!(failure.kind, FailureKind::InsideDataDir, "{failure}");
        }
    }
    assert_eq!(fs::read_dir(&inside).unwrap().count(), 0, "nothing written");
    // Beside it, or above it: not inside.
    assert!(!inside_data_dir(&env.folder(), &data));
    assert!(!inside_data_dir(env.tmp.path(), &data));
    assert!(!inside_data_dir(
        &env.tmp.path().join("data-backups"),
        &data
    ));
    // A data directory moved to be above the folder (`RAPPEL_DATA_DIR`
    // pointed at a parent of it since): refused at the next backup.
    assert!(inside_data_dir(&env.folder(), env.tmp.path()));
    let failure = run(&env.data(), &env.folder(), today, None, &never);
    assert!(failure.is_ok(), "the real data directory is beside it");
    // A data directory that holds the folder (as `RAPPEL_DATA_DIR`
    // pointed at a parent of it since): `run` gets to the folder and
    // refuses it.
    let parent = tempfile::tempdir().unwrap();
    let held = parent.path().join("Backups");
    fs::create_dir(&held).unwrap();
    let mut other = Store::open(parent.path()).unwrap();
    let deck = other.create_deck("Other").unwrap();
    other.add_card(&deck.id, "a", "b").unwrap();
    drop(other);
    let failure = run(parent.path(), &held, today, None, &never).unwrap_err();
    assert_eq!(failure.kind, FailureKind::InsideDataDir, "{failure}");
    assert_eq!(fs::read_dir(&held).unwrap().count(), 0, "nothing written");
    let dir = open_folder(&env.folder()).unwrap();
    assert!(dir_inside(&dir, env.tmp.path()));
    assert!(!dir_inside(&dir, &data));
}

/// A real "disk full" (`ENOSPC` from the kernel, written to `/dev/full`)
/// in the middle of the export: the error names the file in the backup
/// folder, so it reads as "the disk is full", not as something else.
#[test]
fn a_full_disk_says_so() {
    let env = Env::new();
    // Enough media that the export writes past its first buffer.
    for seed in 0..4 {
        env.store
            .add_media_bytes(&crate::test_support::png_bytes(200 + seed))
            .unwrap();
    }
    let Ok(mut full) = fs::OpenOptions::new().write(true).open("/dev/full") else {
        // No /dev/full in this sandbox: nothing to check.
        return;
    };
    let partial = env
        .folder()
        .join(".Rappel backup 2026-11-09.rappel.0.partial");
    let err = env
        .store
        .export_into(
            &mut full,
            &partial,
            ExportOptions::default(),
            crate::interchange::MAX_ARCHIVE_BYTES,
            T0,
            &AtomicBool::new(false),
        )
        .unwrap_err();
    // Before round 5 the archive writer's error was "cannot write the
    // export" (something unexpected), and a failed write while copying a
    // media file named that media file.
    assert!(
        matches!(&err, Error::Io { path, .. } if *path == partial),
        "{err:?}"
    );
    let failure = Failure::core(&err, &env.folder());
    assert_eq!(failure.kind, FailureKind::NoSpace, "{failure}");
}

/// The folder removed while the backup runs (after the export is written,
/// before it is named): the backup fails as "not found", and nothing is
/// written anywhere else, not even in a new folder of the same name.
#[test]
fn a_folder_removed_while_the_backup_runs_fails_safely() {
    let env = Env::new();
    let folder = env.folder();
    let _hook = crate::test_support::set_hook(&AFTER_EXPORT, {
        let folder = folder.clone();
        move || {
            fs::remove_dir_all(&folder).unwrap();
            fs::create_dir(&folder).unwrap();
        }
    });
    let failure = env.run("2026-11-09", None).unwrap_err();
    assert_eq!(failure.kind, FailureKind::FolderMissing, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
}

/// The folder swapped for a link while the backup runs: the backup goes
/// on in the folder it opened (now under another name), and the link's
/// target gets nothing.
#[test]
fn a_folder_swapped_for_a_link_while_the_backup_runs_writes_nothing_elsewhere() {
    let env = Env::new();
    let folder = env.folder();
    let moved = env.tmp.path().join("moved");
    let target = env.tmp.path().join("target");
    fs::create_dir(&target).unwrap();
    let _hook = crate::test_support::set_hook(&AFTER_EXPORT, {
        let (folder, moved, target) = (folder.clone(), moved.clone(), target.clone());
        move || {
            fs::rename(&folder, &moved).unwrap();
            symlink(&target, &folder).unwrap();
        }
    });
    assert!(matches!(
        env.run("2026-11-09", None),
        Ok(Outcome::Written { .. })
    ));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    let names: Vec<String> = fs::read_dir(&moved)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["Rappel backup 2026-11-09.rappel"]);
}

/// A folder of 20,000 entries: 3,000 old backups (eight years of days),
/// 16,000 look-alikes and 1,000 stale partial files. One backup keeps the
/// 14 newest backups, removes the other 2,987 and the partial files, and
/// leaves every look-alike, quickly.
#[test]
fn a_folder_of_twenty_thousand_files_is_pruned_quickly() {
    let env = Env::new();
    let folder = env.folder();
    let first = date("2018-01-01").days();
    let day = |n: i64| {
        let ms = (first + n) * DAY_MS;
        LocalDate::from_utc_ms(ms, 0).unwrap()
    };
    for n in 0..3000 {
        fs::write(folder.join(file_name(day(n))), b"").unwrap();
    }
    for n in 0..16_000 {
        fs::write(
            folder.join(format!("Rappel backup {}.rappel~{n}", day(n % 3000))),
            b"",
        )
        .unwrap();
    }
    let old = now_seconds() - 2 * STALE_PARTIAL_SECONDS;
    for _ in 0..1000 {
        let name = partial_name(&file_name(day(0))).unwrap();
        fs::write(folder.join(&name), b"").unwrap();
        set_modified(&folder.join(&name), old);
    }
    let (used, outcome) = crate::test_support::cpu_time_of(|| env.run("2026-11-09", None));
    let Ok(Outcome::Written { removed, .. }) = outcome else {
        panic!("written: {outcome:?}");
    };
    assert_eq!(removed, 3000 - (KEEP - 1));
    let names = env.names();
    assert_eq!(names.len(), 16_000 + KEEP);
    assert_eq!(
        names
            .iter()
            .filter(|n| parse_file_name(n).is_some())
            .count(),
        KEEP
    );
    assert!(!names.iter().any(|n| is_partial_name(n)));
    if std::env::var_os("RAPPEL_TIMINGS").is_some() {
        eprintln!("a backup in a folder of 20,000 files: {used:?}");
    }
    crate::test_support::assert_cpu_within(
        used,
        // 41 ms measured in a debug build (ADR 0084).
        std::time::Duration::from_millis(500),
        "a backup in a folder of 20,000 files",
    );
}

/// Odd names in the folder (not UTF-8, a newline, 255 bytes, a backup's
/// name with something after it) are left alone; a folder with an odd
/// name of its own gets its backup.
#[test]
fn odd_names_are_left_alone() {
    use std::os::unix::ffi::OsStrExt;
    let env = Env::new();
    let folder = env.folder();
    let odd: Vec<std::ffi::OsString> = vec![
        std::ffi::OsStr::from_bytes(b"Rappel backup 2020-01-01.rappel\xff").to_owned(),
        std::ffi::OsStr::from_bytes(b"\xffRappel backup 2020-01-01.rappel").to_owned(),
        "Rappel backup 2020-01-01.rappel\n".into(),
        "Rappel backup\n2020-01-01.rappel".into(),
        format!("Rappel backup 2020-01-01.rappel{}", "x".repeat(255 - 31)).into(),
        "x".repeat(255).into(),
        "Rappel backup 2020-01-01.rappel.bak".into(),
    ];
    for name in &odd {
        fs::write(folder.join(name), b"not a backup").unwrap();
    }
    for day in 1..=20 {
        fs::write(
            folder.join(file_name(LocalDate::new(2020, 1, day).unwrap())),
            b"old",
        )
        .unwrap();
    }
    let Ok(Outcome::Written { removed, .. }) = env.run("2026-11-09", None) else {
        panic!("written");
    };
    assert_eq!(removed, 7);
    for name in &odd {
        assert!(folder.join(name).exists(), "{name:?}");
    }
    // A folder whose own name is odd.
    let strange = env.tmp.path().join("Back\nups ü 💾 \u{202e}evil");
    fs::create_dir(&strange).unwrap();
    assert!(matches!(
        run(
            &env.data(),
            &strange,
            date("2026-11-09"),
            None,
            &AtomicBool::new(false)
        ),
        Ok(Outcome::Written { .. })
    ));
    assert!(strange.join("Rappel backup 2026-11-09.rappel").is_file());
}

/// Something that is not a folder in the folder's place: a FIFO (opening
/// it must never block) and a socket.
#[test]
fn a_fifo_or_socket_in_the_folders_place_is_not_a_folder() {
    let env = Env::new();
    let fifo = env.tmp.path().join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        FileType::Fifo,
        Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    let socket = env.tmp.path().join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    for folder in [fifo, socket] {
        let failure = run(
            &env.data(),
            &folder,
            date("2026-11-09"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(failure.kind, FailureKind::NotAFolder, "{failure}");
    }
}

/// A run that fails (here cut off by the cap) removes no partial file,
/// however old: one left by a killed run may hold a whole backup, and only
/// a run that has today's backup in the folder lets it go.
#[test]
fn a_failed_backup_removes_no_partial_file() {
    let env = Env::new();
    let folder = env.folder();
    let old = now_seconds() - 2 * STALE_PARTIAL_SECONDS;
    let name = partial_name(&file_name(date("2026-11-01"))).unwrap();
    fs::write(folder.join(&name), b"maybe a whole backup").unwrap();
    set_modified(&folder.join(&name), old);
    let failure = run(
        &env.data(),
        &folder,
        date("2026-11-09"),
        None,
        &AtomicBool::new(true),
    )
    .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Cancelled, "{failure}");
    assert_eq!(env.names(), vec![name.clone()]);
    // The next run that writes today's backup removes it.
    assert!(matches!(
        env.run("2026-11-09", None),
        Ok(Outcome::Written { .. })
    ));
    assert_eq!(env.names(), vec!["Rappel backup 2026-11-09.rappel"]);
}
