//! The memory an import needs at its peak, measured (ADR 0025).
//!
//! A test binary of its own: the peak is the process's (`VmHWM` in
//! `/proc/self/status`), so no other test may run beside it. Linux only.
//!
//! Planning an import holds the archive, then the decompressed
//! `cards.json`, then the parsed document. The budget is 1 GiB at the
//! limits (256 MiB of JSON), so the peak may be at most 4 bytes per byte
//! of JSON. The worst case per byte is many small rows: review events.

#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rappel_core::storage::Store;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The JSON size limit of an import (`Limits::DEFAULT.json`).
const JSON_LIMIT: u64 = 256 << 20;
/// The memory budget at that limit.
const BUDGET: u64 = 1 << 30;
/// The size measured: an eighth of the limit, to keep the test quick.
const MEASURED_JSON: u64 = JSON_LIMIT / 8;

/// A valid lowercase `UUIDv7` for number `n`.
fn id(n: u64) -> String {
    format!("01920000-0000-7000-8000-{n:012x}")
}

/// What fills a test file once its cards are written.
#[derive(Clone, Copy)]
enum Rows {
    /// Review events.
    Reviews,
    /// Pairs of a reset and the undo that cancels it (ADR 0028).
    ResetUndo,
    /// Card tags, over `TAGS` tags (ADR 0029): the smallest rows.
    CardTags,
}

/// How many tags a `Rows::CardTags` file has.
const TAGS: u64 = 1_000;

