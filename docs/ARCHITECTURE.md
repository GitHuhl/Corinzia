# Corinzia architecture

Corinzia is a music notation editor and CLAP DAW for Linux, written in Rust.
One process, one window (iced on wgpu), one project file (SQLite).

## 1. Crates and threads

```
rust/
  corinzia-app       the application: UI, editor, playback glue, files
  corinzia-edit      pure editing operations (note input, ranges, structure)
  corinzia-model     project model + .project storage
  corinzia-layout    the engraver: display lists, pages, PDF
  corinzia-perform   the performance of a movement; tempo maps; export documents
  corinzia-audio     CLAP host, realtime engine, PipeWire output
  corinzia-sine      Corinzia Sine, the built-in CLAP instrument
  corinzia-export    MIDI and MusicXML writers
  corinzia-score     fractions, note values, the engraver's input types
```

| Thread | Does | Never |
|---|---|---|
| UI (iced) | editing, engraving (milliseconds), plan building, plugin main-thread calls | blocks on audio |
| Audio (PipeWire's RT thread) | `Engine::process`: notes, plugins, mixing | allocates, locks, touches the model |
| Worker (iced executor) | autosave, file dialogs | — |

Engraving runs on the UI thread because it is fast: the demo score engraves
in about 3 ms in a release build. Every edit re-engraves the movement.

## 2. Model (`corinzia-model`)

* A `Project` owns:
  * players with their instruments (name, abbreviation, clef, track);
  * DAW tracks (instrument plugin node, MIDI channel, gain, pan, mute, solo);
  * the audio graph (nodes, edges, plugin state chunks);
  * movements.
* A `Movement` holds:
  * maps: tempo, meter, key;
  * structure: barlines, repeat endings, rehearsal marks;
  * per player, a `Lane`: instrument changes, clef changes, text, chord
    symbols, and up to four voices.
* A voice's events are `Arc<Vec<Event>>`. An event is a note, chord or rest:
  start and duration as exact whole-note fractions, heads (pitch, tie,
  velocity), articulation bits, dynamic, span bits (slurs, hairpins) and
  lyrics (verse, syllable, hyphen, extender).
* Every object has a persistent element id. The id is used in the file, in
  the engraving (hit testing) and in playback (highlighting).
* `notation::score` converts a movement into the engraver's input
  (`corinzia_score::score::Score`).

**Storage** (`store.rs`, `schema/project.sql`):

* one SQLite file per project;
* voice content is stored as JSONB in chunks of four whole notes;
* structure marks, clefs, text and chord symbols go in one JSONB column per
  movement;
* plugin state chunks have their own table, and a hash skips rewriting
  unchanged state.

`save` upserts every row and deletes rows that no longer exist.

## 3. Editing (`corinzia-edit`, `corinzia-app::editor`)

* `corinzia-edit` holds the pure functions. Each takes a voice's events (or
  a movement) and returns the new state.
* **Note input** uses the overwrite model. A new event occupies
  `[pos, pos + duration)`: what it covers is removed, and what it partly
  covers is truncated. Voice 1 stays contiguous, because remainders and
  gaps become rests. Voices 2–4 may have holes.
* **Pitches** follow the key. The octave is the one nearest the previous
  note.
* **Ranges** support erase, slice, paste, transpose and spanners. A passage
  is the notes *starting* in `[from, to)`.
* **Structure** covers:
  * popover parsing: key ("Eb", "f#" minor), time, clef, barline, tempo
    ("Allegro 132", "q=96"), endings ("1, 2"), ornaments, chord symbols
    ("F#m7b5/E" is written F♯m7♭5/E);
  * insertion and deletion of bars. Map entries at 0 stay; a barline at the
    cut closes the gap; endings are clipped.
* **`Editor`** (in the app, UI-independent and unit-tested) holds:
  * the caret, input state, selection or passage, clipboard and popover
    targets;
  * undo. Each action snapshots the project, which is cheap because voices
    are `Arc`-shared. Slider drags merge into one step.
  * two counters. `revision` moves on every change; `music_revision` moves
    only when the music changes. The app re-engraves and rebuilds the
    playback plan on `music_revision`, so mixer edits do neither.

## 4. Engraving (`corinzia-layout`)

**Output.** The engraver produces a renderer-independent **display list**
in staff spaces, y pointing down. It contains:
* SMuFL glyphs (Bravura), text, lines, beam quads and tie crescents, each
  tagged with an element id;
* staff geometry and a time→x column map, both tagged with their system.

The app's canvas, the PDF writer (`pdf.rs`, glyph and text outlines with no
fonts embedded) and the tiny-skia previews in the tests all draw the same
list.

**Spacing** follows LilyPond's model and defaults, on rhythmic columns
shared by all staves:
* The movement's *common shortest duration* is the shortest note most bars
  have, capped at a dotted eighth (`base-shortest-duration`).
* A note of that duration gets 2 increments of 1.2 spaces
  (`shortest-duration-space`, `spacing-increment`); each doubling of
  duration adds one increment.
