# 4. Data directory resolution

- Status: Accepted
- Date: 2026-10-15
- Amended by: ADR 0020 (data directory names)

## Context

SPEC says data lives in `~/.local/share/rappel/`, debug builds use `~/.local/share/rappel-dev/` "so development never touches real data", and `RAPPEL_DATA_DIR` overrides both. Directories must have mode `0700` (SPEC §3). Losing review history is the worst possible bug (SPEC §1), so the rule must be enforced by code and tests, before any storage code exists.

## Decision

- **One pure function**, `rappel_core::data_dir::resolve_data_dir(env, profile)`. The environment values (`RAPPEL_DATA_DIR`, `XDG_DATA_HOME`, `HOME`) and the build profile are parameters. Only `DataDirEnv::from_process()` reads the real environment, and the app computes the profile from `cfg!(debug_assertions)`. Every case is testable from a single build.
- **Priority:** `RAPPEL_DATA_DIR`; else `$XDG_DATA_HOME`, or `$HOME/.local/share` if `XDG_DATA_HOME` is not usable, joined with `rappel-dev` (debug) or `rappel` (release). We follow XDG because `~/.local/share` is just its default value.
- **Empty variables count as unset.** A relative `XDG_DATA_HOME` is ignored, because the XDG spec says so. A relative `RAPPEL_DATA_DIR` is an **error**, not resolved against the current directory: where the data ends up should not depend on where the app was started from. If no absolute base can be found, that is an error too.
- **Debug guard.** In a debug build, a path that is the release directory, or inside it, is refused. Two base locations are checked, `<XDG_DATA_HOME>` and `$HOME/.local/share`, each with `rappel` and with `cards`, the name before ADR 0020 (four directories). The `$HOME` one covers a developer who sets `XDG_DATA_HOME` for one run while their real data sits in the default place. This applies to `RAPPEL_DATA_DIR` too, so an override cannot point a debug build at real data. Paths are compared component by component, after `.` and `..` are removed without looking at the disk. Release builds have no guard.
- **Creation:** `ensure_data_dir` creates the directory and any missing parents with mode `0700`. If the final directory already exists with other permissions, they are set to `0700`. A path that exists but is not a directory is an error.
- **On failure the app still starts.** It logs the error and shows a toast that stays until dismissed. Nothing uses the directory yet; the storage slice will decide what to block when there is no data directory. On success, the path is logged at `info`.

## Consequences

- The guard does not follow symlinks. A `RAPPEL_DATA_DIR` that is a symlink to the release directory would get past it. Catching that would mean reading the disk during resolution and would make the function impure. Making a symlink like that is a deliberate act, so we accept this limit.
- The guard only knows the default release locations. If you keep real data in a custom `RAPPEL_DATA_DIR` exported in your session (for example in `~/.profile`), `cargo run` inherits it, and the debug build will use that data. To make this visible, a debug build logs a warning whenever `RAPPEL_DATA_DIR` is set. A debug-only variable could replace it later if this turns out to matter.
- Parents created by Rappel get `0700` minus the umask, which is exactly `0700` with a normal umask. Only the final directory is forced to `0700`.
- Setting `RAPPEL_DATA_DIR` to an existing directory changes its permissions to `0700`. This is what SPEC asks for, and it is the directory the user chose for their data.
- Parents that already exist are never changed.
- Unix only (`std::os::unix`), which is fine for a GNOME app.
