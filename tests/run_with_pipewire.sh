#!/usr/bin/env bash
# Runs a command against a private, headless PipeWire daemon (Dummy-Driver).
# Usage: run_with_pipewire.sh <test-binary> [args...]
set -euo pipefail
if ! command -v pipewire >/dev/null; then
    echo "pipewire binary not found; skipping"; exit 0
fi
runtime=$(mktemp -d)
# Only kill PIDs we started: `kill 0` would signal the whole process group (ctest, the Nix builder).
cleanup() {
    for pid in ${wp_pid:-} ${pw_pid:-}; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
    rm -rf "$runtime"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR=$runtime PIPEWIRE_RUNTIME_DIR=$runtime
unset PIPEWIRE_REMOTE
pipewire >"$runtime/pipewire.log" 2>&1 &
pw_pid=$!
for _ in $(seq 50); do [ -S "$runtime/pipewire-0" ] && break; sleep 0.1; done
if [ ! -S "$runtime/pipewire-0" ]; then
    echo "pipewire failed to start:"; cat "$runtime/pipewire.log"; exit 1
fi
# CORINZIA_PW_SESSION=1: also run a session manager (WirePlumber), which
# configures and links streams as on a desktop (the Rust backend test).
if [ -n "${CORINZIA_PW_SESSION:-}" ]; then
    if ! command -v wireplumber >/dev/null; then echo "wireplumber not found; skipping"; exit 0; fi
    wireplumber >"$runtime/wireplumber.log" 2>&1 &
    wp_pid=$!
    sleep 1.5
fi
# The C++ test creates its own sink node (a pw_filter with DSP input ports) and
# links to it with pw-link, which is the session manager's job on a desktop.
"$@"
