//! Guards the symbolic icons the app uses (ADR 0089, which replaces the
//! allow-list of ADR 0086).
//!
//! GTK looks an icon name up in the user's icon theme and the themes it
//! inherits, then in `hicolor`. In the snap the user's theme may not exist
//! at all (one in the host's `~/.local/share/icons` is not visible inside
//! the confinement), and GTK then searches `hicolor` alone, so a theme icon
//! such as `edit-undo-symbolic` shows as "image missing" (ADR 0086,
//! amendment). So the app names no theme icon: every symbolic icon it uses
//! is bundled in the `GResource` under `icons/scalable/actions/` with the
//! app id as prefix (`io.github.rappelapp.Rappel-edit-undo-symbolic`).
//! `GtkApplication` adds the resource's `icons/` folder to the icon theme,
//! where GTK treats it as part of `hicolor`, which every theme chain ends
//! with; and no theme can have an icon of that name to replace it.
//!
//! This test lists every `*-symbolic` name in the Blueprint files and the
//! Rust sources and fails unless each one is bundled. Only names written
//! out as literals are found: the app builds no icon name at run time, and
//! one built so would not be guarded.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use rappel_core::APP_ID;

/// Where the bundled icons are, in the `GResource` manifest and in `data/`.
const ICON_DIR: &str = "icons/scalable/actions/";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every token ending in `-symbolic` in `text`, a token being a run of
/// the characters an icon name can hold (the app id has dots and capitals).
fn symbolic_names(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_')))
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

/// Names of the SVGs the `GResource` bundles as icons, right-to-left
/// variants (`…-symbolic-rtl`) included.
fn bundled_files() -> BTreeSet<String> {
    let xml = std::fs::read_to_string(manifest_dir().join("data/resources.gresource.xml"))
        .unwrap_or_default();
    bundled_in(&xml)
}

/// The icon names listed under `icons/scalable/actions/` in a `GResource` file.
fn bundled_in(xml: &str) -> BTreeSet<String> {
    xml.split(['<', '>'])
        .filter_map(|part| part.strip_prefix(ICON_DIR))
        .filter_map(|file| file.strip_suffix(".svg"))
        .map(str::to_owned)
        .collect()
}

/// The bundled icon names, without the right-to-left variants.
fn bundled_names() -> BTreeSet<String> {
    bundled_files()
        .into_iter()
        .filter(|name| !name.ends_with("-rtl"))
        .collect()
}

/// The prefix every bundled icon name starts with.
fn prefix() -> String {
    format!("{APP_ID}-")
}

#[test]
fn every_icon_used_is_bundled() {
    let bundled = bundled_names();
    let used = used_names();
    assert!(
        used.len() > 10,
        "found too few icon names; did the scan break?"
    );
    let missing: Vec<String> = used
        .iter()
        .filter(|(name, _)| !bundled.contains(name))
        .map(|(name, file)| format!("{name} ({})", file.display()))
        .collect();
    assert!(
        missing.is_empty(),
        "icons not bundled in the `GResource` under {ICON_DIR} (an icon theme \
         can lack them, and in the snap they then show as \"image missing\"; \
         bundle the SVG as {}<name>.svg and use that name, ADR 0089): {missing:?}",
        prefix()
    );
}

#[test]
fn every_bundled_icon_has_the_app_prefix_and_is_used() {
    let used: BTreeSet<String> = used_names().into_iter().map(|(name, _)| name).collect();
    for name in bundled_files() {
        assert!(
            name.starts_with(&prefix()),
            "{name}: a bundled icon is named {}…, so no theme can replace it",
            prefix()
        );
        let base = name.strip_suffix("-rtl").unwrap_or(&name);
        assert!(
            base.ends_with("-symbolic"),
            "{name}: only symbolic icons are bundled (GTK recolours them by \
             the `-symbolic` name)"
        );
        assert!(used.contains(base), "{name}: bundled but not used");
    }
}

#[test]
fn bundled_icon_files_match_the_listing() {
    let dir = manifest_dir().join("data").join(ICON_DIR);
    let mut on_disk = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("the icon folder exists") {
        let name = entry.expect("the icon folder is readable").file_name();
        let name = name.to_string_lossy();
        let stem = name
            .strip_suffix(".svg")
            .unwrap_or_else(|| panic!("{name}: only SVG files belong in {ICON_DIR}"));
        on_disk.insert(stem.to_owned());
    }
    assert_eq!(
        on_disk,
        bundled_files(),
        "the files in data/{ICON_DIR} and the icons in resources.gresource.xml differ"
    );
}

#[test]
fn bundled_icons_are_plain_symbolic_svgs() {
    for name in bundled_files() {
        let path = manifest_dir().join(format!("data/{ICON_DIR}{name}.svg"));
        let svg = std::fs::read_to_string(&path).expect("the listing test checks it exists");
        assert!(svg.starts_with("<svg"), "{name}: not a bare SVG");
        // GTK recolours a symbolic icon's fills itself; nothing in it may
        // load something else or style it past that.
        for refused in ["<script", "<image", "<style", "href", "url(", "class="] {
            assert!(!svg.contains(refused), "{name}: contains {refused}");
        }
    }
}

/// The icons as GTK finds them where no theme helps: a theme name that
/// does not exist and no theme folders, as in the snap with a host-only
/// theme. Only the app's resources (and GTK's own) are left.
#[test]
fn gtk_finds_every_icon_used_without_any_theme() {
    let used: Vec<String> = used_names().into_iter().map(|(name, _)| name).collect();
    let resource_icons = format!("/{}/icons", APP_ID.replace('.', "/"));
    let (missing, unbundled_found, rtl) = crate::testing::with_widgets(move || {
        let theme = gtk::IconTheme::new();
        theme.set_search_path(&[]);
        theme.set_resource_path(&[resource_icons.as_str()]);
        theme.set_theme_name(Some("rappel-test-no-such-theme"));
        let missing: Vec<String> = used
            .iter()
            .filter(|name| !theme.has_icon(name))
            .cloned()
            .collect();
        let unbundled_found = theme.has_icon(&format!("{APP_ID}-not-bundled-symbolic"));
        // A right-to-left lookup takes the `-rtl` variant.
        let rtl = theme
            .lookup_icon(
                &format!("{APP_ID}-go-next-symbolic"),
                &[],
                16,
                1,
                gtk::TextDirection::Rtl,
                gtk::IconLookupFlags::empty(),
            )
            .file()
            .and_then(|file| file.uri().split('/').next_back().map(str::to_owned));
        (missing, unbundled_found, rtl)
    });
    assert!(missing.is_empty(), "GTK does not find: {missing:?}");
    assert!(!unbundled_found, "the lookup finds names nothing bundles");
    assert_eq!(
        rtl.as_deref(),
        Some(format!("{APP_ID}-go-next-symbolic-rtl.svg").as_str())
    );
}

#[test]
fn finds_symbolic_names_in_text() {
    let names = symbolic_names(&format!(
        "x(a-b-symbolic);go-next-symbolic;\"{APP_ID}-go-next-symbolic\";symbolic;{{APP_ID}}-symbolic.svg"
    ));
    let found: Vec<String> = names.into_iter().collect();
    assert_eq!(
        found,
        [
            "a-b-symbolic".to_owned(),
            "go-next-symbolic".to_owned(),
            format!("{APP_ID}-go-next-symbolic"),
        ]
    );
}

#[test]
fn bundled_icons_are_read_from_the_gresource_listing() {
    let xml = "<file>icons/scalable/actions/a-symbolic.svg</file><file>window.ui</file>";
    let names: Vec<String> = bundled_in(xml).into_iter().collect();
    assert_eq!(names, ["a-symbolic"]);
}
