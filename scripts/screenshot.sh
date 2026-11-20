#!/usr/bin/env bash
# Runs the app headless (Xvfb + Mesa's software Vulkan) and saves a
# screenshot: scripts/screenshot.sh out.png [corinzia arguments...]
# Run inside `nix develop`; CORINZIA_BIN overrides the binary. The window
# fills a 1920x1080 virtual display (SHOT_SIZE overrides it, e.g. SHOT_SIZE=1280x860).
set -euo pipefail
out=$1; shift
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${CORINZIA_BIN:-$here/rust/target/debug/corinzia}
export VK_ICD_FILENAMES=${CORINZIA_LAVAPIPE_ICD:?run inside nix develop}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
size=${SHOT_SIZE:-1920x1080}
xvfb-run -a -s "-screen 0 ${size}x24" "$bin" --size "$size" --screenshot "$out" "$@"
