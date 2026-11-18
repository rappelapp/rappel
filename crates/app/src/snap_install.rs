//! Tests of the snap's packaging (ADR 0085): `scripts/snap-install.sh`,
//! the files `build.rs` prepares for it, `snap/snapcraft.yaml` and
//! `.github/workflows/snap.yml`.
//!
//! - The install script, run into a temporary folder with this build's own
//!   `OUT_DIR` (and a stand-in for the binary, so no release build is
//!   needed), puts exactly the expected files at their paths, named after
//!   the app id, with their modes; the installed desktop entry and metainfo
//!   are valid and carry every language of `po/LINGUAS`.
//! - It refuses a relative or `..` destination, and half of its test
//!   variables, writing nothing.
//! - `scripts/cargo-outputs.sh` finds the binary and the build script's
//!   output folder in cargo's JSON messages by the app's package id, in
//!   the build-folder layouts before and since cargo 1.100, and refuses
//!   messages where either is missing or not one.
//! - The install script itself, with a stand-in `cargo` on `PATH`, takes
//!   the binary and that folder from it by package id and prints cargo's
//!   version, without a release build.
//! - The snap's recipe keeps the permissions the brief allows (no `home`,
//!   `removable-media`, `network`, `browser-support`, no classic
//!   confinement) and the workflow never publishes or reads a secret: the
//!   brief's stop conditions, checked on the text of both files (no YAML
//!   parser in the tree, and none is added for this).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rappel_core::APP_ID;

use crate::desktop_files::{
    BINARY, assert_desktop_entry_valid, assert_metainfo_valid, between, desktop_value, read, repo,
};
use crate::i18n::extract;

/// The app crate's build-script output: `locale/` and `data/` (`build.rs`).
const OUT_DIR: &str = env!("OUT_DIR");

