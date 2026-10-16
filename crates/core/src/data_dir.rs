//! Where Rappel keeps its data, and creating that directory safely.
//!
//! Resolution is a pure function of its inputs ([`DataDirEnv`] and
//! [`BuildProfile`]) so every case can be tested without touching the real
//! environment. See ADR 0004 for the reasoning.

use std::fs::{self, DirBuilder, Permissions};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Directory name under the XDG data home for release builds.
pub(crate) const RELEASE_DIR_NAME: &str = "rappel";

/// Directory name release builds used before the rename to Rappel. Nothing
/// reads it any more (the data is moved with export and import, ADR 0020),
/// but debug builds still refuse it, so development cannot touch data that
/// has not been moved yet.
pub const LEGACY_RELEASE_DIR_NAME: &str = "cards";

/// Directory name under the XDG data home for debug builds, so development
/// never touches real data.
pub(crate) const DEBUG_DIR_NAME: &str = "rappel-dev";

/// Permission bits for the data directory: owner only.
const DIR_MODE: u32 = 0o700;

/// Which kind of build is running. The app passes this in (from
/// `cfg!(debug_assertions)`) so both cases are testable from either build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// Development build: uses `DEBUG_DIR_NAME`.
    Debug,
    /// Release build: uses `RELEASE_DIR_NAME`.
    Release,
}

