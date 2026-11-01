#!/usr/bin/env bash
# Live development loop for the workspace.
#
#   scripts/dev-watch.sh                   # launch/restart the GUI on every save
#   WATCH_MODE=check scripts/dev-watch.sh  # fast cargo check instead
#   WATCH_MODE=test scripts/dev-watch.sh   # full test suite instead
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-watch >/dev/null 2>&1; then
    echo "cargo-watch is not installed:" >&2
    echo "  cargo install cargo-watch --locked" >&2
    exit 1
fi

mode="${WATCH_MODE:-gui}"
if [[ "${WATCH_TESTS:-0}" == "1" ]]; then
    mode="test"
fi

case "$mode" in
    gui)
        action="run -p musicwasp-gui"
        ;;
    check)
        action="check --workspace"
        ;;
    test)
        action="test --workspace"
        ;;
    *)
        echo "unknown WATCH_MODE: $mode (use gui, check or test)" >&2
        exit 2
        ;;
esac

exec cargo watch \
    -w Cargo.toml \
    -w core/src -w core/tests \
    -w cli/src \
    -w gui/src -w gui/tests \
    -w assets \
    -x "$action"