/// The languages of `po/LINGUAS`.
fn languages() -> Vec<String> {
    read(&repo().join("po/LINGUAS"))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Runs the install script with `args` and the test variables `env`.
fn install(args: &[&str], env: TestEnv<'_>) -> Output {
    let mut command = Command::new("bash");
    command
        .arg(repo().join("scripts/snap-install.sh"))
        .args(args)
        .env_remove("RAPPEL_BINARY")
        .env_remove("RAPPEL_OUT_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .output()
        .unwrap_or_else(|err| panic!("bash could not run scripts/snap-install.sh ({err})"))
}

/// Every regular file under `root`, relative to it, sorted, with its mode.
fn files_under(root: &Path) -> Vec<(String, u32)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u32)>) {
        for entry in std::fs::read_dir(dir).expect("the folder is readable") {
            let path = entry.expect("its entries are readable").path();
            let meta = std::fs::symlink_metadata(&path).expect("the entry is readable");
            if meta.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("the walk stays under its root")
                    .to_string_lossy()
                    .into_owned();
                out.push((relative, meta.permissions().mode() & 0o7777));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// The install script's test variables: (name, value).
type TestEnv<'a> = &'a [(&'a str, &'a Path)];

/// A stand-in for the release binary, so the test needs no release build.
fn fake_binary(dir: &Path) -> PathBuf {
    let path = dir.join("binary");
    std::fs::write(&path, b"#!/bin/sh\n").expect("the temporary folder is writable");
    path
}

#[test]
fn the_install_script_puts_every_file_in_its_place() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let binary = fake_binary(dir.path());
    let dest = dir.path().join("dest");
    let out_dir = Path::new(OUT_DIR);
    let output = install(
        &[dest.to_str().expect("a UTF-8 temporary path"), "/usr"],
        &[("RAPPEL_BINARY", &binary), ("RAPPEL_OUT_DIR", out_dir)],
    );
    assert!(
        output.status.success(),
        "the install failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut expected = vec![
        (format!("usr/bin/{BINARY}"), 0o755),
        (format!("usr/share/applications/{APP_ID}.desktop"), 0o644),
        (format!("usr/share/metainfo/{APP_ID}.metainfo.xml"), 0o644),
        (
            format!("usr/share/icons/hicolor/scalable/apps/{APP_ID}.svg"),
            0o644,
        ),
        (
            format!("usr/share/icons/hicolor/symbolic/apps/{APP_ID}-symbolic.svg"),
            0o644,
        ),
    ];
    let domain = crate::i18n::DOMAIN;
    for language in languages() {
        expected.push((
            format!("usr/share/locale/{language}/LC_MESSAGES/{domain}.mo"),
            0o644,
        ));
    }
    expected.sort();
    assert_eq!(files_under(&dest), expected);

    // Each file is its source, byte for byte.
    let usr = dest.join("usr");
    let same = |installed: &str, source: &Path| {
        assert_eq!(
            std::fs::read(usr.join(installed)).expect("installed"),
            std::fs::read(source).expect("its source is readable"),
            "{installed} differs from {}",
            source.display()
        );
    };
    same(&format!("bin/{BINARY}"), &binary);
    same(
        &format!("share/applications/{APP_ID}.desktop"),
        &out_dir.join(format!("data/{APP_ID}.desktop")),
    );
    same(
        &format!("share/metainfo/{APP_ID}.metainfo.xml"),
        &out_dir.join(format!("data/{APP_ID}.metainfo.xml")),
    );
    for icon in [
        format!("scalable/apps/{APP_ID}.svg"),
        format!("symbolic/apps/{APP_ID}-symbolic.svg"),
    ] {
        same(
            &format!("share/icons/hicolor/{icon}"),
            &repo().join(format!("data/icons/hicolor/{icon}")),
        );
    }
    for language in languages() {
        same(
            &format!("share/locale/{language}/LC_MESSAGES/{domain}.mo"),
            &out_dir.join(format!("locale/{language}/LC_MESSAGES/{domain}.mo")),
        );
    }
}

#[test]
fn the_installed_desktop_files_are_valid_and_translated() {
    let out_dir = Path::new(OUT_DIR);
    let desktop_path = out_dir.join(format!("data/{APP_ID}.desktop"));
    let metainfo_path = out_dir.join(format!("data/{APP_ID}.metainfo.xml"));
    assert_desktop_entry_valid(&desktop_path, "every language");
    assert_metainfo_valid(&metainfo_path, "every language");

    let desktop = read(&desktop_path);
    assert_eq!(desktop_value(&desktop, "Exec"), Some(BINARY));
    assert_eq!(desktop_value(&desktop, "Icon"), Some(APP_ID));
    let metainfo = read(&metainfo_path);
    assert_eq!(between(&metainfo, "<id>", "</id>"), Some(APP_ID));
    assert_eq!(
        between(
            &metainfo,
            "<launchable type=\"desktop-id\">",
            "</launchable>"
        ),
        Some(format!("{APP_ID}.desktop").as_str())
    );
    assert_eq!(between(&metainfo, "<binary>", "</binary>"), Some(BINARY));

    // Every language's translation is merged in, of every translated text.
    let desktop_source = read(&repo().join(format!("data/{APP_ID}.desktop.in")));
    let metainfo_source = read(&repo().join(format!("data/{APP_ID}.metainfo.xml.in")));
    let units = extract::metainfo_units(&metainfo_source).expect("the metainfo reads");
    for language in languages() {
        for key in extract::DESKTOP_KEYS {
            assert!(
                desktop_source.contains(&format!("\n{key}="))
                    == desktop.contains(&format!("\n{key}[{language}]=")),
                "{key} has no {language} in the installed desktop entry"
            );
        }
        assert_eq!(
            metainfo
                .matches(&format!("xml:lang=\"{language}\""))
                .count(),
            units.len(),
            "every translated element of the installed metainfo has its {language}"
        );
    }
}

#[test]
fn several_languages_merge_in_one_pass_in_their_order() {
    let lookup = |language: &str, text: &str| Some(format!("{text} ({language})"));
    let desktop = extract::merge_desktop_languages(
        "[Desktop Entry]\nName=Rappel\nExec=rappel\n",
        &["fr", "de"],
        lookup,
    )
    .expect("the entry reads");
    assert_eq!(
        desktop,
        "[Desktop Entry]\nName=Rappel\nName[fr]=Rappel (fr)\nName[de]=Rappel (de)\nExec=rappel\n"
    );
    let metainfo = extract::merge_metainfo_languages(
        "<component>\n  <summary>A and B</summary>\n</component>\n",
        &["fr", "de"],
        |language, text| (language == "de").then(|| format!("{text} <de>")),
    )
    .expect("the metainfo reads");
    assert_eq!(
        metainfo,
        "<component>\n  <summary>A and B</summary>\n  \
         <summary xml:lang=\"de\">A and B &lt;de&gt;</summary>\n</component>\n"
    );
}

#[test]
fn the_install_script_refuses_bad_arguments_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let binary = fake_binary(dir.path());
    let dest = dir.path().join("dest");
    let dest_text = dest.to_str().expect("a UTF-8 temporary path");
    let out_dir = Path::new(OUT_DIR);
    let both: TestEnv<'_> = &[("RAPPEL_BINARY", &binary), ("RAPPEL_OUT_DIR", out_dir)];
    let parent = format!("{dest_text}/../dest");
    let cases: [(&[&str], TestEnv<'_>); 6] = [
        (&[], both),
        (&["relative/dest"], both),
        (&[parent.as_str()], both),
        (&[dest_text, "usr"], both),
        (&[dest_text, "/usr", "extra"], both),
        (&[dest_text], &[("RAPPEL_BINARY", &binary)]),
    ];
    for (args, env) in cases {
        let output = install(args, env);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?} was not refused:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!dest.exists(), "{args:?} wrote into the destination");
    }
}