* As in LilyPond, the distance runs from the left note's head: one increment
  is replaced by the head's width. The head widths are LilyPond's, so bars
  are as wide as LilyPond's, though the heads are drawn in Bravura.
* Optical corrections:
  * facing stems (up, then down) get room in proportion to their overlap,
    and lose it the other way round;
  * stems in the same direction gain or lose a quarter space for a leap of a
    third or more;
  * stems next to barlines are corrected by how much of the stem lies inside
    the staff;
  * a bar holding a single note gets an extra space before it.
* Minimum distances keep accidentals, dots and flags apart.
* Lyric syllables and chord symbols also widen columns, so they never
  collide horizontally.

`tests/lilypond.rs` checks this against LilyPond itself. It writes each
test score as a `.ly` file, has LilyPond report every notehead and rest
position (`-dbackend=null`), and compares the gaps. The current corpus has
106 gaps, with a mean error of 0.4 % and a worst case of 7.3 %.

**Notes.**
* Notes are split at barlines into tied, notatable values.
* Stems follow the furthest-head rule, or the voice in polyphony.
* Seconds in chords are offset. Accidentals follow key and bar, and stack
  in columns.
* Beams are grouped with LilyPond's defaults: one beam per beat (dotted
  quarters in compound meters). Groups of only eighths span half a bar in
  4/4 and the whole bar in 3/4.
