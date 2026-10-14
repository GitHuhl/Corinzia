#!/usr/bin/env bash
# Runs the app headless (Xvfb + Mesa's software Vulkan) and saves a
# screenshot: scripts/screenshot.sh out.png [corinzia arguments...]
# Run inside `nix develop`; CORINZIA_BIN overrides the binary.
set -euo pipefail
out=$1; shift
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${CORINZIA_BIN:-$here/rust/target/debug/corinzia}
export VK_ICD_FILENAMES=${CORINZIA_LAVAPIPE_ICD:?run inside nix develop}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
xvfb-run -a -s "-screen 0 1400x1000x24" "$bin" --screenshot "$out" "$@"