/// Runs `scripts/cargo-outputs.sh package_id` with `messages` on stdin.
fn cargo_outputs(package_id: &str, messages: &str) -> Output {
    use std::io::Write as _;
    let mut child = Command::new("bash")
        .arg(repo().join("scripts/cargo-outputs.sh"))
        .arg(package_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("bash could not run scripts/cargo-outputs.sh ({err})"));
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(messages.as_bytes())
        .expect("the script reads its stdin");
    child.wait_with_output().expect("the script finishes")
}

/// Cargo's messages for a build of the app, trimmed to the fields the
/// parser reads, around the app's own: rappel-core's library and build
/// script (another `out_dir`), the app's build script as an artifact
/// (`executable` null), and a package whose id starts with the app's.
/// `app` and `core` are the two packages' ids, `out` the app's `out_dir`.
fn messages(app: &str, core: &str, target: &str, out: &str, core_out: &str) -> String {
    let artifact = |id: &str, kind: &str, executable: &str| {
        format!(
            "{{\"reason\":\"compiler-artifact\",\"package_id\":\"{id}\",\
             \"target\":{{\"kind\":[\"{kind}\"]}},\"executable\":{executable},\"fresh\":false}}"
        )
    };
    let script = |id: &str, out_dir: &str| {
        format!(
            "{{\"reason\":\"build-script-executed\",\"package_id\":\"{id}\",\
             \"linked_libs\":[],\"linked_paths\":[],\"cfgs\":[],\"env\":[],\
             \"out_dir\":\"{out_dir}\"}}"
        )
    };
    let decoy = format!("{app}-decoy");
    [
        artifact(core, "custom-build", "null"),
        script(core, core_out),
        artifact(core, "lib", "null"),
        artifact(app, "custom-build", "null"),
        script(app, out),
        artifact(&decoy, "bin", &format!("\"{target}/release/decoy\"")),
        script(&decoy, &format!("{target}/release/build/decoy/0/out")),
        artifact(app, "bin", &format!("\"{target}/release/rappel\"")),
        "{\"reason\":\"build-finished\",\"success\":true}".to_owned(),
    ]
    .join("\n")
        + "\n"
}

#[test]
fn the_binary_and_out_dir_are_found_by_package_id_in_any_layout() {
    let target = "/build/rappel/target";
    // (layout, the app's id, rappel-core's id, the app's out_dir, core's).
    // Cargo before 1.77 is not supported (ADR 0090).
    let cases = [
        // Cargo 1.100 (checked with its own build of this workspace):
        // `build/<package>/<hash>/out`, the id as a package id spec.
        (
            "cargo 1.100",
            "path+file:///build/rappel/crates/app#rappel@0.4.3",
            "path+file:///build/rappel/crates/core#rappel-core@0.4.3",
            format!("{target}/release/build/rappel/b82b475a3d060210/out"),
            format!("{target}/release/build/rappel-core/5d1e0c3a9b7f2468/out"),
        ),
        // Cargo 1.77 to 1.99: `build/<package>-<hash>/out`.
        (
            "cargo 1.99",
            "path+file:///build/rappel/crates/app#rappel@0.4.3",
            "path+file:///build/rappel/crates/core#rappel-core@0.4.3",
            format!("{target}/release/build/rappel-07e9dc817fe1de31/out"),
            format!("{target}/release/build/rappel-core-1f2e3d4c5b6a7980/out"),
        ),
    ];
    for (layout, app, core, out, core_out) in cases {
        let output = cargo_outputs(app, &messages(app, core, target, &out, &core_out));
        assert!(
            output.status.success(),
            "{layout}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{target}/release/rappel\n{out}\n"),
            "{layout}"
        );
    }
}