* Beams are placed as in LilyPond's quanting:
  * ideal stem ends (LilyPond's beamed stem lengths) are fitted by least
    squares;
  * the slope is damped (0.6·tanh), and further for concave groups;
  * the best allowed position wins, scored on stem length and slope. Allowed
    positions put the beam centre on a staff line or 0.19 off it.
  Stems match LilyPond's in direction everywhere in the test corpus. Their
  lengths are within 0.15 spaces on average.
* Also drawn: ledger lines, dots, flags, secondary beams and hooks, rests
  and whole-bar rests.

**Marks around the staff.** Each staff has a skyline: the boxes of
everything drawn so far. Outer marks are placed against it from the staff
outwards, stacking instead of colliding, in this order:
* tuplets;
* fermatas and ornaments;
* technique text;
* chord symbols, on a common baseline above the staff, with SMuFL chord
  accidentals and qualities;
* hairpins with their dynamics, on a shared baseline;
* expression text;
* lyrics: one line per verse, with syllables centred under their notes,
  hyphens centred in the gaps and extenders through melismas;
* on the top staff: volta brackets, rehearsal marks and tempo marks.

Items that join staves (barlines) belong to no staff's skyline.

**Vertical spacing** follows LilyPond's `staff-staff-spacing`:
* Staves are 9 spaces apart (`basic-distance`).
* They move apart only where one staff's music would come within 1 space
  (`padding`) of the next staff's music.
* The two staves' skylines are compared box by box where they overlap
  horizontally. A system is engraved, measured and, if needed, engraved
  again with the new distances.
* The spacing test matches LilyPond's staff distances to within 0.1 space.

**Pages.**
* System breaks follow LilyPond's optimal breaker. A dynamic program over
  bar boundaries minimises, over all lines, each line's force squared plus a
  fixed per-line cost. Force is the room a line must gain or give up,
  divided by its stretchability.
* Lines may be compressed as well as stretched.
* Justification shares the room out by each spring's stretchability, as
  LilyPond does. Every line is justified, the last one included (LilyPond's
  `ragged-last = ##f`).
* `tests/lilypond.rs` also checks justified lines and system breaks against
  LilyPond. The breaks match at 23 of 30 line widths.
* Ties and endings that cross a break are split.
* Systems are stacked by their real extents onto A4 pages (132.3 × 187.1
  staff spaces), with the title on page 1.

**Text** is measured with the font that draws it (`set_text_font`; the app
uses DejaVu Serif). The test notation (`notation.rs`, LilyPond-like) builds
scores for the engraving tests.

## 5. Performance (`corinzia-perform`)

* **Playback order** follows a performer's path. An end repeat jumps back
  to the start repeat (or the beginning) until its passes are done. An
  ending not marked for the current pass is skipped. A repeat with endings
  takes as many passes as its highest ending number; without endings, two.
* **Notes:**
  * ties are merged;
  * ornaments are written out (trills in the key, turns, mordents,
    tremolos);
  * articulations become gate times;
  * dynamics become velocities, with hairpins interpolated in time and
    legato inside slurs;
  * arpeggios roll from the bottom up;
  * accents add weight.
* **Tempo map:** constant segments and linear ramps; seconds ↔ performed
  quarters; bars and beats.
* `document` builds the export document: the notation plus one MIDI track
  per sounding DAW track, with gate and roll applied in time as in
  playback.

## 6. Audio (`corinzia-audio`)

* **`host`** loads `.clap` bundles with `libloading` and `clap-sys`. It
  scans `CLAP_PATH`, `~/.clap`, the Nix profiles, `/run/current-system/sw/lib/clap`
  and `/usr/lib/clap`. Corinzia Sine is hosted in-process through the same
  CLAP entry point.
  * Lifecycle follows CLAP's thread rules:
    * create, activate, deactivate, destroy, state and `on_main_thread`
      run on the UI thread;
    * `start_processing`, `process` and `stop_processing` run on the audio
      thread.
  * The host provides the `thread-check` and `state` extensions. Plugins
    that only speak MIDI get MIDI events.
* **`pool`** keeps one activated instance per track. Instances survive plan
  rebuilds; state is restored from the project and written back on save.
* **`snapshot`** is the render plan. It holds the performance compiler's
  notes, resolved to tracks and placed in samples, with movements laid end
  to end. Notes are sorted with note-offs before note-ons at the same
  sample. A `Timeline` maps samples back to written positions for the
  playhead.
* **`engine`** is the realtime core.
  * The UI's `EngineHandle` sends commands through wait-free SPSC queues
    (`rtrb`): swap plan, play, stop, seek, live mixer values, preview
    notes.
  * Retired plans come back through a second queue and are dropped on the
    UI thread. Plugins that leave a plan stop processing on the audio
    thread first, and are deactivated and destroyed only when the last
    plan using them is dropped.
  * Per block, the engine schedules notes sample-accurately into
    preallocated event lists, gives the plugins a transport (beats, tempo,
    bar, time signature), and mixes with equal-power pan, mute and solo.
  * Atomics carry the playhead back to the UI.
  * `render_offline` drives the same code for tests.
* **`backend`** is a PipeWire playback stream (32-bit float, stereo) on its
  own thread. Its realtime callback runs `Engine::process`.

Not yet: buses and plugin delay compensation (tracks mix straight to the
output), automation, plugin GUIs.

## 7. The application (`corinzia-app`)

| Module | Holds |
|---|---|
| `main.rs` | the iced application: messages, toolbar, Score and Mix tabs, popovers, status bar, autosave, `--keys` / `--screenshot` / `--export` |
| `editor.rs` | the editing controller (§3) |
| `keys.rs` | the keyboard map |
| `score_view.rs` | the score canvas |
| `panels.rs` | the Notes and Structure panels (SMuFL glyph buttons) |
| `playback.rs` | the engine handle, PipeWire output, plugin pool; the plan kept in step with the editor |
| `mix.rs`, `arrange.rs` | mixer strips, plugin browser, arrange view |
| `files.rs` | New Score from an instrument list; PDF/MIDI/MusicXML export |

**The score canvas** (`score_view.rs`):
* Pages are drawn as vector geometry and cached until the engraving, zoom,
  pan or selection changes.
* The caret, rubber band and playhead go on an uncached overlay.
* A click is located on a staff (part, time, staff step) from the system's
  columns. That drives mouse note input.
* Hit testing covers notes, rests, chord symbols and lyrics.

**Files and saving.**
* File dialogs go through the XDG desktop portal (`rfd`).
* Projects with a file are saved 0.8 s after a change, on a worker thread.
* Untitled projects ask before their changes are discarded.

## 8. Packaging

`nix build .#appimage` builds `Corinzia-x86_64.AppImage` for other
distributions:
* The binary is compiled with `cargo-zigbuild` against glibc 2.31, with
  SQLite compiled in (the `portable` feature). It links only the system's
  `libpipewire`; Vulkan, Wayland/X11 and the file-dialog portal load at run
  time.
* `packaging/AppRun` points at the bundled DejaVu Serif. It also switches to
  a stand-in `libpipewire` (`packaging/pipewire-stub.c`) when the system has
  none, so Corinzia runs without sound instead of failing to start.
* The squashfs image is appended to the pinned AppImage type-2 runtime.
* `.github/workflows/release.yml` builds it on GitHub and publishes it:
  * a push to `main` creates release `v<version>` (the workspace version)
    if it doesn't exist yet, so bumping the version makes a new release;
  * a pushed `v*` tag is published as well.

## 9. Verification

* **Unit and integration tests** cover every crate (`cargo test
  --workspace`, also run by `nix build`):
  * edits and parsers;
  * storage round trips;
  * engraving geometry, with previews in `target/tmp/engraving`;
  * PDF structure;
  * playback order and ornaments;
  * MIDI and MusicXML;
  * CLAP loading, both in-process and via dlopen, and state;
  * sample-accurate offline renders, transport, mixing and plan swaps;
  * the app's flows: typing, clicking, marquee, popovers, lyrics and chord
    streams, mixer undo, autosave, playback and the playhead.
* **Live audio:** `CORINZIA_PW=1 CORINZIA_PW_SESSION=1
  scripts/run_with_pipewire.sh cargo test -p corinzia-audio --test
  pipewire` plays through a private PipeWire daemon. WirePlumber links the
  stream to a null sink.
* **The real app:** `scripts/screenshot.sh out.png [--keys "…"]` runs it
  headless (Xvfb and Mesa lavapipe) and saves a screenshot.
* **LilyPond** is the reference for engraving: the spacing test compares
  note positions with LilyPond's (from the dev shell, and in `nix build`).
* **MusicXML** is checked against LilyPond's `musicxml2ly` from the dev
  shell.
