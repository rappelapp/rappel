use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

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
    // Dated after today (flown west, or the clock put back): not twice.
    assert!(!due(Some(&last), folder, date("2026-11-08")));
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
        run(&self.data(), &self.folder(), date(today), last)
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
    env.store
        .add_media_bytes(&crate::test_support::png_bytes(3))
        .unwrap();
    step(&env, "a media file");
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
    // A link of that name: kept, and what it points to is untouched.
    let elsewhere = env.tmp.path().join("elsewhere");
    fs::write(&elsewhere, b"precious").unwrap();
    symlink(&elsewhere, &today).unwrap();
    assert_eq!(env.run("2026-11-09", None), Ok(Outcome::AlreadyThere));
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
    let gone = env.tmp.path().join("gone");
    let failure = run(&env.data(), &gone, today, None).unwrap_err();
    assert_eq!(failure.kind, FailureKind::FolderMissing, "{failure}");

    let file = env.tmp.path().join("a file");
    fs::write(&file, b"x").unwrap();
    let failure = run(&env.data(), &file, today, None).unwrap_err();
    assert_eq!(failure.kind, FailureKind::NotAFolder, "{failure}");

    // A link in place of the folder is refused, and nothing is written
    // where it points.
    let link = env.tmp.path().join("link");
    symlink(env.folder(), &link).unwrap();
    let failure = run(&env.data(), &link, today, None).unwrap_err();
    assert_eq!(failure.kind, FailureKind::FolderIsLink, "{failure}");
    assert_eq!(env.names(), Vec::<String>::new());
    // A link higher up the path is the user's own layout and is followed.
    let parent_link = env.tmp.path().join("parent link");
    symlink(env.tmp.path(), &parent_link).unwrap();
    assert!(matches!(
        run(&env.data(), &parent_link.join("backups"), today, None),
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
    assert_eq!(
        Failure::core(&Error::ExportTooLarge { bytes: 2, limit: 1 }).kind,
        FailureKind::TooLarge
    );
    assert_eq!(
        Failure::core(&Error::Io {
            path: PathBuf::from("/x"),
            source: io::Error::from(Errno::NOSPC),
        })
        .kind,
        FailureKind::NoSpace
    );
    assert_eq!(Failure::core(&Error::IdsExhausted).kind, FailureKind::Other);
}