#[test]
fn the_parser_refuses_missing_or_ambiguous_outputs() {
    let app = "path+file:///r/crates/app#rappel@0.4.3";
    let core = "path+file:///r/crates/core#rappel-core@0.4.3";
    let full = messages(app, core, "/t", "/t/release/build/rappel/1/out", "/t/c");
    let without = |reason: &str| {
        full.lines()
            .filter(|line| !(line.contains(app) && line.contains(reason)))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    let twice = format!(
        "{full}{}\n",
        full.lines()
            .find(|line| line.contains("build-script-executed") && line.contains(app))
            .expect("the app's build script ran")
            .replace("/1/out", "/2/out")
    );
    let two_binaries = format!(
        "{full}{}\n",
        full.lines()
            .find(|line| line.contains("\"bin\"") && line.contains(&format!("\"{app}\"")))
            .expect("the app's binary was built")
            .replace("/release/rappel", "/release/other")
    );
    let cases = [
        ("two binaries", two_binaries),
        ("no build script", without("build-script-executed")),
        ("no binary", without("\"bin\"")),
        ("two out_dirs", twice),
        ("no messages", String::new()),
    ];
    for (case, input) in cases {
        let output = cargo_outputs(app, &input);
        assert_eq!(output.status.code(), Some(1), "{case} was not refused");
        assert!(output.stdout.is_empty(), "{case} printed a result");
    }
    // Another package's id, or none, finds nothing of the app's.
    let other = cargo_outputs("path+file:///r/crates/app#rappel@0.4", &full);
    assert_eq!(other.status.code(), Some(1), "a prefix of the id matched");
    assert_eq!(cargo_outputs("", &full).status.code(), Some(2));
}

#[test]
fn the_install_script_takes_cargos_outputs_by_package_id() {
    // `snap-install.sh` without its test variables, with a stand-in `cargo`
    // first on PATH that answers `--version`, `pkgid` and `build` as cargo
    // 1.100 would: the glue between it and `cargo-outputs.sh` (ADR 0090).
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = dir.path().join("target");
    let release = target.join("release");
    std::fs::create_dir_all(&release).expect("the temporary folder is writable");
    let binary = release.join("rappel");
    std::fs::write(&binary, b"#!/bin/sh\n").expect("the temporary folder is writable");
    let target_text = target.to_str().expect("a UTF-8 temporary path");
    let app = "path+file:///stub/crates/app#rappel@0.4.3";
    let core = "path+file:///stub/crates/core#rappel-core@0.4.3";
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).expect("the temporary folder is writable");
    let stub = |messages: &str| {
        let messages_path = dir.path().join("messages.json");
        std::fs::write(&messages_path, messages).expect("the temporary folder is writable");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n\
             --version) echo 'cargo 1.100.0 (stub)' ;;\n\
             pkgid) echo '{app}' ;;\n\
             build) cat '{}' ;;\n\
             *) exit 3 ;;\nesac\n",
            messages_path.display()
        );
        let cargo = bin.join("cargo");
        std::fs::write(&cargo, script).expect("the temporary folder is writable");
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755))
            .expect("the stand-in can be made executable");
    };
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let path = Path::new(&path);

    stub(&messages(app, core, target_text, OUT_DIR, "/elsewhere/out"));
    let dest = dir.path().join("dest");
    let output = install(
        &[dest.to_str().expect("a UTF-8 temporary path")],
        &[("PATH", path)],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the install failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.starts_with("cargo 1.100.0 (stub)\n"), "{stdout}");
    assert_eq!(
        std::fs::read(dest.join(format!("usr/bin/{BINARY}"))).expect("installed"),
        std::fs::read(&binary).expect("the stand-in is readable"),
    );
    assert!(
        dest.join(format!("usr/share/applications/{APP_ID}.desktop"))
            .is_file()
    );

    // Messages without the app's build script: refused, nothing written.
    let full = messages(app, core, target_text, OUT_DIR, "/elsewhere/out");
    stub(
        &(full
            .lines()
            .filter(|line| !(line.contains("build-script-executed") && line.contains(app)))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"),
    );
    let dest = dir.path().join("dest-2");
    let output = install(
        &[dest.to_str().expect("a UTF-8 temporary path")],
        &[("PATH", path)],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("cargo did not report the binary or the build script's folder")
    );
    assert!(
        !dest.exists(),
        "a refused install wrote into the destination"
    );
}