/// The environment variables that decide the data directory. Unset and empty
/// values are both `None`-like: an empty variable is treated as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataDirEnv {
    /// `RAPPEL_DATA_DIR`: explicit override.
    pub rappel_data_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`: base directory for user data.
    pub xdg_data_home: Option<PathBuf>,
    /// `HOME`: used for the XDG default `$HOME/.local/share`.
    pub home: Option<PathBuf>,
}

impl DataDirEnv {
    /// Reads the variables from the current process environment. This is the
    /// only impure part of data directory resolution.
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            rappel_data_dir: var("RAPPEL_DATA_DIR"),
            xdg_data_home: var("XDG_DATA_HOME"),
            home: var("HOME"),
        }
    }
}

/// Resolves the data directory. Pure: reads nothing from the system.
///
/// Priority:
/// 1. `RAPPEL_DATA_DIR` if set and non-empty. It must be absolute.
/// 2. Otherwise `$XDG_DATA_HOME` if set, non-empty and absolute (the XDG
///    spec says relative values are invalid and must be ignored), else
///    `$HOME/.local/share`; joined with `rappel-dev` for
///    [`BuildProfile::Debug`] or `rappel` for [`BuildProfile::Release`].
///
/// A debug build refuses any result that is, or is inside, a release data
/// directory (`$XDG_DATA_HOME/rappel` or `$HOME/.local/share/rappel`, or the
/// same under the pre-rename name `cards`). Paths
/// are compared after removing `.` and `..` lexically; symlinks are not
/// resolved.
///
/// # Errors
/// - [`Error::RelativeDataDir`] if `RAPPEL_DATA_DIR` is relative.
/// - [`Error::NoHomeDir`] if neither a usable `XDG_DATA_HOME` nor an
///   absolute `HOME` is available.
/// - [`Error::DebugUsesReleaseDir`] for the debug guard above.
pub fn resolve_data_dir(env: &DataDirEnv, profile: BuildProfile) -> Result<PathBuf> {
    let path = if let Some(dir) = non_empty(env.rappel_data_dir.as_deref()) {
        if !dir.is_absolute() {
            return Err(Error::RelativeDataDir(dir.to_path_buf()));
        }
        normalize(dir)
    } else {
        let name = match profile {
            BuildProfile::Debug => DEBUG_DIR_NAME,
            BuildProfile::Release => RELEASE_DIR_NAME,
        };
        data_home(env).ok_or(Error::NoHomeDir)?.join(name)
    };

    if profile == BuildProfile::Debug {
        for release in release_dirs(env) {
            if path.starts_with(&release) {
                return Err(Error::DebugUsesReleaseDir { path, release });
            }
        }
    }
    Ok(path)
}

/// Creates the data directory (and missing parents) with mode `0700`.
///
/// If the directory already exists with other permission bits, they are set
/// to `0700`. Missing parents are created with `0700` minus the process
/// umask (normally exactly `0700`, as the XDG spec recommends for the data
/// home); only the final directory is corrected. Symlinks are followed: a
/// symlinked data directory gets its target's permissions set. Calling it
/// again is harmless.
///
/// # Errors
/// - [`Error::NotADirectory`] if `path` exists and is not a directory.
/// - [`Error::CreateDir`] if creating it or setting permissions fails.
pub fn ensure_data_dir(path: &Path) -> Result<()> {
    let create_err = |source| Error::CreateDir {
        path: path.to_path_buf(),
        source,
    };

    // Checked first so an existing file gets a clear error instead of the
    // generic one from `create`.
    if let Ok(meta) = fs::metadata(path)
        && !meta.is_dir()
    {
        return Err(Error::NotADirectory(path.to_path_buf()));
    }

    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
        .map_err(create_err)?;

    // `mode` above is reduced by the umask and does not apply to a directory
    // that already existed, so check and fix the final directory explicitly.
    let meta = fs::metadata(path).map_err(create_err)?;
    if !meta.is_dir() {
        return Err(Error::NotADirectory(path.to_path_buf()));
    }
    if meta.permissions().mode() & 0o7777 != DIR_MODE {
        fs::set_permissions(path, Permissions::from_mode(DIR_MODE)).map_err(create_err)?;
    }
    Ok(())
}

/// `$XDG_DATA_HOME` if usable, else `$HOME/.local/share` if `HOME` is
/// absolute. Normalised.
fn data_home(env: &DataDirEnv) -> Option<PathBuf> {
    if let Some(xdg) = non_empty(env.xdg_data_home.as_deref())
        && xdg.is_absolute()
    {
        return Some(normalize(xdg));
    }
    home_data_home(env)
}

/// `$HOME/.local/share`, if `HOME` is set and absolute. Normalised.
fn home_data_home(env: &DataDirEnv) -> Option<PathBuf> {
    non_empty(env.home.as_deref())
        .filter(|home| home.is_absolute())
        .map(|home| normalize(home).join(".local/share"))
}

/// Every directory a release build could use with this environment, or
/// would use by default without `XDG_DATA_HOME`, including the one it used
/// before the rename ([`LEGACY_RELEASE_DIR_NAME`]).
fn release_dirs(env: &DataDirEnv) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = [data_home(env), home_data_home(env)]
        .into_iter()
        .flatten()
        .collect();
    bases.dedup();
    bases
        .into_iter()
        .flat_map(|base| {
            [
                base.join(RELEASE_DIR_NAME),
                base.join(LEGACY_RELEASE_DIR_NAME),
            ]
        })
        .collect()
}

fn non_empty(path: Option<&Path>) -> Option<&Path> {
    path.filter(|p| !p.as_os_str().is_empty())
}

/// Removes `.` and resolves `..` lexically (without looking at the
/// filesystem) in an absolute path. `..` at the root stays at the root, as
/// the kernel does.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use BuildProfile::{Debug, Release};

    fn env(explicit: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> DataDirEnv {
        let p = |v: Option<&str>| v.map(PathBuf::from);
        DataDirEnv {
            rappel_data_dir: p(explicit),
            xdg_data_home: p(xdg),
            home: p(home),
        }
    }

    fn resolve(e: &DataDirEnv, profile: BuildProfile) -> PathBuf {
        resolve_data_dir(e, profile).unwrap()
    }

    // --- Priority ---------------------------------------------------------

    #[test]
    fn home_default_per_profile() {
        let e = env(None, None, Some("/home/u"));
        assert_eq!(
            resolve(&e, Debug),
            Path::new("/home/u/.local/share/rappel-dev")
        );
        assert_eq!(
            resolve(&e, Release),
            Path::new("/home/u/.local/share/rappel")
        );
    }

    #[test]
    fn xdg_data_home_wins_over_home() {
        let e = env(None, Some("/data"), Some("/home/u"));
        assert_eq!(resolve(&e, Debug), Path::new("/data/rappel-dev"));
        assert_eq!(resolve(&e, Release), Path::new("/data/rappel"));
    }

    #[test]
    fn rappel_data_dir_wins_over_everything() {
        let e = env(Some("/custom/place"), Some("/data"), Some("/home/u"));
        assert_eq!(resolve(&e, Debug), Path::new("/custom/place"));
        assert_eq!(resolve(&e, Release), Path::new("/custom/place"));
    }

    #[test]
    fn rappel_data_dir_works_without_home() {
        let e = env(Some("/custom"), None, None);
        assert_eq!(resolve(&e, Release), Path::new("/custom"));
    }

    #[test]
    fn debug_and_release_never_share_a_default() {
        for e in [
            env(None, None, Some("/home/u")),
            env(None, Some("/data"), Some("/home/u")),
            env(None, Some("/data"), None),
        ] {
            assert_ne!(resolve(&e, Debug), resolve(&e, Release), "{e:?}");
        }
    }

    // --- Empty and invalid values -------------------------------------------

    #[test]
    fn empty_variables_are_treated_as_unset() {
        let e = env(Some(""), Some(""), Some("/home/u"));
        assert_eq!(
            resolve(&e, Release),
            Path::new("/home/u/.local/share/rappel")
        );
    }

    #[test]
    fn relative_xdg_data_home_is_ignored() {
        let e = env(None, Some("data"), Some("/home/u"));
        assert_eq!(
            resolve(&e, Release),
            Path::new("/home/u/.local/share/rappel")
        );
    }

    #[test]
    fn relative_rappel_data_dir_is_rejected() {
        let e = env(Some("./here"), None, Some("/home/u"));
        for profile in [Debug, Release] {
            match resolve_data_dir(&e, profile) {
                Err(Error::RelativeDataDir(p)) => assert_eq!(p, Path::new("./here")),
                other => panic!("expected RelativeDataDir, got {other:?}"),
            }
        }
    }

    #[test]
    fn no_home_is_an_error() {
        for e in [
            env(None, None, None),
            env(None, None, Some("")),
            env(None, None, Some("relative/home")),
            env(None, Some("relative"), None),
        ] {
            assert!(
                matches!(resolve_data_dir(&e, Release), Err(Error::NoHomeDir)),
                "{e:?}"
            );
        }
    }

    #[test]
    fn paths_are_normalised() {
        let e = env(Some("/a/./b/../c/"), None, None);
        assert_eq!(resolve(&e, Release), Path::new("/a/c"));
        let e = env(None, Some("/x/../data/"), None);
        assert_eq!(resolve(&e, Release), Path::new("/data/rappel"));
    }

    // --- Debug guard ------------------------------------------------------

    fn assert_guarded(e: &DataDirEnv) {
        assert!(
            matches!(
                resolve_data_dir(e, Debug),
                Err(Error::DebugUsesReleaseDir { .. })
            ),
            "{e:?}"
        );
    }

    #[test]
    fn debug_refuses_override_equal_to_release_dir() {
        assert_guarded(&env(
            Some("/home/u/.local/share/rappel"),
            None,
            Some("/home/u"),
        ));
    }

    #[test]
    fn debug_refuses_override_inside_release_dir() {
        assert_guarded(&env(
            Some("/home/u/.local/share/rappel/sub"),
            None,
            Some("/home/u"),
        ));
    }

    #[test]
    fn debug_refuses_the_pre_rename_release_dir() {
        let home = Some("/home/u");
        assert_guarded(&env(Some("/home/u/.local/share/cards"), None, home));
        assert_guarded(&env(Some("/home/u/.local/share/cards/backups"), None, home));
        assert_guarded(&env(Some("/data/cards"), Some("/data"), home));
        // XDG_DATA_HOME inside the old directory, no override.
        assert_guarded(&env(None, Some("/home/u/.local/share/cards"), home));
        // Also reached through the default $HOME location when XDG_DATA_HOME
        // points elsewhere.
        assert_guarded(&env(
            Some("/home/u/.local/share/cards"),
            Some("/data"),
            home,
        ));
    }

    #[test]
    fn debug_allows_names_that_only_start_like_the_pre_rename_dir() {
        let e = env(
            Some("/home/u/.local/share/cards-dev"),
            None,
            Some("/home/u"),
        );
        assert_eq!(
            resolve(&e, Debug),
            Path::new("/home/u/.local/share/cards-dev")
        );
    }

    #[test]
    fn release_may_still_be_pointed_at_the_pre_rename_dir() {
        // The guard is for debug builds only; an explicit override is the
        // user's choice in a release build.
        let e = env(Some("/home/u/.local/share/cards"), None, Some("/home/u"));
        assert_eq!(
            resolve(&e, Release),
            Path::new("/home/u/.local/share/cards")
        );
    }

    #[test]
    fn debug_guard_ignores_trailing_slash_dot_and_dotdot() {
        let home = Some("/home/u");
        assert_guarded(&env(Some("/home/u/.local/share/rappel/"), None, home));
        assert_guarded(&env(Some("/home/u/./.local/share/rappel"), None, home));
        assert_guarded(&env(
            Some("/home/u/.local/share/rappel-dev/../rappel"),
            None,
            home,
        ));
    }

    #[test]
    fn debug_guard_checks_xdg_release_dir() {
        assert_guarded(&env(Some("/data/rappel"), Some("/data"), Some("/home/u")));
    }

    #[test]
    fn debug_guard_checks_home_release_dir_even_with_xdg_set() {
        // XDG_DATA_HOME set for this run must not hide the usual release dir.
        assert_guarded(&env(
            Some("/home/u/.local/share/rappel"),
            Some("/data"),
            Some("/home/u"),
        ));
    }

    #[test]
    fn debug_guard_catches_xdg_data_home_inside_release_dir() {
        assert_guarded(&env(
            None,
            Some("/home/u/.local/share/rappel"),
            Some("/home/u"),
        ));
    }

    #[test]
    fn debug_guard_allows_unrelated_and_similar_names() {
        let home = Some("/home/u");
        let e = env(Some("/home/u/.local/share/rappel-test"), None, home);
        assert_eq!(
            resolve(&e, Debug),
            Path::new("/home/u/.local/share/rappel-test")
        );
        let e = env(Some("/tmp/rappel"), None, home);
        assert_eq!(resolve(&e, Debug), Path::new("/tmp/rappel"));
    }

    #[test]
    fn release_is_not_affected_by_the_guard() {
        let e = env(Some("/home/u/.local/share/rappel"), None, Some("/home/u"));
        assert_eq!(
            resolve(&e, Release),
            Path::new("/home/u/.local/share/rappel")
        );
    }

    // --- Creating the directory -------------------------------------------

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn creates_directory_with_mode_0700() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rappel");
        ensure_data_dir(&dir).unwrap();
        assert!(dir.is_dir());
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn creates_missing_parents_with_mode_0700() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("share");
        let dir = parent.join("rappel");
        ensure_data_dir(&dir).unwrap();
        assert_eq!(mode(&parent), 0o700);
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn tightens_existing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rappel");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
        ensure_data_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn is_idempotent_and_keeps_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rappel");
        ensure_data_dir(&dir).unwrap();
        fs::write(dir.join("keep"), b"data").unwrap();
        ensure_data_dir(&dir).unwrap();
        assert_eq!(fs::read(dir.join("keep")).unwrap(), b"data");
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn existing_file_is_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("rappel");
        fs::write(&file, b"").unwrap();
        match ensure_data_dir(&file) {
            Err(Error::NotADirectory(p)) => assert_eq!(p, file),
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }

    #[test]
    fn unwritable_parent_is_a_create_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        fs::write(&file, b"").unwrap();
        // A regular file as a parent fails on any user, including root.
        let dir = file.join("rappel");
        assert!(matches!(
            ensure_data_dir(&dir),
            Err(Error::CreateDir { .. })
        ));
    }
}
