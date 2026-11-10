//! Which sandbox glycin's image loader runs in (ADR 0055, amended for the
//! snap by ADR 0082).
//!
//! Everywhere but Rappel's own snap, Rappel leaves the choice to glycin
//! (`SandboxSelector::Auto`): bubblewrap with a seccomp filter and a memory
//! limit where the system allows it, flatpak-spawn in a Flatpak, and a
//! plain child process (with a warning, `picture::warn_unsandboxed`) where
//! no namespaces can be made. Inside the snap, bubblewrap cannot run
//! (ADR 0082, finding 2): with `Auto` glycin would pick bubblewrap and every
//! image would fail. There, and only there, the loader runs as a plain
//! child process (`SandboxSelector::NotSandboxed`), and the snap's strict
//! confinement (no network, no home folder) is the boundary.
//!
//! The decision is [`choose`], pure: the environment and the paths come in
//! as arguments, so the tests set no variable. [`current`] makes it once per
//! run from the real environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The snap's name (`name:` in `snap/snapcraft.yaml`), as snapd gives it in
/// `$SNAP_NAME`. A parallel install (`rappel_beta`) keeps this name there;
/// only `$SNAP_INSTANCE_NAME` carries the suffix.
const SNAP_NAME: &str = "rappel";

/// The sandbox glycin's loader is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderSandbox {
    /// glycin chooses (`SandboxSelector::Auto`), as before the snap.
    Auto,
    /// Inside Rappel's own snap: no bubblewrap
    /// (`SandboxSelector::NotSandboxed`), the snap's confinement is the
    /// boundary.
    SnapConfinement,
}

impl LoaderSandbox {
    /// glycin's selector for this choice. `Auto` is glycin's default, so
    /// outside the snap the loader is set up exactly as before.
    pub fn selector(self) -> glycin::SandboxSelector {
        match self {
            Self::Auto => glycin::SandboxSelector::Auto,
            Self::SnapConfinement => glycin::SandboxSelector::NotSandboxed,
        }
    }

    /// The start-up log line saying which mode is in use (not translated:
    /// logs are for developers).
    pub fn log_text(self) -> &'static str {
        match self {
            Self::Auto => {
                "image loader: glycin chooses the sandbox (bubblewrap where the system allows it)"
            }
            Self::SnapConfinement => {
                "image loader: not sandboxed by bubblewrap; inside the rappel snap, \
                 its strict confinement is the boundary"
            }
        }
    }
}

/// Chooses the loader's sandbox. [`LoaderSandbox::SnapConfinement`] only
/// when all of these hold, else [`LoaderSandbox::Auto`]:
///
/// - `$SNAP` is set to an absolute path;
/// - `$SNAP_NAME` is exactly `rappel`;
/// - the running binary, `exe`, resolved by `canonical` (symbolic links
///   followed), is inside `$SNAP`, also resolved, and is not `$SNAP` itself.
///
/// So a shell or a test with `$SNAP` set by hand, another snap's
/// environment, or a build run from `target/` inside a snap shell all keep
/// glycin's own choice. `var` reads an environment variable, `exe` is
/// `std::env::current_exe()` (`None` if it failed), and `canonical` is
/// `std::fs::canonicalize` (`None` if it failed): any failure is `Auto`.
pub fn choose(
    var: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
) -> LoaderSandbox {
    let in_snap = || -> Option<bool> {
        let snap = PathBuf::from(var("SNAP")?);
        if !snap.is_absolute() || var("SNAP_NAME")? != SNAP_NAME {
            return Some(false);
        }
        let snap = canonical(&snap)?;
        let exe = canonical(exe?)?;
        // `starts_with` compares whole components: `/snap/rappel2` is not
        // inside `/snap/rappel`.
        Some(exe != snap && exe.starts_with(&snap))
    };
    if in_snap() == Some(true) {
        LoaderSandbox::SnapConfinement
    } else {
        LoaderSandbox::Auto
    }
}

/// The choice for this run, made once from the real environment.
pub fn current() -> LoaderSandbox {
    static CURRENT: OnceLock<LoaderSandbox> = OnceLock::new();
    *CURRENT.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        choose(
            |name| std::env::var_os(name),
            exe.as_deref(),
            |path| std::fs::canonicalize(path).ok(),
        )
    })
}

