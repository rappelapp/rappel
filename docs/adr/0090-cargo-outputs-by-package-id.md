# 90. The snap's install script finds cargo's outputs by package id

- Status: Accepted
- Date: 2026-11-18
- Refers to: ADR 0085 (the snap, `scripts/snap-install.sh`); snap build run 44250902522

## Context

`scripts/snap-install.sh` builds the release binary and must then find two paths cargo chose: the binary, and the app's build-script output folder (`OUT_DIR`, holding the compiled catalogs and the merged desktop entry and metainfo). It read both from cargo's JSON messages, picking the output folder by its name, `…/build/rappel-<hex>/out`.

The snap's `rust` plugin installs the newest stable Rust (`rust-channel: stable`). The 0.4.3 build (run 44250902522) got Rust 1.100 and failed right after cargo's "Finished release": "cargo did not report the binary or the build script's folder". Building this workspace with cargo 1.100.0 shows why: its `build-script-executed` message now gives `out_dir` as `…/build/rappel/<hash>/out` (one folder per package, the hash below it), which the pattern refused. The binary's path was still found.

## Decision

- A new script, `scripts/cargo-outputs.sh PACKAGE_ID`, reads cargo's JSON messages on stdin and prints the package's binary (the `executable` of its `compiler-artifact` messages, which is null for a library or a build script) and its `out_dir` (from its `build-script-executed` message). Messages are chosen by their `package_id`, matched in full, never by a folder name. It fails unless it finds exactly one of each.
- `snap-install.sh` prints `cargo --version` (so a build log says which cargo ran), takes the id from `cargo pkgid --locked -p rappel`, which prints the id in the same form as the messages' `package_id` (the package id spec, `path+file:///…/crates/app#rappel@0.4.3`) from cargo 1.77 on, and passes it with the messages to the new script.
- A separate script rather than a function in `snap-install.sh`, so the tests can feed it messages without a release build. It stays in bash with `grep` and `sed`, as `jq` or Python are not promised in the snap's build environment; cargo prints one JSON object per line, and paths with a `"` or `\` are not supported.
- `crates/app/src/snap_install.rs` tests it on messages in two layouts: cargo 1.100's (from a real build with 1.100.0) and that of cargo 1.77 to 1.99; with rappel-core's messages, the app's build script as an artifact, and a package whose id starts with the app's around them. It also checks that missing, doubled or empty outputs, and a prefix of the id, are refused. A further test runs `snap-install.sh` itself with a stand-in `cargo` first on `PATH` that answers `--version`, `pkgid` and `build` with 1.100-layout messages, so the glue between the two scripts is covered without a release build. The full install was run with cargo 1.100.0 and installed every file.
- The snap workflow also runs on pull requests that touch `scripts/cargo-outputs.sh`.

## Consequences

- Cargo 1.77 or later is needed: before it, `cargo pkgid` and the messages' `package_id` had different forms, so nothing would match and the install would stop with its error.
- A future change of cargo's folder layout does not break the install, as long as the messages keep `package_id`, `executable` and `out_dir`, which are documented in cargo's "JSON messages".
- The snap still follows the newest stable Rust; pinning a version was not chosen, as CI's gate uses stable too, and a pinned toolchain would need updating by hand.
