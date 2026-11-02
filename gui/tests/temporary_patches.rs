//! Guards the temporary Wayland dependency patches.
//!
//! When `winit`, `egui`, `eframe` or `egui-winit` change, this test fails so the
//! patches are reviewed against `patches/REMOVAL.md`.

use std::path::{Path, PathBuf};
use toml::Value;

const WINIT_REV: &str = "7a962fd46470a137d9a7a1945bf5b5d0b95a002c";

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gui has a workspace parent")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn parse(path: &Path) -> Value {
    toml::from_str(&read(path))
        .unwrap_or_else(|error| panic!("invalid TOML in {}: {error}", path.display()))
}

fn package<'a>(lock: &'a Value, name: &str) -> &'a Value {
    lock.get("package")
        .and_then(Value::as_array)
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package.get("name").and_then(Value::as_str) == Some(name))
        })
        .unwrap_or_else(|| panic!("{name} is missing from Cargo.lock"))
}

fn version(package: &Value) -> &str {
    package
        .get("version")
        .and_then(Value::as_str)
        .expect("package version")
}

#[test]
fn wayland_patches_are_reviewed_on_dependency_updates() {
    let root = workspace();
    let checklist = root.join("patches/REMOVAL.md");
    assert!(
        checklist.is_file(),
        "restore patches/REMOVAL.md or delete the patches and this test"
    );

    // Workspace patch entries must still point at the reviewed sources.
    let manifest = parse(&root.join("Cargo.toml"));
    let patch = manifest
        .get("patch")
        .and_then(|patch| patch.get("crates-io"))
        .expect("workspace patch table");
    let winit = patch
        .get("winit")
        .expect("winit patch; follow patches/REMOVAL.md before removing it");
    assert_eq!(
        winit.get("rev").and_then(Value::as_str),
        Some(WINIT_REV),
        "the winit backport revision changed; review patches/REMOVAL.md"
    );
    assert!(
        winit
            .get("git")
            .and_then(Value::as_str)
            .is_some_and(|url| url.contains("crmne/winit")),
        "unexpected winit patch source; review patches/REMOVAL.md"
    );
    assert_eq!(
        patch
            .get("egui-winit")
            .and_then(|entry| entry.get("path"))
            .and_then(Value::as_str),
        Some("patches/egui-winit"),
        "the local egui-winit patch moved; review patches/REMOVAL.md"
    );

    // The patched crate must match the eframe/egui line used by the GUI.
    let gui_manifest = parse(&root.join("gui/Cargo.toml"));
    let eframe_requirement = gui_manifest
        .get("dependencies")
        .and_then(|deps| deps.get("eframe"))
        .and_then(|eframe| eframe.get("version"))
        .and_then(Value::as_str)
        .expect("eframe version requirement");
    assert_eq!(
        eframe_requirement, "0.36",
        "eframe was bumped; check whether the Wayland patches are still needed and follow \
         patches/REMOVAL.md"
    );

    let lock = parse(&root.join("Cargo.lock"));
    let locked_winit = package(&lock, "winit");
    assert!(
        locked_winit
            .get("source")
            .and_then(Value::as_str)
            .is_some_and(|source| source.contains(WINIT_REV)),
        "Cargo.lock no longer uses the reviewed winit revision; run `cargo update -p winit` \
         only after reviewing patches/REMOVAL.md"
    );
    let locked_eframe = package(&lock, "eframe");
    let locked_patched = package(&lock, "egui-winit");
    assert_eq!(
        version(locked_eframe),
        version(locked_patched),
        "eframe and the patched egui-winit versions diverged; update the patch or remove it \
         following patches/REMOVAL.md"
    );

    let patched = parse(&root.join("patches/egui-winit/Cargo.toml"));
    assert_eq!(
        patched
            .get("package")
            .and_then(|package| package.get("version"))
            .and_then(Value::as_str),
        Some(version(locked_eframe)),
        "patches/egui-winit/Cargo.toml is stale; review patches/REMOVAL.md"
    );
}
