//! Compiles Blueprint files to `GtkBuilder` XML, then bundles them into a
//! gresource that `main.rs` embeds in the binary.

#![expect(
    clippy::expect_used,
    reason = "a build script reports failure by panicking: cargo shows the message and \
              stops the build, so `expect()` is the correct error handling here"
)]

use std::{env, fs, path::PathBuf, process::Command};

const DATA_DIR: &str = "data";

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR"));
    let ui_dir = out_dir.join("ui");

    let mut blueprints: Vec<PathBuf> = fs::read_dir(DATA_DIR)
        .expect("crates/app/data is part of the source tree")
        .map(|entry| {
            entry
                .expect("entries of crates/app/data are readable")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "blp"))
        .collect();
    blueprints.sort();

    println!("cargo:rerun-if-changed={DATA_DIR}");
    for blueprint in &blueprints {
        println!("cargo:rerun-if-changed={}", blueprint.display());
    }

    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(&ui_dir)
        .arg(DATA_DIR)
        .args(&blueprints)
        .status()
        .unwrap_or_else(|err| {
            panic!("could not run blueprint-compiler ({err}); install it, see README.md")
        });
    assert!(status.success(), "blueprint-compiler failed: {status}");

    glib_build_tools::compile_resources(
        &[PathBuf::from(DATA_DIR), ui_dir],
        "data/resources.gresource.xml",
        "rappel.gresource",
    );
}