/// Writes a `.rappel` file whose `cards.json` is about `json_bytes` long:
/// one deck, `cards` cards with `text` on the front, and `rows` for the
/// rest. Streamed, so building it costs no memory to speak of. Returns the
/// JSON's length.
fn write_export(
    path: &Path,
    json_bytes: u64,
    cards: u64,
    text: &str,
    stored: bool,
    rows: Rows,
) -> zip::result::ZipResult<u64> {
    const T: i64 = 1_792_067_696_789;
    let file = BufWriter::new(File::create(path)?);
    let mut zip = ZipWriter::new(file);
    let method = if stored {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    zip.start_file(
        "cards.json",
        SimpleFileOptions::DEFAULT.compression_method(method),
    )?;
    let mut json = Counting(&mut zip, 0);
    write!(
        json,
        r#"{{"format_version":2,"exported_at":{T},"decks":[{{"id":"{}","name":"Deck","created_at":{T},"updated_at":{T},"deleted_at":null}}],"cards":["#,
        id(0)
    )?;
    for c in 1..=cards {
        let comma = if c == 1 { "" } else { "," };
        write!(
            json,
            r#"{comma}{{"id":"{}","deck_id":"{}","front_md":"{text}","back_md":"","created_at":{T},"updated_at":{T},"deleted_at":null}}"#,
            id(c),
            id(0)
        )?;
    }
    json.write_all(br#"],"tags":["#)?;
    // Tag ids come after every card id, and are sorted.
    let tag = |t: u64| id(cards + 1 + t);
    if matches!(rows, Rows::CardTags) {
        for t in 0..TAGS {
            let comma = if t == 0 { "" } else { "," };
            write!(json, r#"{comma}{{"id":"{}","name":"tag {t}"}}"#, tag(t))?;
        }
    }
    json.write_all(br#"],"card_tags":["#)?;
    if matches!(rows, Rows::CardTags) {
        // By card, then tag, as an export writes them.
        'cards: for c in 1..=cards {
            for t in 0..TAGS {
                if json.1 >= json_bytes {
                    break 'cards;
                }
                let comma = if c == 1 && t == 0 { "" } else { "," };
                write!(
                    json,
                    r#"{comma}{{"card_id":"{}","tag_id":"{}"}}"#,
                    id(c),
                    tag(t)
                )?;
            }
        }
    }
    json.write_all(br#"],"review_events":["#)?;
    let mut n = cards + 1 + TAGS;
    let mut first = true;
    while !matches!(rows, Rows::CardTags) && json.1 < json_bytes {
        let comma = if first { "" } else { "," };
        first = false;
        let card = id(1 + n % cards);
        let at = T + i64::try_from(n).map_err(io::Error::other)?;
        match rows {
            Rows::Reviews => write!(
                json,
                r#"{comma}{{"id":"{}","card_id":"{card}","kind":"review","reviewed_at":{at},"grade":"pass","duration_ms":1200,"undoes":null}}"#,
                id(n)
            )?,
            Rows::ResetUndo | Rows::CardTags => write!(
                json,
                r#"{comma}{{"id":"{}","card_id":"{card}","kind":"reset","reviewed_at":{at},"grade":null,"duration_ms":0,"undoes":null}},{{"id":"{}","card_id":"{card}","kind":"undo","reviewed_at":{at},"grade":null,"duration_ms":0,"undoes":"{}"}}"#,
                id(n),
                id(n + 1),
                id(n)
            )?,
        }
        n += 2;
    }
    json.write_all(br#"],"settings":{},"media":[]}"#)?;
    let len = json.1;
    zip.add_directory("media/", SimpleFileOptions::DEFAULT)?;
    zip.finish()?.flush()?;
    Ok(len)
}

/// A writer that counts what goes through it.
struct Counting<'a, W: Write>(&'a mut W, u64);

impl<W: Write> Write for Counting<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.0.write(buf)?;
        self.1 += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// A field of `/proc/self/status`, in bytes.
fn status_bytes(field: &str) -> io::Result<u64> {
    let status = fs::read_to_string("/proc/self/status")?;
    let kib = status
        .lines()
        .find_map(|l| l.strip_prefix(field))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        .ok_or_else(|| io::Error::other(format!("no {field} in /proc/self/status")))?;
    Ok(kib * 1024)
}

/// The peak memory of `f` above what the process held before it, in
/// bytes: the peak resident set is reset to the current one first.
fn peak_of<T>(f: impl FnOnce() -> T) -> io::Result<(T, u64)> {
    // Resets VmHWM to the current resident size (proc(5)). Fails in some
    // sandboxes; the test then fails rather than measuring nothing.
    fs::write("/proc/self/clear_refs", "5")?;
    let before = status_bytes("VmRSS:")?;
    let value = f();
    Ok((value, status_bytes("VmHWM:")?.saturating_sub(before)))
}

#[test]
fn planning_an_import_stays_within_the_memory_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("data")).unwrap();
    // Worst first: memory freed by a case but kept by the allocator is
    // counted in the next case's baseline and reused, which can hide some
    // of that case's peak.
    let cases = [
        // Many small rows, stored: the archive is as large as the JSON,
        // the worst case per byte. Measured each first: reviews 3.75
        // bytes per JSON byte, resets and undos about the same, card tags
        // less (their rows are smaller, but they hold no set of pairs).
        ("review events, stored", 5_000, "q", true, Rows::Reviews),
        (
            "resets and undos, stored",
            5_000,
            "q",
            true,
            Rows::ResetUndo,
        ),
        ("card tags, stored", 5_000, "q", true, Rows::CardTags),
        // Deflated.
        ("review events, deflated", 5_000, "q", false, Rows::Reviews),
        // Long card sides: few, large strings.
        (
            "long cards, deflated",
            4_000,
            &*"x".repeat(8_000),
            false,
            Rows::Reviews,
        ),
    ];
    for (what, cards, text, stored, rows) in cases {
        let path = tmp.path().join("big.rappel");
        let json = write_export(&path, MEASURED_JSON, cards, text, stored, rows).unwrap();
        let (plan, peak) = peak_of(|| store.plan_import(&path)).unwrap();
        let plan = plan.unwrap();
        assert!(plan.summary().decks.new == 1, "{what}");
        drop(plan);
        fs::remove_file(&path).unwrap();
        let at_limit = peak.saturating_mul(JSON_LIMIT / MEASURED_JSON);
        eprintln!(
            "{what}: {} MiB JSON, peak {} MiB ({}.{:02} bytes per JSON byte, {} MiB extrapolated to the limit)",
            json >> 20,
            peak >> 20,
            peak / json,
            peak * 100 / json % 100,
            at_limit >> 20
        );
        assert!(
            at_limit <= BUDGET,
            "{what}: {} MiB at the limit is over the {} MiB budget",
            at_limit >> 20,
            BUDGET >> 20
        );
    }
}
