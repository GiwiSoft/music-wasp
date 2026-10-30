#!/usr/bin/env bash
# Install a Linux desktop launcher for MusicWasp.
#
# The launcher uses assets/ic_launcher-web.png as its icon, while the running
# window keeps the MusicWasp logo as its window/taskbar icon.
#
# Usage: ./scripts/install-desktop.sh [path-to-musicwasp-binary]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="${1:-${MUSICWASP_BIN:-}}"

if [ -z "$bin" ]; then
    for candidate in \
        "$root/musicwasp" \
        "$root/target/release/musicwasp" \
        "$root/target/debug/musicwasp" \
        "$(command -v musicwasp || true)"
    do
        if [ -n "$candidate" ] && [ -x "$candidate" ]; then
            bin="$candidate"
            break
        fi
    done
fi

if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "musicwasp binary not found; run 'cargo build --release' or pass its path" >&2
    exit 1
fi
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

icon_dir="$data/icons/hicolor/512x512/apps"
apps_dir="$data/applications"
mkdir -p "$icon_dir" "$apps_dir"

install -m 644 "$root/assets/ic_launcher-web.png" "$icon_dir/musicwasp.png"
sed "s|^Exec=.*|Exec=$bin %U|" "$root/packaging/musicwasp.desktop" \
    > "$apps_dir/musicwasp.desktop"

if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null; then
    gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "installed $apps_dir/musicwasp.desktop (Exec=$bin)"
echo "installed $icon_dir/musicwasp.png"
