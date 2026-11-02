# Temporary upstream patches

These directories exist only until the required changes are released upstream.

## `egui-winit`

Published `egui-winit` 0.36.2 plus the Wayland clipboard part of
[emilk/egui#8657](https://github.com/emilk/egui/pull/8657). It uses winit's
Wayland data-device clipboard instead of starting `smithay-clipboard`, which
would bind a second data device. Hyprland sends the selection and drags only to
a client's first data device, so both are required together.

## winit

Pinned in the workspace `Cargo.toml` to
[rust-windowing/winit#4729](https://github.com/rust-windowing/winit/pull/4729),
the reviewed 0.30 backport of native Wayland file drops. Remove the patch and
the local `egui-winit` directory once winit and egui-winit releases include
these changes.

See [REMOVAL.md](REMOVAL.md) for the removal checklist. The
`wayland_patches_are_reviewed_on_dependency_updates` test fails when the pinned
revision, eframe requirement or patched crate version drifts.
