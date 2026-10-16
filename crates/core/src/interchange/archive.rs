//! Writing and reading the `.cards` zip archive.
//!
//! The archive holds exactly `cards.json` and an empty `media/` directory.
//! Nothing is ever extracted to disk: `cards.json` is read into memory by
//! its exact name, and any other entry refuses the file, so paths in entry
//! names (zip-slip) cannot matter.

use std::io::{Cursor, Read, Write};

use zip::result::ZipResult;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::{ImportProblem, Limits};

/// The JSON document's entry name.
pub(super) const JSON_ENTRY: &str = "cards.json";
/// The media directory's entry name (empty in format version 1).
pub(super) const MEDIA_DIR: &str = "media/";

/// Length of the end of central directory record without a comment.
const EOCD_LEN: usize = 22;
const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";

/// The archive bytes for `json`. Deterministic: fixed entry order, the
/// zip format's earliest timestamp (1980-01-01) instead of the clock,
/// fixed permissions, no extra fields and no comment.
pub(super) fn write_archive(json: &[u8]) -> ZipResult<Vec<u8>> {
    // `DEFAULT` (not `default()`) has the fixed 1980 timestamp.
    let file = SimpleFileOptions::DEFAULT
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);
    let dir = SimpleFileOptions::DEFAULT.unix_permissions(0o700);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(JSON_ENTRY, file)?;
    zip.write_all(json)?;
    zip.add_directory(MEDIA_DIR, dir)?;
    Ok(zip.finish()?.into_inner())
}

/// The decompressed `cards.json` of the archive `bytes`, after checking
/// that the archive holds exactly the expected entries.
pub(super) fn read_cards_json(bytes: &[u8], limits: &Limits) -> Result<Vec<u8>, ImportProblem> {
    let declared = declared_entries(bytes)?;
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| invalid(&e))?;
    // The reader keys entries by name, so a repeated name would silently
    // hide one of them: compare with the count the archive declares.
    if zip.len() != declared {
        return Err(ImportProblem::UnexpectedEntry(
            "(a name used twice)".to_owned(),
        ));
    }

    let mut json_index = None;
    let mut media_seen = false;
    for i in 0..zip.len() {
        // Raw: only the metadata is looked at, nothing is decompressed.
        let entry = zip.by_index_raw(i).map_err(|e| invalid(&e))?;
        match entry.name_raw() {
            name if name == JSON_ENTRY.as_bytes() => {
                let stored_ok = matches!(
                    entry.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                );
                if entry.encrypted() || !entry.is_file() || !stored_ok {
                    return Err(ImportProblem::UnsupportedEntry(JSON_ENTRY));
                }
                json_index = Some(i);
            }
            name if name == MEDIA_DIR.as_bytes() => {
                if entry.encrypted() || !entry.is_dir() || entry.size() != 0 {
                    return Err(ImportProblem::UnsupportedEntry(MEDIA_DIR));
                }
                media_seen = true;
            }
            other => return Err(ImportProblem::UnexpectedEntry(shown_name(other))),
        }
    }
    let json_index = json_index.ok_or(ImportProblem::MissingEntry(JSON_ENTRY))?;
    if !media_seen {
        return Err(ImportProblem::MissingEntry(MEDIA_DIR));
    }

    // The declared size is not trusted: reading stops one byte past the
    // limit, whatever the entry claims, so a zip bomb is cut off. The zip
    // reader checks the CRC once the entry is read to its end.
    let entry = zip.by_index(json_index).map_err(|e| invalid(&e))?;
    let mut json = Vec::new();
    entry
        .take(limits.json.saturating_add(1))
        .read_to_end(&mut json)
        .map_err(|e| ImportProblem::InvalidArchive(e.to_string()))?;
    if json.len() as u64 > limits.json {
        return Err(ImportProblem::EntryTooLarge {
            entry: JSON_ENTRY,
            limit: limits.json,
        });
    }
    Ok(json)
}

/// The number of entries the archive's end record declares. The record
/// must end the file (no archive comment, which Cards never writes) and
/// describe a single-disk archive without zip64, so it cannot be confused
/// with bytes inside an entry.
fn declared_entries(bytes: &[u8]) -> Result<usize, ImportProblem> {
    let not_cards = || ImportProblem::InvalidArchive("no end of archive record".to_owned());
    let start = bytes.len().checked_sub(EOCD_LEN).ok_or_else(not_cards)?;
    let eocd = &bytes[start..];
    let u16_at = |i: usize| u16::from_le_bytes([eocd[i], eocd[i + 1]]);
    let (disk, cd_disk, on_disk, total, comment) =
        (u16_at(4), u16_at(6), u16_at(8), u16_at(10), u16_at(20));
    if eocd[..4] != EOCD_SIGNATURE
        || disk != 0
        || cd_disk != 0
        || on_disk != total
        || total == u16::MAX
        || comment != 0
    {
        return Err(not_cards());
    }
    Ok(usize::from(total))
}

fn invalid(err: &zip::result::ZipError) -> ImportProblem {
    ImportProblem::InvalidArchive(err.to_string())
}

/// An entry name for a message: escaped and at most 64 characters, since
/// it comes from the untrusted file.
fn shown_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    let mut shown: String = name
        .chars()
        .take(64)
        .collect::<String>()
        .escape_debug()
        .to_string();
    if name.chars().count() > 64 {
        shown.push('…');
    }
    format!("\"{shown}\"")
}
