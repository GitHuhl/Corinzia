# Corinzia

A Linux-only hybrid **music notation editor + DAW** with native **CLAP** plugin hosting.

* Zero-lag score canvas (Qt Quick scene graph) with an instant provisional layer
  and an asynchronous definitive LilyPond engraving layer.
* Lock-free real-time engine: sample-accurate `clap_event_note`,
  `clap_event_param_value` / `clap_event_param_mod` and transport.
* Multi-movement scores, players with instrument doubling (switches plugin and
  channel at the exact sample), up to four voices per staff, automation lanes.
* SQLite `.project` container: chunked JSONB music payload, isolated plugin
  state blobs, non-blocking autosave, persistent undo journal.

**Read the blueprint:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

## Build & test (Nix only)

```sh
nix develop                                   # pinned nixos-26.05 toolchain
cmake -S . -B build -G Ninja
cmake --build build
ctest --test-dir build --output-on-failure    # core + offscreen Qt/LilyPond tests
python3 -I scripts/validate_schema.py         # schema + example data check

nix build                                     # sandboxed build running the full test suite
```

## Layout

| Path | What |
|---|---|
| `include/corinzia/core` | `ElementId`, typed ids, exact `Fraction` time |
| `include/corinzia/model` | semantic model (Project → Movement → PlayerLane → Voice), commands, undo |
| `include/corinzia/rt` | SPSC queue, seqlock, tempo map, playback snapshot, playhead tracker |
| `include/corinzia/audio` | CLAP host (lifecycle, extensions), RT event lists, audio engine |
| `include/corinzia/compile` | model → `PlaybackSnapshot` compiler |
| `include/corinzia/engrave` | model → LilyPond emitter with SVG object ids |
| `include/corinzia/storage` | SQLite project store + background persistence worker |
| `include/corinzia/ui` | engraving service, SVG hit index, layout anchors, `ScoreCanvas` |
| `schema/` | `project.sql` (embedded into the binary), `example_project.json` |
| `tests/` | dependency-free test harness, in-process CLAP recorder plugin fixture |

Status: the core engine, CLAP hosting, storage, engraving round trip and canvas
are implemented and tested. The audio backends (PipeWire/JACK), the QML
application shell and plugin GUI embedding are still design only, listed in
§9 of the architecture document.
