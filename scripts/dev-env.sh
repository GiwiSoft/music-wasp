#!/usr/bin/env bash
# No-root fallback for the ALSA build dependency.
#
# Normally your distro provides the headers:
#   Debian/Ubuntu : sudo apt install libasound2-dev
#   Fedora        : sudo dnf install alsa-lib-devel
#   Arch          : sudo pacman -S alsa-lib
#
# If you cannot install packages, run this once. It unpacks the Debian dev
# package into .sysroot/ without root; `.cargo/config.toml` and `core/build.rs`
# pick that up, so plain `cargo build` / `cargo run` work afterwards.
#
# Usage:  ./scripts/dev-env.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sysroot="$root/.sysroot"

if pkg-config --exists alsa 2>/dev/null; then
    echo "system ALSA dev files found, nothing to do"
    exit 0
fi

if ! command -v dpkg-deb >/dev/null; then
    echo "dpkg-deb not available: install the ALSA dev package with your package manager instead" >&2
    exit 1
fi

if [ ! -d "$sysroot" ]; then
    echo "fetching libasound2-dev into $sysroot"
    mkdir -p "$sysroot"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    (cd "$tmp" && apt-get download libasound2-dev >/dev/null)
    dpkg-deb -x "$tmp"/libasound2-dev_*.deb "$sysroot"
fi

# The dev package ships no unversioned .so; borrow the runtime one.
pc_dir="$(find "$sysroot" -name alsa.pc -printf '%h\n' | head -1)"
if [ -z "$pc_dir" ]; then
    echo "alsa.pc missing after extraction" >&2
    exit 1
fi
lib_dir="$(dirname "$pc_dir")"

if [ ! -e "$lib_dir/libasound.so" ]; then
    # Match the multiarch tuple of the extracted package so we never pick up a
    # foreign-architecture runtime library.
    triple="$(basename "$lib_dir")"
    runtime=""
    for candidate in "/usr/lib/$triple/libasound.so.2" /usr/lib64/libasound.so.2 /usr/lib/libasound.so.2; do
        if [ -e "$candidate" ]; then
            runtime="$candidate"
            break
        fi
    done
    if [ -z "$runtime" ]; then
        echo "no runtime libasound.so.2 found for $triple" >&2
        exit 1
    fi
    ln -sf "$runtime" "$lib_dir/libasound.so"
fi

echo "ready: local ALSA sysroot at $sysroot"
echo "now run: cargo run --release -p musicwasp-gui"
