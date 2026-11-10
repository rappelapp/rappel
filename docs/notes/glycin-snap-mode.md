# Proposal for glycin: a snap mode with seccomp and a memory limit

*Draft, written 2026-11-10 for Rappel's snap batch (slice 1). Not filed anywhere. It is "option B" of `docs/briefs/QUESTIONS.md` (Phase 3b, question 1): the safer follow-up to what Rappel ships in 0.4.1 (ADR 0055 and ADR 0082, amendments of 2026-11-10). If the maintainer wants it filed, it would go to GNOME's GitLab, project `GNOME/glycin`, as an issue first.*

## Problem

Apps that use the glycin Rust crate cannot keep its sandbox inside a strictly confined snap:

- bubblewrap needs user namespaces, `mount` and `pivot_root`. snapd's AppArmor policy refuses `userns` to every interface but `browser-support` with `allow-sandbox: true`, and even inside a snap with the GNOME extension, `bwrap` fails at once (`Can't read /proc/sys/kernel/overflowuid: Permission denied`).
- With `SandboxSelector::Auto`, `Sandbox::check_bwrap_syscalls_blocked` does not recognise that error as "blocked", so glycin picks `Bwrap` and **every image fails to load**. If a later bubblewrap failed at `unshare` instead, the `EPERM` message would match and glycin would silently fall back to `NotSandboxed`.
- The only working choice for the app today is `SandboxSelector::NotSandboxed`, which starts the loader with a cleared environment and `PR_SET_PDEATHSIG` only: **no seccomp filter and no memory limit** (`Sandbox::no_sandbox_command`, glycin 3.1 and 4.0). The loader then has every syscall the snap's profile allows, and a decompression bomb or a lying header can use all the memory the session has.

Canonical already solves this for the C library in the GNOME platform snap: `gnome-core26-sdk` builds glycin 2.2.1 with `patches/glycin-snap-no-sandbox.patch` (`RunEnvironment::Snap` when `$SNAP` is set; no bwrap, but the seccomp filter and `PR_SET_NO_NEW_PRIVS` applied before `exec`). The Rust crate on crates.io has no equivalent, so every Rust app packaged as a snap either patches glycin or loses the filter.

## Proposal

Add a fourth mechanism, used in a snap, that keeps everything of the bubblewrap sandbox that does not need namespaces.

1. **Detection.** `RunEnvironment::Snap` when `$SNAP` is a non-empty absolute path and the running executable resolves inside it (the second check keeps a development build in a `snap run --shell` session, or a stray `$SNAP` in a terminal, on the host path). Checked before the bwrap test run, after the Flatpak check. `SandboxMechanism::detect` maps it to a new `SandboxMechanism::SnapConfined` (name open), and `SandboxSelector` gets the matching variant so an app can ask for it explicitly. Neither enum is `#[non_exhaustive]` in 3.1, so a new variant breaks apps that match on them exhaustively: it fits the next major version, or the enums are made `#[non_exhaustive]` then.
2. **Seccomp.** The same filter as for bubblewrap (`Sandbox::seccomp_filter`, default action `Trap`, `GLYCIN_SECCOMP_DEFAULT_ACTION` honoured), loaded in the child between `fork` and `exec`: `prctl(PR_SET_NO_NEW_PRIVS, 1)` then `seccomp_load` of a filter built **before** the fork (building it in `pre_exec` would allocate after `fork`, which is not async-signal-safe). The allow-list must also permit `execve` of the loader itself, since the filter is in place before the `exec`, unlike under bwrap, where bwrap applies it last; alternatively the loader applies it to itself first thing, before it reads the image (in `glycin-utils`, where the loaders' shared start-up code lives; to be checked against its current structure), which avoids widening the list.
3. **Memory limit.** `Sandbox::set_memory_limit` (`setrlimit(RLIMIT_AS)` to 80 % of available memory, at most 20 GB considered, 200 MB kept free) in the same `pre_exec`, as bwrap's path already does. `RLIMIT_AS` needs no privilege and works under snap confinement.
4. **What stays the same:** the cleared environment (`INHERITED_ENVIRONMENT_VARIABLES`), `PR_SET_PDEATHSIG`, the private D-Bus socket on an inherited fd, the configuration lookup through `XDG_DATA_DIRS` (the GNOME extension already points it at the platform's loaders), `active_sandbox_mechanism()` reporting the new value, and the "running without sandbox" warning **not** printed for it (it is a reduced sandbox, not none).
5. **What it cannot give:** no mount or PID namespace, so the loader still sees what the snap sees (the app's `$SNAP_USER_DATA`, portal documents) and can talk to the session bus as far as the snap's plugs allow. The documentation of the mechanism should say so plainly, as Canonical's patch comment does.

## Why upstream rather than in each app

- An app cannot do it from outside: the filter must be installed in the loader process, which glycin starts, and `Loader` has no hook for a `pre_exec`. Patching glycin per app is what Canonical does for the C library, and it is what Rappel chose not to do (ADR 0055 amendment).
- The filter's syscall list belongs to glycin: it changes with the loaders, and only glycin can keep it right.
- It would make glycin's Rust and C behaviour agree inside snaps, where today they differ.

## Testing it

- A unit test for the detection (environment and executable path given as arguments, as Rappel's `render/loader_sandbox.rs` does).
- An integration test that starts a loader under the new mechanism outside any snap (the mechanism needs no snap to run; only its detection does) and checks that a blocked syscall gets `SIGSYS` and that a decompression bomb ends in an allocation failure in the loader, not in the caller.
- A manual check in a strict snap with the GNOME extension (`snap run --shell`): PNG, JPEG, WebP and GIF load; `grep Seccomp /proc/<loader pid>/status` shows `2`.

## What Rappel would change once it exists

Replace `SandboxSelector::NotSandboxed` in `render/loader_sandbox.rs` by the new selector (or go back to `Auto` if detection is reliable), update ADR 0055's row 42 and the residual risk (the memory limit and syscall filter would be back; the data folder and backup folder would still be reachable, since no namespace), and drop this note.

## Sources

- glycin 3.1.0 as vendored in `~/.cargo/registry`: `src/api_common.rs` (`SandboxMechanism::detect`, `SandboxSelector::determine_sandbox_mechanism`), `src/util.rs` (`RunEnvironment::cached`), `src/sandbox.rs` (`Sandbox::spawn`, `bwrap_command`, `no_sandbox_command`, `seccomp_filter`, `set_memory_limit`, `check_bwrap_syscalls_blocked`).
- `patches/glycin-snap-no-sandbox.patch` in <https://github.com/ubuntu/gnome-sdk/tree/gnome-core26-sdk> (read for ADR 0082, not copied).
- ADR 0082, finding 2, for the evidence that bubblewrap does not run under strict confinement.