/// The lines of a YAML file without comments, trimmed.
fn yaml_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split(" #").next().unwrap_or(line).trim().to_owned())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

#[test]
fn the_snap_keeps_to_the_allowed_permissions() {
    let text = read(&repo().join("snap/snapcraft.yaml"));
    let lines = yaml_lines(&text);
    let has = |line: &str| lines.iter().any(|candidate| candidate == line);
    for line in [
        "name: rappel",
        "base: core26",
        "confinement: strict",
        "grade: stable",
        "license: GPL-3.0-or-later",
        "adopt-info: rappel",
        "extensions: [gnome]",
        "- audio-playback",
        "interface: dbus",
        "bus: session",
        &format!("name: {APP_ID}"),
        &format!("common-id: {APP_ID}"),
        &format!("desktop: usr/share/applications/{APP_ID}.desktop"),
        &format!("parse-info: [usr/share/metainfo/{APP_ID}.metainfo.xml]"),
        &format!("icon: data/icons/hicolor/scalable/apps/{APP_ID}.svg"),
        "RAPPEL_LOCALEDIR: $SNAP/usr/share/locale",
        "scripts/snap-install.sh \"$CRAFT_PART_INSTALL\" /usr",
    ] {
        assert!(has(line), "snap/snapcraft.yaml lacks `{line}`");
    }
    // The only plug of its own is audio-playback; the extension adds the
    // desktop set. Never a plug that reaches the home folder, removable
    // media, the network or user namespaces, nor classic confinement.
    for word in [
        "home",
        "removable-media",
        "network",
        "network-bind",
        "browser-support",
        "personal-files",
        "system-files",
        "classic",
        "devmode",
    ] {
        assert!(
            !lines.iter().any(|line| line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .any(|token| token == word)),
            "snap/snapcraft.yaml names `{word}`"
        );
    }
    // Every list item, so a new plug cannot hide in a list: a new item
    // needs this test changed, and so a look at the brief.
    let items: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("- "))
        .collect();
    assert_eq!(
        items,
        ["audio-playback", "gcc", "pkg-config"],
        "the list items of snap/snapcraft.yaml changed"
    );
}

#[test]
fn the_snap_workflow_only_builds() {
    let text = read(&repo().join(".github/workflows/snap.yml"));
    let lines = yaml_lines(&text);
    for forbidden in [
        "secrets",
        "upload",
        "SNAPCRAFT_STORE_CREDENTIALS",
        "snapcraft-store",
        "publish",
        "store_login",
        "release:",
    ] {
        assert!(
            !lines
                .iter()
                .filter(|line| !line.starts_with("- uses: actions/upload-artifact@"))
                .any(|line| line.contains(forbidden)),
            ".github/workflows/snap.yml uses `{forbidden}`"
        );
    }
    assert!(
        text.contains("\npermissions:\n  contents: read\n"),
        "the workflow's token must be read-only"
    );
    // Every action pinned by a full commit SHA, as in ci.yml.
    let actions: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("- uses: ") || line.starts_with("uses: "))
        .collect();
    assert!(
        actions
            .iter()
            .any(|line| line.contains("snapcore/action-build@")),
        "the snap is built with snapcore/action-build"
    );
    for line in actions {
        let sha = line.rsplit('@').next().unwrap_or_default();
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "not pinned by a commit SHA: {line}"
        );
    }
    // Pull requests only for the packaging itself; never every change.
    for path in [
        "- 'snap/**'",
        "- 'scripts/snap-install.sh'",
        "- 'scripts/cargo-outputs.sh'",
        "- '.github/workflows/snap.yml'",
    ] {
        assert!(
            lines.iter().any(|line| line == path),
            "the workflow's pull-request paths lack {path}"
        );
    }
}
