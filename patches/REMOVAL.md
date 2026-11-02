# Removing the temporary Wayland patches

The workspace patches `winit` and `egui-winit` so native Wayland file drops and
text clipboard work on compositors that only serve a client's first data device
(for example Hyprland). Remove them once both fixes are in released crates.

Follow this checklist whenever `winit`, `egui`, `eframe` or `egui-winit` is
updated. The `wayland_patches_are_reviewed_on_dependency_updates` test fails
until the patches and this checklist are addressed.

## 1. Confirm upstream availability

- [ ] `winit` release includes [rust-windowing/winit#4729](https://github.com/rust-windowing/winit/pull/4729)
      (native Wayland file drops).
- [ ] `egui-winit` release includes the clipboard part of
      [emilk/egui#8657](https://github.com/emilk/egui/pull/8657)
      (winit data-device clipboard instead of `smithay-clipboard`).
- [ ] The released crates support the same MSRV and feature flags currently used by
      `eframe`/`egui`.

## 2. Try without the patches

- [ ] Bump `eframe`, `egui`, and `egui_kittest` together to the release that contains the
      clipboard fix.
- [ ] Remove the `[patch.crates-io]` entry for `winit` and `egui-winit` from the workspace
      `Cargo.toml`.
- [ ] Run `cargo update -p winit -p egui-winit` (or a full `cargo update` if needed) and
      check `cargo tree -i winit` and `cargo tree -i egui-winit` resolve registry versions.
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo build --release`

## 3. Manual Wayland check

- [ ] On Hyprland (or another first-data-device compositor), drag files from a file manager
      into the library and onto a playlist.
- [ ] Copy text from another app into a MusicWasp text field, and copy text out of MusicWasp.
- [ ] `MUSICWASP_X11=1 musicwasp` still starts through XWayland as a fallback.
- [ ] No duplicate data-device warnings in the terminal.

## 4. Remove the scaffolding

- [ ] Delete `patches/egui-winit/`.
- [ ] Update or delete `patches/README.md`.
- [ ] Delete or rewrite `gui/tests/temporary_patches.rs`.
- [ ] Update the workspace `Cargo.toml` comment above the removed patch section.
- [ ] Update Architecture, Development, Troubleshooting and README wording that mentions the
      pinned backport.
- [ ] Commit the dependency bump and patch removal together.

## If the patches must stay

If only one half landed upstream, keep the other patch, update its version,
refresh this checklist, and adjust the test expectations. Do not silently bump
`eframe` past the patched `egui-winit` version.
