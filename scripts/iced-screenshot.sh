#!/usr/bin/env bash
# Runs the iced front end headless (Xvfb + Mesa's software Vulkan) and saves a
# screenshot: scripts/iced-screenshot.sh out.png [corinzia-iced arguments...]
# Run inside `nix develop`.
set -euo pipefail
out=$1; shift
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${CORINZIA_ICED_BIN:-$here/rust/target/debug/corinzia-iced}
export VK_ICD_FILENAMES=${CORINZIA_LAVAPIPE_ICD:?run inside nix develop}
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
xvfb-run -a -s "-screen 0 1400x1000x24" "$bin" --screenshot "$out" "$@"