/// Logs, once at start-up, which mode the image loader uses.
pub fn log_mode() {
    tracing::info!("{}", current().log_text());
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// An environment of `pairs`.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |name| map.get(name).cloned()
    }

    /// Paths as they are: no symbolic links.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "stands in for `canonicalize`, which can fail"
    )]
    fn same(path: &Path) -> Option<PathBuf> {
        Some(path.to_path_buf())
    }

    /// Environment variables, by name and value.
    type Vars = &'static [(&'static str, &'static str)];

    const SNAP_ENV: [(&str, &str); 2] = [("SNAP", "/snap/rappel/x1"), ("SNAP_NAME", "rappel")];
    const SNAP_EXE: &str = "/snap/rappel/x1/usr/bin/rappel";

    #[test]
    fn inside_rappel_s_snap_the_loader_is_not_sandboxed() {
        let exe = Path::new(SNAP_EXE);
        assert_eq!(
            choose(env(&SNAP_ENV), Some(exe), same),
            LoaderSandbox::SnapConfinement
        );
        assert!(matches!(
            LoaderSandbox::SnapConfinement.selector(),
            glycin::SandboxSelector::NotSandboxed
        ));
    }

    #[test]
    fn everywhere_else_glycin_chooses() {
        let exe = Some(SNAP_EXE);
        let cases: [(&str, Vars, Option<&str>); 9] = [
            ("no snap variables", &[], exe),
            (
                "$SNAP without $SNAP_NAME",
                &[("SNAP", "/snap/rappel/x1")],
                exe,
            ),
            ("$SNAP_NAME without $SNAP", &[("SNAP_NAME", "rappel")], exe),
            (
                "another snap",
                &[("SNAP", "/snap/rappel/x1"), ("SNAP_NAME", "other")],
                exe,
            ),
            (
                "a name that only starts with rappel",
                &[("SNAP", "/snap/rappel/x1"), ("SNAP_NAME", "rappel-dev")],
                exe,
            ),
            (
                "a relative $SNAP",
                &[("SNAP", "snap/rappel/x1"), ("SNAP_NAME", "rappel")],
                Some("snap/rappel/x1/usr/bin/rappel"),
            ),
            (
                "an empty $SNAP",
                &[("SNAP", ""), ("SNAP_NAME", "rappel")],
                exe,
            ),
            (
                "a build run from target/ in a snap shell",
                &SNAP_ENV,
                Some("/home/me/rappel/target/debug/rappel"),
            ),
            ("the binary's path unknown", &SNAP_ENV, None),
        ];
        for (what, vars, exe) in cases {
            assert_eq!(
                choose(env(vars), exe.map(Path::new), same),
                LoaderSandbox::Auto,
                "{what}"
            );
        }
        assert!(matches!(
            LoaderSandbox::Auto.selector(),
            glycin::SandboxSelector::Auto
        ));
    }

    #[test]
    fn a_neighbouring_folder_or_snap_itself_is_not_inside() {
        let var = || env(&SNAP_ENV);
        for exe in [
            "/snap/rappel/x10/usr/bin/rappel",
            "/snap/rappel/x1",
            "/snap/rappel",
        ] {
            assert_eq!(
                choose(var(), Some(Path::new(exe)), same),
                LoaderSandbox::Auto,
                "{exe}"
            );
        }
    }

    /// Both paths are compared once resolved: a binary reached through a
    /// link into the snap counts, a link out of it does not, and a path
    /// that cannot be resolved is `Auto`.
    #[test]
    fn paths_are_compared_resolved() {
        let var = || env(&SNAP_ENV);
        // `/snap` is a link to `/var/lib/snapd/snap` (Fedora, Arch).
        let resolve = |path: &Path| {
            let text = path.to_str()?;
            Some(PathBuf::from(text.replacen(
                "/snap/",
                "/var/lib/snapd/snap/",
                1,
            )))
        };
        assert_eq!(
            choose(var(), Some(Path::new(SNAP_EXE)), resolve),
            LoaderSandbox::SnapConfinement
        );
        // A link inside the snap that points outside it.
        let outside = |path: &Path| {
            if path == Path::new(SNAP_EXE) {
                Some(PathBuf::from("/home/me/rappel"))
            } else {
                Some(path.to_path_buf())
            }
        };
        assert_eq!(
            choose(var(), Some(Path::new(SNAP_EXE)), outside),
            LoaderSandbox::Auto
        );
        assert_eq!(
            choose(var(), Some(Path::new(SNAP_EXE)), |_| None),
            LoaderSandbox::Auto
        );
        // `$SNAP` itself cannot be resolved (it does not exist).
        let no_snap = |path: &Path| (path != Path::new("/snap/rappel/x1")).then(|| path.into());
        assert_eq!(
            choose(var(), Some(Path::new(SNAP_EXE)), no_snap),
            LoaderSandbox::Auto
        );
    }

    #[test]
    fn the_log_line_names_the_mode() {
        assert!(LoaderSandbox::Auto.log_text().contains("bubblewrap"));
        assert!(LoaderSandbox::SnapConfinement.log_text().contains("snap"));
        assert_ne!(
            LoaderSandbox::Auto.log_text(),
            LoaderSandbox::SnapConfinement.log_text()
        );
    }

    /// The test binary runs from `target/`, never from a snap.
    #[test]
    fn the_tests_run_with_glycin_s_choice() {
        assert_eq!(current(), LoaderSandbox::Auto);
    }
}
