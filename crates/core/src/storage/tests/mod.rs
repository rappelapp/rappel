//! Tests for `Store::open`: fresh database, re-open, migrations, backups,
//! integrity failures and the append-only rule.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::*;
use crate::domain::{CardState, Phase, Schedule};
use crate::scheduling::Scheduler;
use crate::test_support::with_suffix;

pub(super) use crate::test_support::{DAY_MS, T0 as DAY0};

pub(super) const DECK: &str = "01920000-0000-7000-8000-000000000001";
pub(super) const CARD: &str = "01920000-0000-7000-8000-000000000002";
pub(super) const CARD2: &str = "01920000-0000-7000-8000-000000000003";
const EVENT: &str = "01920000-0000-7000-8000-000000000004";

struct Env {
    _tmp: tempfile::TempDir,
    db: PathBuf,
    backups: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("data").join(DB_FILE_NAME);
    let backups = tmp.path().join("data").join(BACKUP_DIR_NAME);
    Env {
        _tmp: tmp,
        db,
        backups,
    }
}

impl Env {
    fn open(&self, now: i64) -> Result<Store> {
        Store::open_with(&self.db, &self.backups, now, MIGRATIONS)
    }

    fn open_with(&self, now: i64, migrations: &[Migration]) -> Result<Store> {
        Store::open_with(&self.db, &self.backups, now, migrations)
    }

    fn backups(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.backups) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

pub(super) fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub(super) fn insert_sample(conn: &Connection) {
    conn.execute_batch(&format!(
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('{DECK}', 'Maths', 'maths', 1, 1);
         INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
             VALUES ('{CARD}', '{DECK}', 'front', 'back', 1, 1);
         INSERT INTO review_events VALUES ('{EVENT}', '{CARD}', 2, 'pass', 1500);
         INSERT INTO card_schedule VALUES ('{CARD}', 'learning', 3, 1.5, 5.0, 2, 1, 0);"
    ))
    .unwrap();
}

pub(super) fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub(super) const FAKE_V1: Migration = Migration {
    version: 1,
    name: "fake v1",
    sql: "CREATE TABLE t(x INTEGER NOT NULL) STRICT;",
    rebuild_schedule: false,
    backfill: None,
};

pub(super) const FAKE_V2: Migration = Migration {
    version: 2,
    name: "fake v2",
    sql: "ALTER TABLE t ADD COLUMN y INTEGER NOT NULL DEFAULT 7;",
    rebuild_schedule: false,
    backfill: None,
};

pub(super) const FAILING_V2: Migration = Migration {
    version: 2,
    name: "failing v2",
    sql: "CREATE TABLE u(x INTEGER) STRICT; INSERT INTO missing VALUES (1);",
    rebuild_schedule: false,
    backfill: None,
};

fn db_at_fake_v1(e: &Env) {
    let store = e.open_with(DAY0, &[FAKE_V1]).unwrap();
    store
        .conn
        .execute_batch("INSERT INTO t VALUES (1), (2);")
        .unwrap();
}

/// Overwrites the root page of a btree with garbage, the way a bad disk
/// sector or a torn write would.
pub(super) fn corrupt_btree(db: &Path, name: &str) {
    let (root, page_size): (i64, i64) = {
        let conn = Connection::open(db).unwrap();
        let root = conn
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        let page_size = conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .unwrap();
        (root, page_size)
    };
    let mut bytes = fs::read(db).unwrap();
    let start = usize::try_from((root - 1) * page_size).unwrap();
    let end = start + usize::try_from(page_size).unwrap();
    bytes[start..end].fill(0xA5);
    fs::write(db, bytes).unwrap();
}

/// Inserts review events for `card` at `DAY0 + offsets`, ids from `first_id`.
pub(super) fn insert_events(conn: &Connection, card: &str, first_id: u32, reviews: &[(i64, &str)]) {
    for (i, (offset, grade)) in (first_id..).zip(reviews) {
        conn.execute(
            "INSERT INTO review_events VALUES (?1, ?2, ?3, ?4, 1000)",
            rusqlite::params![
                format!("01920000-0000-7000-8000-1{i:011}"),
                card,
                DAY0 + offset,
                grade
            ],
        )
        .unwrap();
    }
}

pub(super) const REVIEWS: [(i64, &str); 4] = [
    (0, "fail"),
    (STEP, "pass"),
    (3 * DAY_MS, "fail"),
    (3 * DAY_MS + STEP, "pass"),
];

const STEP: i64 = crate::scheduling::STEP_MS;

mod cache;
mod fresh;
mod integrity;
mod migrations;
mod perf;
mod reopen;
mod schema_rules;
