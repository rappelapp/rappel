//! Guards the symbolic icon names the app uses (ADR 0086).
//!
//! The snap draws its icons from `gnome-core26`'s Adwaita, not from the
//! host's theme. A name that theme lacks shows as "image missing", and
//! nothing on a developer's desktop would reveal it. This test lists every
//! `*-symbolic` name in the Blueprint files and the Rust sources and fails
//! unless each one is bundled in the `GResource` under
//! `icons/scalable/actions/` or is on `data/gnome-core26-icons.txt`, the
//! names that snap ships. Only names written out as literals are found, so
//! a name built at run time, or a theme icon that does not end in
//! `-symbolic`, is not guarded.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every token ending in `-symbolic` in `text`.
fn symbolic_names(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .filter(|token| token.len() > "-symbolic".len() && token.ends_with("-symbolic"))
        .map(str::to_owned)
        .collect()
}

fn files_with_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_with_extension(&path, extension, out);
        } else if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

/// The names used by the app, with the file of one use of each.
fn used_names() -> Vec<(String, PathBuf)> {
    let root = manifest_dir();
    let mut files = Vec::new();
    files_with_extension(&root.join("data"), "blp", &mut files);
    files_with_extension(&root.join("src"), "rs", &mut files);
    // This file names icons in its own tests.
    files.retain(|file| !file.ends_with("src/icon_names.rs"));
    files.sort();
    let mut used: Vec<(String, PathBuf)> = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for name in symbolic_names(&text) {
            if !used.iter().any(|(seen, _)| *seen == name) {
                used.push((name, file.clone()));
            }
        }
    }
    used
}

/// Names of the SVGs the `GResource` bundles as theme icons.
fn bundled_names() -> BTreeSet<String> {
    let xml = std::fs::read_to_string(manifest_dir().join("data/resources.gresource.xml"))
        .unwrap_or_default();
    bundled_in(&xml)
}

/// The icon names listed under `icons/scalable/actions/` in a `GResource` file.
fn bundled_in(xml: &str) -> BTreeSet<String> {
    const DIR: &str = "icons/scalable/actions/";
    xml.split(['<', '>'])
        .filter_map(|part| part.strip_prefix(DIR))
        .filter_map(|file| file.strip_suffix(".svg"))
        .map(str::to_owned)
        .collect()
}

fn shipped_names() -> BTreeSet<String> {
    std::fs::read_to_string(manifest_dir().join("data/gnome-core26-icons.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_icon_is_bundled_or_shipped_by_gnome_core26() {
    let bundled = bundled_names();
    let shipped = shipped_names();
    assert!(
        shipped.len() > 100,
        "data/gnome-core26-icons.txt is missing or empty"
    );
    let used = used_names();
    assert!(
        used.len() > 10,
        "found too few icon names; did the scan break?"
    );
    let missing: Vec<String> = used
        .iter()
        .filter(|(name, _)| !bundled.contains(name) && !shipped.contains(name))
        .map(|(name, file)| format!("{name} ({})", file.display()))
        .collect();
    assert!(
        missing.is_empty(),
        "icons neither bundled in the `GResource` nor shipped by gnome-core26 \
         (they would show as \"image missing\" in the snap): {missing:?}"
    );
}

#[test]
fn every_bundled_icon_file_exists() {
    for name in bundled_names() {
        let path = manifest_dir().join(format!("data/icons/scalable/actions/{name}.svg"));
        assert!(path.is_file(), "{name}: {} is missing", path.display());
    }
}

#[test]
fn finds_symbolic_names_in_text() {
    let names = symbolic_names("x(a-b-symbolic);go-next-symbolic;symbolic");
    let found: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(found, ["a-b-symbolic", "go-next-symbolic"]);
}

#[test]
fn bundled_icons_are_read_from_the_gresource_listing() {
    let xml = "<file>icons/scalable/actions/a-symbolic.svg</file><file>window.ui</file>";
    let names: Vec<String> = bundled_in(xml).into_iter().collect();
    assert_eq!(names, ["a-symbolic"]);
}
