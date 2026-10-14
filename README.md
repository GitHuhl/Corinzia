# Corinzia

## A LilyPond-backed DAW/Music Notation Hybrid Program with CLAPi and VST3 Instrument Support for Linux.

<img width="1400" height="900" alt="image-1791944260201" src="https://github.com/user-attachments/assets/f8f6399e-5659-416a-ae44-e3661d441b29" />


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

Run it with `nix run`, or `./build/corinzia` from the dev shell. Press **N** to
write notes: A–G enter pitches, 1–7 pick durations (5 = quarter), R enters a
rest, and the left Notes panel does the same with the mouse. Clicking a staff
in input mode places a note at that pitch and beat. The full key map is in
§5.5 of the architecture document.

Select passages by dragging a marquee on empty paper, Shift+clicking or
pressing Ctrl+A. Then Ctrl+C/X/V, R (repeat), Delete, ↑/↓, S (slur) and
H/Shift+H (hairpins) work on the whole passage. **Page | Panorama** switches
between the printed layout and one endless system. Zoom stays vector-sharp at
any level.

Score structure uses industry-style popovers: Shift+K key ("Eb", "f#" minor),
Shift+M time ("6/8"), Shift+T tempo ("Allegro 132"), Shift+B barline (":|"
repeats play back), Shift+C clef, Shift+X / Shift+P expression or technique
text, and Shift+A rehearsal mark. The right-hand Structure panel has the same
actions plus insert/delete bars (Ctrl+B to insert). Click a mark or text and
press Delete to remove it. Shift+O adds ornaments (tr, mordent, turn, trem,
arp, breath, caesura); "1st / 2nd ending" or Shift+R add repeat endings, and
repeats play back as written.

**⚡ Native** engraves with Corinzia's own engraver, written in Rust with
the Bravura font, in both Page and Panorama view. Re-engraving takes tens of
milliseconds instead of seconds. PDF export still uses LilyPond. See §5.5f of the architecture document for the plan
towards a full native engraver and an iced front end.

**Export ▾** in the toolbar writes PDF (LilyPond), MIDI (sounds like playback,
one track per mixer track) and MusicXML 4.0. MIDI and MusicXML come from a
small Rust crate (`rust/corinzia-export`) built by the same Nix flake.

The notation and DAW sides are separate tabs: **Score** (Ctrl+1) for engraving
and note input, **Mix** (Ctrl+2) for the arrange view, mixer strips and plugin
browser. Selection, transport and undo are shared between them.

Status: implemented and tested are the core engine with plugin delay
compensation, CLAP hosting (including the MIDI-dialect fallback and plugin
scanning and lifecycle), the PipeWire backend, storage, the engraving round
trip, the canvas and a QML application shell. Note input works from the
keyboard, the mouse and the Notes panel. Plugin GUI embedding, range editing
and copy/paste, slurs and hairpins, and the piano roll are next; see §9 of the
architecture document.
