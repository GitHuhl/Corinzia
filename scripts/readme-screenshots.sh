#!/usr/bin/env bash
# Remakes the README's screenshots in docs/screenshots: public-domain scores
# from the music21 corpus (downloaded once into $CACHE) and the demo
# project, each shown by the app headless with a private PipeWire session.
# Run inside `nix develop`, after `cargo build -p corinzia-app`.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
cache=${CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/corinzia-screenshots}
out=$here/docs/screenshots
mkdir -p "$cache" "$out"
if [ ! -f "$cache/movement1.mxl" ]; then
    pip download --no-deps --quiet music21==10.5.2 -d "$cache"
    for f in beethoven/opus18no1/movement1.mxl handel/rinaldo/Lascia_chio_pianga.mxl leadSheet/fosterBrownHair.mxl; do
        unzip -o -j -q "$cache"/music21-*.whl "music21/corpus/$f" -d "$cache"
    done
fi
shots() {
    shot() { local name=$1; shift; "$here/scripts/screenshot.sh" "$out/$name.png" "$@" | tail -n 1; }
    shot quartet "$cache/movement1.mxl"
    shot quartet-panorama "$cache/movement1.mxl" --view panorama
    shot quartet-part "$cache/movement1.mxl" --part 1
    shot aria "$cache/Lascia_chio_pianga.mxl"
    shot leadsheet "$cache/fosterBrownHair.mxl"
    shot mix "$cache/movement1.mxl" --keys 'ctrl+2'
    shot byzantine --chant 1 --keys 'home right right right right right right right right right right right right'
    shot help --keys 'f1'
    shot new-score --keys 'ctrl+n'
    shot preferences --keys 'ctrl+,'
}
export -f shots
export here out cache
CORINZIA_PW_SESSION=1 "$here/scripts/run_with_pipewire.sh" bash -c shots
