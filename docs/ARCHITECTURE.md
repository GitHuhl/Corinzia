# Corinzia — Architecture Specification

A Linux-only hybrid of **music notation editor** and **DAW**: an ultra-responsive
score canvas, a background engraving compiler (LilyPond), and a lock-free
real-time audio engine hosting **CLAP** plugins sample-accurately.

This document is the blueprint. Most of it is backed by code in this
repository, built and tested with Nix (see [§9](#9-build-test--verification-status)).
Each section says which parts are implemented and which are still design only.

| Layer | Technology | Code |
|---|---|---|
| Domain model + undo | C++20, no Qt | `include/corinzia/model/` |
| RT primitives | lock-free SPSC, seqlock, tempo map | `include/corinzia/rt/` |
| Audio engine + CLAP host | CLAP 1.2 ABI, `dlopen` | `include/corinzia/audio/`, `src/audio/` |
| Model → playback compiler | worker thread | `src/compile/SnapshotCompiler.cpp` |
| Notation emitter | LilyPond text with object IDs | `src/engrave/LilyPondEmitter.cpp` |
| Storage | SQLite ≥ 3.45, WAL, JSONB | `schema/project.sql`, `src/storage/` |
| Canvas + engraving service | Qt 6 Quick scene graph (RHI), QtSvg | `include/corinzia/ui/`, `src/ui/` |

---

## 1. System architecture

### 1.1 Processes and threads

```
┌──────────────────────────────── corinzia (one process) ────────────────────────────────┐
│                                                                                         │
│  GUI thread (Qt main)          Qt render thread          RT audio thread (SCHED_FIFO)   │
│  ├ QML UI (inspectors,         ├ ScoreCanvas::            ├ PipeWire pw_filter / JACK    │
│  │  mixer, piano roll)         │  updatePaintNode()       │  process callback            │
│  ├ Project (sole owner)        ├ QSGNode tree → RHI       ├ AudioEngine::process()       │
│  ├ UndoStack / Commands        │  (Vulkan / GL)           │  └ ClapPluginInstance::      │
│  ├ PluginManager (CLAP         │                          │     process()  ×N            │
│  │  main-thread calls, GUIs)   │                          │                              │
│  └ 60 Hz pump: playhead,       │                          │                              │
│    telemetry, garbage          │                          │                              │
│                                                                                         │
│  Worker pool (QThreadPool)     Persistence thread         CLAP plugin threads           │
│  ├ SnapshotCompiler            ├ ProjectStore (SQLite      (plugin-owned; reach the host │
│  ├ EngravingService jobs       │  write connection, WAL)    only through thread-safe     │
│  │  (emit .ly → QProcess →     └ coalescing SaveBatch      host callbacks)              │
│  │   SVG → index → raster)        queue                                                 │
└───────────────────────────────────────┬─────────────────────────────────────────────────┘
                                        │ stdin/.svg files
                                ┌───────▼────────┐
                                │ lilypond (child │  out-of-band engraving compiler,
                                │ process, killed │  one per movement in flight
                                │ when stale)     │
                                └────────────────┘
```

**Invariants**

1. Only the GUI thread mutates the `Project`, and only through `Command`s.
2. The RT thread never allocates, frees, locks, logs, or touches the model. It sees
   an immutable `PlaybackSnapshot` and talks through SPSC queues and one seqlock.
3. Every worker gets **immutable snapshots**. Bulky payloads (event lists,
   automation points, plugin state) are `shared_ptr<const T>` copy-on-write, so
   a snapshot costs O(#voices) pointer copies, not a deep copy.
4. A single **ElementId** (64-bit, project-unique, never reused) names an object
   in memory, in SQLite, in the engraved SVG (`id="e-<base36>"`) and as the CLAP
   `note_id`. That shared key is what holds the system together.

### 1.2 Frameworks

* **Frontend:** Qt Quick/QML for chrome (toolbars, inspectors, mixer, piano roll,
  timeline). QML binds to C++ facades (`QAbstractItemModel` adapters over the
  model). The model itself stays Qt-free so it can be tested and threaded freely.
* **Canvas:** `ScoreCanvas : QQuickItem` builds raw `QSGNode`s. Qt 6 renders
  them through RHI (Vulkan, or OpenGL on Linux). Zoom and pan are a single
  `QSGTransformNode` matrix change, so they never re-engrave.
* **Engraving backend:** LilyPond 2.24+ run as a child process per job.
  `output-attributes` gives every grob a stable `id`/`class` in the SVG.
* **Audio backend:** PipeWire (`pw_filter` with RT data loop via RTKit) as
  primary, JACK as an alternative, ALSA as a fallback. All three just call
  `AudioEngine::process(out, frames, callbackTimeNs, outputLatencyFrames)`.

---

## 2. Domain data model

### 2.1 Hierarchy

```
Project
 ├─ ProjectMeta (title, composer, sample rate)          ElementId allocator: next_id
 ├─ Player*            (logical human)                  "Flute 1"
 │   └─ Instrument*    (doubling set)                   Flute, Piccolo …
 │        └─▶ TrackId  (which DAW lane it sounds through)
 ├─ Track*             (DAW lane / channel strip)
 │        └─▶ NodeId + note port + MIDI channel  (CLAP instrument in the graph)
 ├─ AudioGraph         nodes (CLAP plugin | bus | master) + edges = routing matrix
 └─ Movement*          (flow: independent block of time)
     ├─ tempo / meter / key maps
     ├─ PlayerLane*    one per participating player
     │   ├─ InstrumentChange[]   active instrument from position X
     │   └─ Voice[1..4] ──▶ shared_ptr<const EventList>     (Note | Rest | Chord)
     └─ AutomationLane* ──▶ shared_ptr<const vector<AutomationPoint>>
                           target: (NodeId, clap_id param, Value|Modulation, note_id)
```

The design lets mid-movement instrument changes, multi-voice writing and DAW
routing work without special cases:

* Notes never store a track. The **snapshot compiler** resolves each note at its
  onset: `PlayerLane::instrumentAt(start) → Instrument::track → Track → (node, port, channel)`.
  A doubling change at bar 37 therefore switches plugins at exactly bar 37,
  in playback, in the engraved instrument label and in the click → track mapping.
* Voices hold **sounding (concert) pitch** with explicit spelling (`step`,
  `alter`, `octave`). Transposition only applies when engraving transposed parts.
* Musical time is an exact rational `Fraction` (whole notes) so tuplets and
  grace timings are lossless. Samples appear only in the compiled snapshot.

### 2.2 C++ header blueprint (condensed from `include/corinzia/model/Model.hpp`)

```cpp
namespace corinzia {
struct ElementId { std::uint64_t value; std::string toSvgId() const; /* "e-<base36>" */ };
template <class Tag> struct TypedId { ElementId id; };
using MovementId = TypedId<struct MovementTag>;   // … PlayerId, InstrumentId, TrackId,
using VoiceId    = TypedId<struct VoiceTag>;      //     EventId, LaneId, NodeId
struct Fraction { std::int64_t num, den; };       // exact musical time, whole notes
}

namespace corinzia::model {
struct Pitch      { int8_t step, alter, octave; int midiKey() const; };
struct NoteHead   { EventId id; Pitch pitch; bool tieToNext; uint8_t velocity; };  // id → SVG + CLAP note_id
struct MusicEvent { EventId id; EventKind kind; Fraction start, duration;
                    std::vector<NoteHead> heads; uint16_t articulations; std::optional<uint8_t> dynamicMark; };
using  EventList = std::vector<MusicEvent>;

struct Voice          { VoiceId id; uint8_t index; std::shared_ptr<const EventList> events; };   // COW
struct AutomationTarget { NodeId node; uint32_t clapParamId; AutomationMode mode; int32_t noteId; };
struct AutomationLane { LaneId id; AutomationTarget target;
                        std::shared_ptr<const std::vector<AutomationPoint>> points; };          // COW

struct Instrument { InstrumentId id; PlayerId owner; std::string name, engraverName, clef;
                    int transpositionSemis; TrackId track; };          // ── DAW linkage ──▶ Track
struct Player     { PlayerId id; std::string name; std::vector<std::unique_ptr<Instrument>> instruments; };
struct Track      { TrackId id; std::string name; NodeId instrumentNode; uint16_t notePort;
                    int16_t midiChannel; float gainDb, pan; bool mute, solo; NodeId outputBus; };
struct AudioNode  { NodeId id; NodeKind kind; std::string clapBundlePath, clapPluginId;
                    std::shared_ptr<const std::vector<std::byte>> stateChunk; bool bypassed; };
struct AudioGraph { std::vector<AudioNode> nodes; std::vector<AudioEdge> edges; };

struct PlayerLane { PlayerId player; std::vector<InstrumentChange> changes; std::vector<Voice> voices;
                    InstrumentId instrumentAt(Fraction) const; };
struct Movement   { MovementId id; std::vector<TempoEvent> tempo; std::vector<MeterEvent> meter;
                    std::vector<KeyEvent> keys; std::vector<PlayerLane> lanes;
                    std::vector<AutomationLane> automation; Fraction length; uint64_t revision; };
struct MovementSnapshot { std::shared_ptr<const Movement> movement; uint64_t revision; };

class Project {
public:
    std::vector<std::unique_ptr<Player>>   players;
    std::vector<std::unique_ptr<Track>>    tracks;
    AudioGraph                             graph;
    std::vector<std::unique_ptr<Movement>> movements;
    ElementId allocateId();
    const Locator* locate(ElementId) const;     // O(1): SVG id → movement/player/voice/track
    MovementSnapshot snapshot(MovementId) const; // O(#voices), for workers
};
}
```

### 2.3 Mutation, undo and fan-out

`Command::apply/revert` return a **`ChangeSet`** (dirty movements, musical
ranges, voices, lanes, and whether routing changed). The GUI thread fans it out:

| Consumer | Receives | Latency target |
|---|---|---|
| `ScoreCanvas::showProvisional` | dirty `[from,to)` + edited movement | next frame (≤ 16 ms) |
| `SnapshotCompiler` (worker) | all movement snapshots + `RoutingSnapshot` | < 5 ms typical, then RT swap |
| `EngravingService` | dirty movement snapshot | debounce 350 ms + compile (~1 s) |
| `PersistenceWorker` | `SaveBatch` (dirty voices only) | debounce ~2 s, coalesced |

`UndoStack` supports command merging (dragging a note through five pitches is
one undo step) and macros ("paste 32 bars"). Because `EventList` is COW,
`ReplaceVoiceEvents` stores *before* and *after* as two `shared_ptr`s, so undo
restores the identical object (tested: `undo_redo_and_merge`).

---

## 3. Real-time audio and CLAP integration

### 3.1 Boundary between the model and the RT thread

```
 GUI thread                     worker                         RT audio thread
 ──────────                     ──────                         ───────────────
 Command → ChangeSet ──▶ compileSnapshot(movements, routing)
                          └ new PlaybackSnapshot (heap)
 post(SwapSnapshot{ptr}) ───────────── SpscQueue<UiToAudio,1024> ──▶ handleCommands()
                                                                      swapSnapshot(next)
 collectGarbage(): delete ◀──────────── SpscQueue<AudioToUi,4096> ◀── RetireSnapshot{old}
 playhead() ◀─────────────────────────── Seqlock<PlayheadState>  ◀── store() each block
 ParamChanged/Gesture → automation rec ◀─ SpscQueue<AudioToUi>    ◀── plugin out_events
```

* `SpscQueue<T,N>` (`rt/SpscQueue.hpp`) is a bounded wait-free ring with
  cache-line-separated head/tail and cached opposite indices. `T` must be
  trivially copyable (enforced by `static_assert`) so a pop never runs a destructor.
* **Ownership transfer:** the RT thread receives snapshots as raw pointers and
  returns the old one through the telemetry queue. All `delete` calls happen on
  the GUI thread. If that queue is ever full, the engine **leaks** one snapshot
  and counts it, because freeing on the RT thread is worse.
* `Seqlock<T>` (`rt/Seqlock.hpp`) publishes the playhead once per block.
  Readers retry on a torn read and the writer never waits.
* Everything the RT thread touches is preallocated by `AudioEngine::prepare()`:
  256 node slots, each with a 2048-event CLAP input list and stereo buffers.

### 3.2 `PlaybackSnapshot`: the flattened render plan

Built off-thread by `compileSnapshot()`:

* **Sections**: movements laid end to end in samples (default 2 s gap), each
  with its own `TempoMap` (constant segments plus linear ramps, solved
  analytically in both directions, O(log n), allocation-free).
* **Notes**: sorted `ScheduledNote{at, nodeIndex, noteId, port, channel, key, edge, velocity, element}`.
  Ties are merged into one sounding note. The gate comes from articulation
  (staccato 50 %, tenuto 100 %, otherwise 90 %). Velocity comes from the last
  dynamic mark (accent ×1.15) unless the head has an explicit velocity.
  `noteId = ElementId & 0x7fffffff`.
* **Automation**: sample-positioned breakpoints per lane, with curve shape.
* **Graph**: nodes in topological order (Kahn), edges as index pairs, plugin
  pointers resolved, and `graphLatency` = the longest-latency path into the master.

### 3.3 CLAP plugin lifecycle (`audio/ClapPluginHost.*`)

| Step | Thread | Call |
|---|---|---|
| Load bundle | main | `dlopen(RTLD_NOW|RTLD_LOCAL)` → `clap_entry->init(path)` → `get_factory(CLAP_PLUGIN_FACTORY_ID)` |
| Instantiate | main | `factory->create_plugin(host, id)` → `plugin->init()` |
| Discover | main | `get_extension`: params, state, latency, audio-ports, note-ports (gui, timer-support, posix-fd-support for editors) |
| Restore | main | `clap_plugin_state.load(istream)` from `plugin_state.state` |
| Activate | main | `activate(sr, 1, maxFrames)`: the plugin may allocate here |
| Start | audio | `start_processing()` lazily on the first `process()` after activation |
| Render | audio | `process(&clap_process)` every block |
| Stop | audio | `stop_processing()` (triggered by the `BypassNode` command) |
| Deactivate / destroy | main | `deactivate()` → `destroy()`; the library refcount drops → `entry->deinit()` → `dlclose` |

**Host callbacks** (`request_restart`, `request_process`, `request_callback`,
`params.request_flush`, `latency.changed`) may arrive on any thread. They only
set atomics. `pumpMainThread()` (GUI 60 Hz timer) calls `on_main_thread()` and
reports restarts. `clap_host_thread_check` is answered from thread-local role
flags.

**Hot reconfiguration handshake** (latency change, plugin removal, re-activation):

```
GUI: post(BypassNode n) ─▶ RT: stop_processing(); bypassed=true; push NodeBypassed(n)
GUI: on NodeBypassed  ─▶ deactivate(); activate(sr,…) / or rebuild snapshot without n
GUI: post(SwapSnapshot) + post(UnbypassNode n)
GUI: destroy the instance only after the snapshot that referenced it was retired
```

### 3.4 Sample-accurate event translation (`AudioEngine::process`)

For each block `[pos, pos+frames)`, split at the loop end if one is crossed:

1. **Notes:** advance `noteCursor_` while `notes[i].at < end`. Push
   `CLAP_EVENT_NOTE_ON/OFF` into the target node's list with
   `header.time = at − blockStart + subBlockOffset`, carrying `note_id`,
   `port_index`, `channel`, `key` and `velocity`. Track sounding notes in a fixed table.
2. **Automation**, per lane:
   * `AutomationMode::Value` → `CLAP_EVENT_PARAM_VALUE`, which *is* the
     parameter's value: tempo-synced filter sweeps, mixer moves.
   * `AutomationMode::Modulation` → `CLAP_EVENT_PARAM_MOD`, a non-destructive
     offset on top of the value. With `note_id ≥ 0` it is **polyphonic
     per-note modulation**, e.g. vibrato on one sustained note of a chord.
   * Breakpoints are hit on their exact sample. Curved segments are sampled every
     32 samples, flat ones not at all, and unchanged values are never re-sent.
   * Per-note expressive data (dynamics swells, pitch bends from notation) uses
     `CLAP_EVENT_NOTE_EXPRESSION` through the same list (`pushNoteExpression`).
3. **Ordering:** CLAP requires time-sorted input. Sources are interleaved, so
   each list is sorted once with `std::sort` (introsort, no allocation). Ties
   resolve as note-off/choke → param values → note-on → per-note modulation, so
   a retrigger at the same sample is never swallowed and a note-on sees the
   right parameter values.
4. **Transport:** one `clap_event_transport` per block: `song_pos_beats` and
   `bar_start` in `CLAP_BEATTIME_FACTOR` fixed point (continuous across
   movements), `song_pos_seconds`, tempo, time signature, `bar_number` (0-based),
   and playing/loop flags. `clap_process.steady_time` is a separate monotonic
   counter that never jumps on seek or loop.
5. **Graph:** nodes run in topological order. Each node's input is the sum of
   its incoming edges × gain. Plugins render into preallocated buffers, then node
   gain is applied and master nodes sum into the device buffer.

**Swap, seek and loop semantics** (tests: `engine_snapshot_swap_releases_deleted_notes_only`, `engine_loop_and_seek`):

* On **snapshot swap** while playing, a sounding note survives only if the new
  plan still contains its note-off (same `note_id`, same node, found by id).
  Otherwise it gets a note-off at offset 0. Editing bar 40 never cuts the
  chord ringing in bar 12. Deleting the ringing chord silences it at once.
* **Seek/stop** releases everything. **Loop wrap** releases at the wrap offset
  and re-seeks the note cursor inside the same block.
* Event-list overflow never blocks: drops are counted and reported as `EventsDropped`.

### 3.5 Latency-compensated playhead (`rt/PlayheadTracker.hpp`)

The RT thread publishes per block: `blockStartSample`, `callbackTimeNs`
(`CLOCK_MONOTONIC`), `blockFrames`, `outputLatencyFrames` and `graphLatencyFrames`.
Output latency comes from the backend: PipeWire `pw_stream_get_time_n()`
(`delay` + `buffered`), or JACK `jack_port_get_latency_range(…, JackPlaybackLatency)`.

On every rendered frame (QQuickWindow `beforeRendering`, or a 60 Hz GUI timer):

```
heard(now) = blockStartSample
           + clamp((now − callbackTimeNs)·sr, 0, 2·blockFrames)  // extrapolate inside/after the block
           − outputLatencyFrames                                 // DAC + driver quantum
           − graphLatencyFrames                                  // plugin delay compensation
```

A first-order tracking filter removes block-quantisation jitter, but it
**snaps** on seeks or jumps larger than max(4 blocks, 50 ms). Clamping the
extrapolation to two blocks makes the cursor stall with the audio on an xrun
instead of running ahead. Then:

```
sample ──Section.tempo.quarterAtSample──▶ quarter ──LayoutAnchors::cursorAt──▶ (page, x, systemTop, systemBottom)
```

`cursorAt` interpolates x between the engraved onsets on either side (same
system), so the cursor glides across bars with uneven spacing. It is tested
against the real LilyPond output (`engraving_service_definitive_pass_and_hit_testing`).

### 3.6 Delay compensation, MIDI fallback, plugin management, backend

* **PDC.** The compiler gives every edge `delay = slowest input of its summing
  node − this edge's path latency`. The engine applies it with a pool of 64
  preallocated ring buffers (16384 samples each). Lines are assigned to edges
  by identity `(fromId, toId)` on each snapshot swap, so an edit never empties
  a delay line, which would click (`engine_pdc_aligns_parallel_paths`).
* **MIDI-dialect plugins.** If a plugin's note port lacks
  `CLAP_NOTE_DIALECT_CLAP`, note events are rewritten in place as
  `CLAP_EVENT_MIDI` (0x9n/0x8n, velocity ×127). `note_id` is lost, so per-note
  modulation needs the CLAP dialect (`engine_midi_dialect_fallback`).
* **`PluginManager`** (`audio/PluginManager.*`) scans `CLAP_PATH`, `~/.clap`,
  the NixOS profile directories (`~/.nix-profile/lib/clap`,
  `/etc/profiles/per-user/$USER/lib/clap`, `/run/current-system/sw/lib/clap`),
  `/usr/lib/clap` and `/usr/local/lib/clap`. It owns the instances and runs
  the §3.3 handshakes: `request_restart` → bypass → ack → reactivate →
  unbypass; removal → bypass → ack → graveyard → destroy once a newer snapshot
  generation is live. Tested against a real `dlopen`ed `.clap` bundle built
  from the test fixture.
* **PipeWire backend** (`audio/PipeWireBackend.*`) is a `pw_filter` with
  declared DSP ports `output_FL/FR`, which patchbays and the session manager
  can link immediately (a `pw_stream` only gets ports once a session manager
  configures it). The callback runs on PipeWire's RT data thread. Timing comes
  from `spa_io_position.clock`; output latency is
  `max(|clock.delay|, port SPA_PARAM_Latency)`; xruns are counted from
  `SPA_IO_CLOCK_FLAG_XRUN_RECOVER`. Tested end-to-end against a private
  headless daemon, also inside the Nix sandbox. JACK clients are covered by
  `pw-jack`. Without PipeWire, the app falls back to a silent dummy driver
  thread that keeps transport and playhead real-time.
* **Not yet:** plugin GUI embedding (`clap.gui` X11/Wayland, timer and
  posix-fd host extensions); note-expression lanes in the UI.

---

## 4. Storage layer (`schema/project.sql`, `storage/ProjectStore.*`)

### 4.1 Container format

A `.project` file is a single SQLite database (`application_id = 'CORZ'`,
`user_version` = schema version) in **WAL** mode with `synchronous=NORMAL`.
An application crash can't lose a committed save; only a power loss can drop
the last one. Tables, with every key being an ElementId:

| Group | Tables | Notes |
|---|---|---|
| Global | `project` | meta, `next_id`, JSONB house-style settings |
| Routing / mixer | `audio_node`, `audio_edge` (routing matrix, `WITHOUT ROWID`), `track`, `mixer_layout` (JSONB) | upsert + delete-missing; never delete-all, because cascades would wipe plugin state |
| Plugin state | `plugin_state(node_id, state BLOB, state_hash)` | isolated so a metadata change never rewrites MBs; skips the write when the hash is unchanged |
| Orchestra | `player`, `instrument(track_id → track)` | |
| Movements | `movement` (tempo/meter/keys JSONB), `player_lane`, `instrument_change`, `voice` | |
| Music payload | `voice_chunk(voice_id, chunk_index, events JSONB)` | **chunked**: span = 4 whole notes |
| Automation | `automation_lane`, `automation_chunk(points JSONB)` | same chunking |
| History / cache | `undo_journal` (forward/inverse JSONB, per-movement scope), `engraving_cache` (SVG + revision + engraver version) | |

Deleting a movement cascades to its lanes, voices, chunks, automation, undo
history and engraving cache, but never to players, routing or plugin state.
`scripts/validate_schema.py` checks this.

### 4.2 Fast, non-blocking auto-save

```
GUI: ChangeSet ─▶ SaveBatch{ movements: [MovementSnapshot], dirtyVoices, header?, pluginStates }
                       │  (built in O(#voices): COW pointers only)
                       ▼
PersistenceWorker::enqueue ── merges into the pending batch if the writer is busy
                       ▼  (own thread, own connection)
ProjectStore::save: BEGIN IMMEDIATE
   upsert movement/lane/voice rows
   for each dirty voice: serialize events per chunk →
      INSERT … ON CONFLICT DO UPDATE SET events=excluded.events
                         WHERE voice_chunk.events IS NOT excluded.events   ◀── unchanged chunks cost nothing
   COMMIT
```

Editing one note in a 300-bar movement rewrites one ~1–4 KB row (tested with
triggers: `storage_incremental_save_touches_one_chunk`). Loading needs no JSON
library: SQLite decodes the JSONB itself (`json_each`, `->>`), and the C++ side
just reads columns. CLAP state is saved on the main thread
(`clap_plugin_state.save` is `[main-thread]`) when the plugin calls
`state.mark_dirty` or before a manual save, then handed to the writer as bytes.

### 4.3 Granular, transactional undo/redo

* The in-memory `UndoStack` is the authority. Each step's command is also
  appended to `undo_journal` with `forward`/`inverse` JSONB in the same
  transaction as its data, so history survives reopening and can be scoped per
  movement (`WHERE movement_id = ?`): "undo in this movement only".
* Redo-tail truncation is mirrored (`undone = 1` rows are deleted on a new step).
* Undo is reconstructed from commands, not from database diffs. The database is
  durable state, not the undo mechanism, so undo never waits on I/O.

---

## 5. The interactive rendering pipeline: the dual-layer illusion

### 5.1 Timeline of one edit

```
t=0 ms    key press "↑" ─▶ Command ─▶ ChangeSet{range: bar 37 beat 1}
t<1 ms    ScoreCanvas::showProvisional(edited, from, to):
             masks   = anchors.staleRegions(from,to)      paper-coloured rects over stale glyphs
             glyphs  = anchors.provisional(edited,from,to) (x from engraved onsets, y from fitted pitch line)
          SnapshotCompiler job queued ─▶ RT swap (you hear the new pitch on the next block)
t≈16 ms   render thread: one QSGGeometryNode draw for all provisional glyphs
t=350 ms  debounce expires (restarted by every further edit) ─▶ EngravingService::launch
t≈1.3 s   worker: emit .ly → lilypond -dbackend=svg → index + anchors + raster
          (killed within ~30 ms if a newer revision arrives; stale results discarded)
t≈1.3 s   GUI: setEngraving(result) ─▶ textures swapped, overlay cleared
```

### 5.2 Instant-feedback pass (`ui/LayoutAnchors`, `ui/ScoreCanvas`)

No font metrics, no collision avoidance and no respacing. It uses only the
geometry of the last definitive engraving:

* **x:** if the onset still exists (pitch or articulation edits), use its
  engraved x exactly. New onsets interpolate between the engraved onsets around
  them in the same system, by musical time.
* **y:** a least-squares fit `y = c₀ + c₁·diatonicStep` over that lane's engraved
  noteheads in the same system. Clef, staff size and transposition are absorbed
  by the fit (LilyPond: c₁ ≈ −0.5 staff spaces per step). Tested: an A5→C6
  edit lands within 0.3 staff spaces of the engraved C6.
* Drawn as textured sprites (one pre-rendered notehead/rest image shared by
  pooled `QSGSimpleTextureNode`s) over paper-coloured `QSGSimpleRectNode`
  masks covering the stale regions. Sprites rather than custom geometry,
  because Qt's software adaptation (used headless and on machines without a
  GPU) ignores custom `QSGGeometryNode`s.
* **Scene-graph rule learned the hard way:** the software adaptation visits a
  node the moment it is appended and keeps what it saw. So every node is fully
  configured *before* insertion, overlay nodes are persistent (hidden with an
  empty rect, never deleted per frame), and page textures are swapped in
  place. Pixel-level tests guard this (`score_canvas_renders_engraving_pixels`).

### 5.3 Asynchronous definitive pass (`ui/EngravingService`)

* Debounced per movement. One job is in flight per movement, and edits during
  a compile coalesce into exactly one follow-up.
* The job emits LilyPond text from the immutable snapshot
  (`engrave::emitLilyPond`), pipes it to `lilypond -dbackend=svg -dno-point-and-click -s -o <tmp> -`,
  and polls `waitForFinished(30)` so a newer revision can `kill()` it.
* On the worker it builds the `SvgHitIndex`, the `LayoutAnchors` and a raster of
  every page (`QSvgRenderer` → `QImage`). The GUI thread only uploads textures.
  Zoom scales the textures on the GPU immediately. A re-raster at the new zoom
  after zooming settles is specified but not implemented yet.
* Results carry `(movement, revision)`. Anything older than the latest request is
  dropped (tested: `engraving_service_drops_stale_revisions`).
* Planned: `engraving_cache` stores the last good SVG so reopening a project
  shows definitive engraving immediately while a background pass revalidates it.

### 5.4 Point-and-click: embedding structural IDs

The emitter tags every engraved object through LilyPond's `output-attributes`:

```lilypond
\tweak output-attributes #'((id . "e-2s") (class . "ev note v-1k t-9")) a''4-.
< \tweak output-attributes #'((id . "e-2u") (class . "ev head v-1k t-9")) e''
  \tweak output-attributes #'((id . "e-2v") (class . "ev head v-1k t-9")) g'' >2
```

The SVG backend wraps each grob as `<g id="e-2s" class="ev note v-1k t-9">…</g>`.
Then:

1. `SvgHitIndex::addPage` streams the SVG (`QXmlStreamReader`), collects `e-*`
   groups, gets exact bounds from `QSvgRenderer::boundsOnElement(id)` (viewBox
   units = staff spaces), decodes `v-` (voice) and `t-` (DAW track) from the
   class, and buckets entries into an 8×8-staff-space grid.
2. On click, `ScoreCanvas::toPage` inverts pan/zoom/page stacking, and `hitTest`
   returns the **innermost** containing entry (a head beats its chord).
3. `elementClicked(elementId, voiceId, trackId)` gives the UI enough to select
   the notehead, open the voice, and highlight the mixer strip or plugin.
   `Project::locate(id)` gives the full `Locator` (movement, player, voice,
   track) in O(1).

Chord heads get individual ids, so a click selects one note of a chord. The
same id is that note's CLAP `note_id`, so selection, playback highlighting
(`ScheduledNote::element`) and per-note modulation lanes all refer to the same
thing. Tested end-to-end with real LilyPond output: every head is found, the
centre-click resolves to that head, and its track is correct before *and after*
the Flute→Piccolo change (`score_canvas_click_maps_to_element_and_track`).

### 5.5 Note input

Writing notes follows the Sibelius/Dorico overwrite model and reuses every
pipeline above. No special rendering path exists for input.

* **Editing core** (`edit/NoteInput.*`, Qt-free, unit-tested): pure functions
  from an `EventList` to a new COW `EventList`. They cover overwrite insertion
  (covered events removed, partly covered ones truncated), voice-1 contiguity
  (remainders and gaps become rests, voices 2–4 may have holes), chords,
  delete (a chord loses one head, a note becomes a rest), articulation, tie
  and dynamic changes, triplets via `DurationSpec`, and key-aware spelling
  with the octave nearest the previous note.
* **Commands:** each entered item is one `ReplaceVoiceEvents` (non-merging),
  wrapped in a macro with `AddVoice` (typing into a new voice) and
  `SetMovementLength` (typing past the final barline adds whole bars). One
  undo removes exactly one entered item.
* **Caret:** `(lane, voice, position)`. It follows the selection and is drawn
  on its lane's staff by `LayoutAnchors::caretAt`.
* **Mouse entry:** `LayoutAnchors::locate(page, point)` inverts the engraved
  geometry. The system comes from y; the beat comes from x via the
  piecewise-linear onset map; the staff and pitch come from the per-lane
  `y = c₀ + c₁·diatonic` fit. The beat snaps to the current duration grid.
  Shift-click adds to a chord.
* **Feedback:** the provisional layer (§5.2) shows the new note on the next
  frame; the snapshot swap makes it audible; the note is auditioned through its
  track's plugin (`PreviewNote` by `NodeId`). The definitive engraving follows.
* **Engraving support:** triplets are emitted as `\tuplet 3/2 { … }`, and the
  Completion engravers split notes and rests that cross barlines into ties.

| Key | Action | Key | Action |
|---|---|---|---|
| N / Esc | input on / off | A–G | note (Shift = add to chord) |
| 1–7 | 64th … whole | . | dot (cycles double dot) |
| ; | triplet | R or , | rest |
| − / = / 0 | flat / sharp / natural for the next note | Q | chord mode |
| T | tie | V | next voice |
| [ ' ] \\ | staccato, accent, tenuto, marcato | ←/→ | move caret |
| ↑/↓ (Ctrl) | transpose selection (octave) | Alt+↑/↓ | previous / next staff |
| Backspace | delete before the caret | Del | delete selection |

The left **Notes panel** (`qml/NotesPanel.qml`) mirrors all of these with
drawn duration icons, accidentals, articulations, dynamics and voices. Its
buttons never take keyboard focus, so typing always reaches the score.

### 5.5a Selection, clipboard, slurs and hairpins

* **Selection model:** a single element (click) or a **passage** (lanes
  `[a, b]` × beats `[from, to)`). A passage comes from a **marquee** (drag on
  empty paper; the band selects every engraved element it touches and the
  union of their staves and spans becomes the passage), from **Shift+click**
  (extends from the current selection), or from **Ctrl+A**. The highlight is
  one rectangle per system (`LayoutAnchors::rangeRects`).
* **Passage operations** (pure `edit::` functions, one undo step each):
  `eraseRange` (Delete; voice 1 refills with rests), `sliceRange` +
  `pasteEvents` (Ctrl+C/X/V with fresh ids, extending the movement when
  needed; pasting at a selected note puts it *at* that note), repeat (R),
  `transposeRange` (↑/↓), articulations (added unless every note already has
  them, then removed) and dynamics (first note per staff).
* **Spanners** are start/stop marks on events (`SpanMark`), as in LilyPond and
  MusicXML, so copy/paste, undo, chunked storage and engraving carry them for
  free. S adds a slur; H / Shift+H add a crescendo / diminuendo. Playback:
  `compile::expressiveShape` ramps velocity over a hairpin towards the next
  written dynamic (or ±2 steps) and plays slurred notes legato.

### 5.5b Zoom quality and score views

* **Vector-sharp zoom** (`ui/VectorTiles`): visible 512 px tiles are
  rasterized on a worker thread straight from the engraving's SVG vectors, at
  the exact device scale. While zooming, the previous image is scaled
  (instant); crisp tiles replace it as soon as they are ready. Tested
  pixel-exact against direct vector rendering at 4×.
* `normalizeSvgText` fixes LilyPond's tiny text font sizes, at which FreeType
  silently dropped glyphs when zoomed in.
* **Page | Panorama** toggle (Score toolbar): Page is the printed layout;
  Panorama engraves the movement as one endless system
  (`ly:one-line-auto-height-breaking`, staff names kept) and the wheel scrolls
  through time.

### 5.5c Score structure and text

* **Model.** A movement's non-voice content is its *header*: tempo (with
  optional words, "Allegro"), meter and key maps; special barlines (double,
  final, start/end/both repeats); lettered rehearsal marks; per-staff clef
  changes and text (expression: italic, below; technique: above). It is edited
  as one undoable unit (`EditMovementHeader`) and stored in the
  `movement.notation` JSONB column (schema v2, migrated in place from v1).
* **Popovers** (Dorico letters, `qml/Popover.qml`): Shift+K key ("Eb", "f#" =
  F♯ minor, "a minor"), Shift+M time ("6/8", "C", "C|"), Shift+C clef,
  Shift+B barline ("||", "|.", "|:", ":|", ":|:"), Shift+T tempo ("Allegro",
  "Andante 84", "q=96"), Shift+X expression, Shift+P technique, Shift+A
  rehearsal mark. In note input Shift+A/B/C stay chord notes, so use the
  Structure panel for those. Parsing is pure and tested (`edit/Structure`);
  text it doesn't understand keeps the popover open with a hint. Position:
  passage start, else the selected note, else the caret. Key, time and marks
  snap to the bar; barlines to the nearest bar line.
* **Structure panel** (right of the score): the same actions as buttons, and
  **insert/delete bars** (Ctrl+B inserts a bar before the current one;
  Ctrl+Shift+Backspace deletes the selected bars). Music, maps, marks, clefs
  and text move together in one undo step. A barline at the cut ends the
  previous bar and stays.
* **Point-and-click**: barlines aside, structure items carry their ElementId
  into the SVG. Click one and press Delete to remove it.
* **Engraving**: `\bar`, `\mark \default`, `\tempo "Allegro" 4 = 132`, clefs and
  text go in a spacer voice per staff. Qt SVG strips the leading spaces in
  LilyPond's text runs, so `normalizeSvgText` shifts those runs by the space's
  width instead ("Allegro (♩ = 132)").
* **Playback**: `compile::playbackOrder` unrolls repeats into written spans.
  Each Section's tempo/meter maps run over performed time, and
  `Section::writtenQuarter` / `performedQuarter` map the playhead back into the
  score and seek to the first pass.
* **Provisional layer and mouse entry** work in clef-relative ("treble
  equivalent") steps. One fit per system therefore spans a mid-system clef
  change; the tests fail if the shift is disabled.
* Known gaps: no voltas or D.C./D.S. yet; insert/delete bars does not move
  automation points.

### 5.5d Gap analysis vs Sibelius / Dorico / MuseScore (roadmap)

| Area | Have | Missing (in priority order) |
|---|---|---|
| Input | step entry (keys, mouse, panel), chords, rests, dots, triplets, accidentals, ties, voices 1–4, overwrite | insert mode; MIDI-keyboard step input and real-time recording; re-input pitches over rhythms; grace notes; general tuplets (5:4, 7:8 …); hover ghost note; percussion & tab input |
| Selection & editing | click, Shift+click, marquee, select all; copy/cut/paste/repeat/delete/transpose passages; undo of everything | filters (select only voice 2 / only dynamics); multi-selection of non-contiguous items; drag notes with the mouse; respell/enharmonic; transpose by interval with accidental spelling; swap voices; explode/reduce |
| Notation items | articulations, dynamics, slurs, crescendo/diminuendo hairpins, ties, tempo words + metronome + rit./accel., expression and technique text, rehearsal marks | lyrics, chord symbols, ornaments (trills, mordents, turns), tremolos, arpeggios, glissandi, pedal, octave lines (8va), fingering, breath marks / caesuras |
| Structure | multi-movement, instrument changes, key/time/tempo/clef changes via popovers and the panel, barline types, repeats (played back), insert/delete bars | pickup bars; voltas, D.C./D.S./Coda; multi-bar rests; system text |
| Instruments | players with doublings, transposition data | add/remove/reorder players; grand staff (piano), divisi, percussion kits, unpitched staves; transposed vs concert-pitch toggle; staff-type changes |
| Layout & parts | Page and Panorama views, LilyPond engraving | system/page breaks, staff spacing, beam & stem overrides, cue notes, part extraction (layouts), house styles, condensing |
| I/O | own SQLite project | MusicXML import/export, MIDI import/export, PDF/SVG/PNG export, audio export, printing |
| Playback | dynamics, hairpins, slurs (legato), articulations gate/accent, PDC, CLAP | expression maps / key switches per articulation, swing, voltas and jumps in playback, humanize, mixer automation from notation |

### 5.6 Two workspaces: Score and Mix

The notation and DAW sides are separate tabs over one project. Both share the
model, undo stack, selection, transport and playhead. Ctrl+1 and Ctrl+2
switch; the shared transport toolbar sits above the tabs.

* **Score** (`ScoreCanvas` + `NotesPanel`): engraved notation, note input, the
  §5.2 provisional layer and point-and-click.
* **Mix** (`qml/MixView.qml`):
  * an **arrange view** (`ui/ArrangeView`): one lane per DAW track, a bar
    ruler (click to locate), note blocks scaled by pitch within each lane,
    the shared selection and the playhead. Ctrl+wheel zooms, wheel scrolls;
  * **mixer strips** (fader through the undoable `SetTrackGain`, CLAP
    instrument assignment);
  * the **plugin browser** (scan results, rescan).

  Blocks are placed by the same rule as playback: each note goes on the track
  that sounds it at its onset, so a Flute→Piccolo doubling shows up on the
  Piccolo lane. Clicking a block selects the note in both tabs. Clicking empty
  timeline locates the transport sample-exactly through the tempo map, and
  Stop returns there, as in a DAW.
* Keyboard focus follows the visible tab. Leaving the score ends note input.

**Scene-graph rule** (software adaptation, verified by tests that fail without
it): inserting nodes into an already-rendered subtree can leave them without
the item's transform or clip, so they draw at window coordinates. Both views
therefore:
1. pool nodes per colour or material, fixed for each node's lifetime;
2. change only rects and textures per frame;
3. rebuild the whole tree, fully built before returning it, when it has to
   grow (more pages, a new track colour, a pool overflow; capacities double).

`arrange_view_pixels_track_content_growth_and_scroll` and
`score_canvas_grows_from_one_to_two_pages` check this at pixel level with the
item offset inside the window.

---

## 6. Thread synchronization diagram

```mermaid
sequenceDiagram
    autonumber
    participant UI as GUI thread
    participant W as Worker pool
    participant LP as lilypond (process)
    participant P as Persistence thread
    participant RT as RT audio thread
    participant PL as CLAP plugin

    UI->>UI: Command.apply() → ChangeSet
    UI->>UI: ScoreCanvas.showProvisional() (next frame)
    UI->>W: compileSnapshot(MovementSnapshots, RoutingSnapshot)
    W-->>UI: PlaybackSnapshot* (queued signal)
    UI->>RT: SpscQueue<UiToAudio>: SwapSnapshot(ptr)
    RT->>RT: swapSnapshot(): keep/release sounding notes
    RT-->>UI: SpscQueue<AudioToUi>: RetireSnapshot(old) → delete on UI
    loop every block (e.g. 256 frames)
        RT->>PL: process(clap_process{in_events: NOTE_ON/OFF, PARAM_VALUE/MOD, transport})
        PL-->>RT: out_events (PARAM_VALUE, GESTURE_*)
        RT-->>UI: Seqlock<PlayheadState>.store()
        RT-->>UI: SpscQueue<AudioToUi>: ParamChanged (automation recording)
    end
    UI->>UI: PlayheadTracker.update() → TempoMap → LayoutAnchors.cursorAt()
    UI->>UI: debounce 350 ms
    UI->>W: EngravingService job (snapshot, revision r)
    W->>LP: stdin: generated .ly
    UI->>W: newer revision r+1 requested
    W->>LP: kill() (stale)
    W->>LP: re-run with r+1
    LP-->>W: page SVGs with <g id="e-…">
    W-->>UI: EngraveResult{pages, SvgHitIndex, LayoutAnchors, rasters}
    UI->>UI: setEngraving() → texture swap, overlay cleared
    UI->>P: SaveBatch (dirty voices, COW snapshots)
    P->>P: BEGIN; upsert changed chunks only; COMMIT (WAL)
```

### Shared-state matrix

| Data | Writer | Readers | Mechanism |
|---|---|---|---|
| `Project` | GUI | GUI | single-thread ownership |
| `MovementSnapshot` | GUI (creates) | workers, persistence | immutable + `shared_ptr` refcount |
| `PlaybackSnapshot` | worker (creates) | RT | raw-pointer handoff via SPSC; freed on GUI |
| RT commands | GUI | RT | `SpscQueue<UiToAudio,1024>` |
| Telemetry / garbage | RT | GUI | `SpscQueue<AudioToUi,4096>` |
| Playhead | RT | GUI / render | `Seqlock<PlayheadState>` |
| Plugin host flags | any (plugin threads) | GUI | `std::atomic<bool>` per instance |
| Engraving results | worker | GUI → render thread | queued signal; read in `updatePaintNode` while GUI is blocked |
| SaveBatch queue | GUI | persistence | mutex + condvar (non-RT; coalescing) |

---

## 7. Serialization formats

`schema/example_project.json` shows every table with realistic rows. The JSONB
payloads use these formats:

```jsonc
// voice_chunk.events: chunk_index = floor(start / 4 whole notes)
[{"id":41,"k":"n","s":[0,1],"d":[1,4],"a":0,"dyn":3,
  "h":[{"id":42,"p":[0,0,5],"t":0,"v":0}]},         // p = [step, alter, octave]; t = tie
 {"id":46,"k":"n","s":[1,2],"d":[1,6],"a":0,        // triplet crotchet → LilyPond 4*2/3
  "h":[{"id":47,"p":[3,-1,5],"t":0,"v":0}]},
 {"id":54,"k":"r","s":[3,2],"d":[1,2],"a":0,"h":[]}]

// movement.tempo
[{"at":[0,1],"bpm":120,"ramp":true},{"at":[1,1],"bpm":96,"ramp":false}]

// automation_chunk.points (curve: step|lin|exp|s)
[{"id":62,"at":[0,1],"v":0.2,"c":"s"},{"id":63,"at":[1,1],"v":0.85,"c":"step"}]
```

---

## 8. Performance budget (targets)

| Path | Budget | How |
|---|---|---|
| Edit → provisional pixels | 1 frame | O(edited events) math + one geometry node |
| Edit → audible change | ≈ one block + compile | snapshot compile is linear in notes; swap is a pointer |
| Zoom / pan | 1 frame | transform node only; textures rescaled on the GPU |
| Definitive engraving | ~1 s per movement (LilyPond start-up dominates) | out of process, debounced, cancellable, per movement |
| Auto-save | ≪ 10 ms per edit | one chunk upsert in a WAL transaction on its own thread |
| RT callback | allocation- and lock-free | preallocated lists and buffers; sort without allocation |

The measured LilyPond compile in the test environment was about 1.05 s for a
small movement. Large movements should be split per movement (already the unit
of work). Engraving several movements in parallel is a natural next step.

---

## 9. Build, test & verification status

All development and testing runs through Nix (`flake.nix`, pinned to the
`nixos-26.05` channel):

```sh
nix develop                      # GCC 15, CMake, Qt 6.11, CLAP 1.2.8, SQLite 3.51, LilyPond 2.26, PipeWire
cmake -S . -B build -G Ninja && cmake --build build
ctest --test-dir build --output-on-failure
python3 -I scripts/validate_schema.py
nix build                        # sandboxed build + full test suite (incl. LilyPond + offscreen Qt)
```

**Verified by tests.** Four ctest suites: core (57 cases, also clean under
ASan + UBSan), PipeWire end-to-end, Qt/UI (14 cases, pixel-level) and an app
smoke run.

| Area | Tests |
|---|---|
| Lock-free primitives | SPSC FIFO/capacity, 1M-item threaded stress, seqlock never tears under contention |
| Tempo map | constant + ramp segments vs closed form, inverse round trip, bars across meter changes |
| Playhead | latency subtraction, extrapolation cap, smoothing vs snap |
| Model | fractions, SVG id round trip, instrument-change resolution, locator → track, undo/redo/merge/macro, COW sharing |
| Engine + CLAP | in-process recorder plugin: exact note on/off samples, tie merge, gates, dynamics; Flute→Piccolo switches plugin **and** channel at the exact sample; automation lands on breakpoints with ≤32-sample resolution; transport beats/bars/tempo; graph mix to master; PDC alignment across swaps; MIDI-dialect fallback; edit-during-playback keeps/releases notes correctly; loop and seek; state save/load |
| Plugin manager | scan + `dlopen` of a real `.clap` bundle; restart and removal handshakes through the RT thread |
| PipeWire | private headless daemon, engine linked to an in-process sink: real-time rate, on-time CLAP events, signal level at the sink |
| Engraving | pitch/duration/tuplet/key/tempo-ramp emission; every head id present in real LilyPond 2.26 SVG output |
| Storage | full round trip; a one-note edit rewrites exactly one chunk; 1 MiB plugin state skipped when unchanged; worker coalescing; undo journal |
| Qt | real engraving through the service; click → head → correct track; cursor x at and between onsets; provisional y within 0.3 spaces; stale-revision dropping; page, playhead and provisional sprites verified at pixel level across re-engraves |
| Note input | overwrite insert/truncate/fill, chords, delete, modify, durations incl. triplets, key-aware nearest-octave spelling; `locate` inverts every engraved head to its staff, beat and pitch; tuplet and barline-completion emission compiled by LilyPond |
| Selection & spanners | eraseRange/slice/paste/applyToRange/transposeRange/markSpan; hairpin velocity ramps and slur legato; spans survive save/load and compile in LilyPond without warnings; smoke: real marquee drag (band + highlight pixels), Ctrl+C/V at a note, R, undo identity, Shift+click passage, S/H, ↑, Ctrl+A + Delete + undo, Esc |
| Structure & text | popover parsers (keys incl. minor and Unicode accidentals, meters, clefs, barlines, tempo words); bar positions across meter changes; header edits; insert/delete bars for voices and header (maps keep their entry at 0, text follows its bar, barlines between bars survive); repeat unrolling with exact note samples and playhead mapping; LilyPond compiles barlines, marks, tempo words, clefs and text without warnings, with clickable ids; v1→v2 file migration; provisional notes and mouse entry after a mid-system clef change; smoke: Shift+K/M/T/X/B popovers incl. an invalid entry and Esc, panel rehearsal mark, click text + Delete + undo, Ctrl+B and Delete bars + undo, survives save/reopen |
| Zoom & views | canvas pixel-exact vs direct SVG rendering at 4×; text present at every zoom; Page/Panorama engrave, names, mouse entry in panorama |
| Mix tab | arrange blocks keep colour/position/clip through swaps, growth, scroll and shrink; score canvas growing 1 → 2 pages |
| App (`corinzia --smoke`) | QML window loads; demo engraved; select + transpose through undo; background re-engrave; transport advances in real time; **keys** N, 4, C D E, Shift+G (chord), R into the canvas; undo of one item; **mouse click** on a clarinet note places an eighth at that pitch and beat; voice 2 created on demand; typing past the end adds a bar (and undo removes it); **panel button** selects durations; **Ctrl+2** Mix tab: one arrange block per note head, doubling on the Piccolo lane, blocks drawn, clicking a block selects the note, clicking the timeline locates the transport sample-exactly; **Ctrl+1** back with score keys working; save to `.project` and reopen; engraved music visible on screen |

**Run the application:** `nix run` (or `corinzia [--no-audio] [file.project]`
from the dev shell). Space plays or stops; ↑/↓ transpose the selected note
(Ctrl for octaves); Ctrl+Z / Ctrl+Shift+Z undo and redo; Ctrl+wheel zooms. The
mixer lets you assign any installed CLAP instrument to a track.

**Still to do** (next milestones):

1. Plugin GUI embedding (`clap.gui`, X11/Wayland) and the timer/posix-fd host extensions.
2. MIDI-keyboard input, a hover "ghost note" in input mode, lyrics and chord symbols, voltas/D.S./Coda; piano roll view.
3. Zoom-settled re-rasterization; engraving cache read path; multi-movement navigation; transposed parts.
4. Command serialization for `undo_journal` replay on open; undoable routing edits (plugin assignment).
5. Note-expression and automation lanes in the UI; automation recording from plugin `ParamChanged`.
