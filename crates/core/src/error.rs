//! Error type shared by all of `cards-core`.

use std::io;
use std::path::PathBuf;

/// Everything that can go wrong in `cards-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No usable base directory: `CARDS_DATA_DIR` and `XDG_DATA_HOME` are
    /// unset (or unusable) and `HOME` is unset, empty or relative.
    #[error("cannot find the data directory: set HOME, XDG_DATA_HOME or CARDS_DATA_DIR")]
    NoHomeDir,

    /// `CARDS_DATA_DIR` is a relative path. It is rejected instead of being
    /// resolved against whatever the current directory happens to be.
    #[error("CARDS_DATA_DIR must be an absolute path, got {}", .0.display())]
    RelativeDataDir(PathBuf),

    /// A debug build resolved to the release data directory (or a directory
    /// inside it), which would let development touch real data.
    #[error(
        "debug builds must not use the release data directory: {} is {} or inside it",
        path.display(),
        release.display()
    )]
    DebugUsesReleaseDir {
        /// The directory the debug build would have used.
        path: PathBuf,
        /// The release data directory it collides with.
        release: PathBuf,
    },

    /// The data directory path exists but is not a directory.
    #[error("{} exists but is not a directory", .0.display())]
    NotADirectory(PathBuf),

    /// Creating the data directory or setting its permissions failed.
    #[error("cannot create data directory {}: {source}", path.display())]
    CreateDataDir {
        /// The directory that could not be created.
        path: PathBuf,
        /// The underlying IO error.
        source: io::Error,
    },
}

/// `Result` with [`Error`] as the error type.
pub type Result<T> = std::result::Result<T, Error>;
