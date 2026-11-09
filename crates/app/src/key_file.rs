//! Small key files in the data directory: `window-state.ini` (ADR 0039)
//! and `auto-backup.ini` (ADR 0080). They are read as untrusted files and
//! written in one atomic step, only for the user.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use gtk::glib;

/// The files' mode, like the database's: only the user reads them.
const FILE_MODE_BITS: u32 = 0o600;
const FILE_MODE: i32 = 0o600;

/// Why a file was not read.
#[derive(Debug)]
pub enum Unread {
    /// There is no file (the first run).
    Missing,
    /// Something else: logged at debug level.
    Refused(String),
}

/// The file's bytes, if it is a regular file of at most `max_bytes`. The
/// file is untrusted (ADR 0050, row 33). It is opened with `O_NOFOLLOW`,
/// so a symbolic link is refused (`ELOOP`) rather than followed, and
/// `O_NONBLOCK`, so opening a pipe returns at once instead of waiting for a
/// writer (open(2)). What was opened is then checked on the open file
/// itself, so there is no gap between a check and the open: anything but a
/// regular file (a pipe, a device, a folder) is refused before a byte is
/// read, and the read stops one byte past the limit. `O_NONBLOCK` changes
/// nothing for a regular file.
pub fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Unread> {
    use std::os::unix::fs::OpenOptionsExt;
    let refuse = |what: &str| Unread::Refused(what.to_owned());
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(Unread::Missing),
        Err(err) => return Err(Unread::Refused(err.to_string())),
    };
    let meta = file
        .metadata()
        .map_err(|err| Unread::Refused(err.to_string()))?;
    if !meta.file_type().is_file() {
        return Err(refuse("not a regular file"));
    }
    if meta.len() > max_bytes {
        return Err(refuse("too large"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| Unread::Refused(err.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(refuse("too large"));
    }
    Ok(bytes)
}

/// The key file in `bytes`, or why not. Bytes that are not UTF-8 or hold a
/// NUL are refused: no key file has them, and `g_key_file_load_from_data`
/// takes a C string (the binding would panic on a NUL).
pub fn parse(bytes: &[u8]) -> Result<glib::KeyFile, String> {
    let text = std::str::from_utf8(bytes).map_err(|err| err.to_string())?;
    if text.contains('\0') {
        return Err("holds a NUL".to_owned());
    }
    let file = glib::KeyFile::new();
    file.load_from_data(text, glib::KeyFileFlags::NONE)
        .map_err(|err| err.to_string())?;
    Ok(file)
}

/// Writes `data` to `path`. `g_file_set_contents_full` with
/// `G_FILE_SET_CONTENTS_CONSISTENT` writes a temporary file created with
/// `mode` (less the umask) and renames it over the old one (`GLib` docs), so
/// a crash leaves the old file or the new one, and a new file is never there
/// with a wider mode than 0600, like the database. An existing file keeps
/// its mode (same docs), so a wider one is narrowed to 0600 afterwards, as
/// the database's files are (`prepare_private_file` in core).
///
/// # Errors
/// The write's error; a failure to narrow the mode is only logged.
pub fn write_private(path: &Path, data: &str) -> Result<(), glib::Error> {
    glib::file_set_contents_full(
        path,
        data.as_bytes(),
        glib::FileSetContentsFlags::CONSISTENT,
        FILE_MODE,
    )?;
    match std::fs::metadata(path) {
        Ok(meta) if meta.permissions().mode() & 0o7777 != FILE_MODE_BITS => {
            if let Err(err) =
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(FILE_MODE_BITS))
            {
                tracing::warn!("a key file's permissions could not be set: {err}");
            }
        }
        Ok(_) => {}
        Err(err) => tracing::warn!("a key file's permissions could not be read: {err}"),
    }
    Ok(())
}
