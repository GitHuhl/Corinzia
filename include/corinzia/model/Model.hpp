#pragma once
// Semantic domain model. Pure C++ (no Qt, no audio buffers, no coordinates).
//
// Ownership:
//   Project
//    ├── Players ──owns──> Instruments ──refers──> Track (DAW lane / channel strip)
//    ├── Tracks  ──refers──> AudioGraph Node (CLAP plugin instance) + note port + channel
//    ├── AudioGraph (nodes + edges = routing matrix, plugin state blobs)
//    └── Movements (flows)
//          ├── TempoMap / Meter / Key tracks
//          ├── PlayerLanes (one per participating Player)
//          │     ├── InstrumentChanges  (which Instrument is active from position X)
//          │     └── Voices (1..4) ──> COW EventList (Note / Rest / Chord)
//          └── AutomationLanes ──> COW point list, targeting (Node, CLAP param id)
//
// Thread rules: the Project is owned and mutated ONLY by the UI (main) thread,
// exclusively through Commands (see Commands.hpp). Background workers never
// touch it; they receive immutable MovementSnapshots whose bulky parts are
// shared_ptr<const ...> copy-on-write payloads, so snapshotting is O(#voices).

#include "corinzia/core/Types.hpp"

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace corinzia::model {

// ----------------------------------------------------------------- notation --
struct Pitch {
    std::int8_t step = 0;     // 0=C .. 6=B   (spelling matters for notation)
    std::int8_t alter = 0;    // -2..+2 semitones
    std::int8_t octave = 4;   // scientific pitch octave (C4 = middle C)

    constexpr int midiKey() const noexcept {
        constexpr int kStepSemis[7] = {0, 2, 4, 5, 7, 9, 11};
        return (octave + 1) * 12 + kStepSemis[step] + alter;
    }
};

struct NoteHead {
    EventId id;               // own id: point-and-click target AND CLAP note_id source
    Pitch pitch;
    bool tieToNext = false;
    std::uint8_t velocity = 0; // 0 = derive from dynamics during snapshot compile
};

enum class EventKind : std::uint8_t { Note, Rest, Chord };

enum Articulation : std::uint16_t {
    ArtNone = 0, ArtStaccato = 1 << 0, ArtAccent = 1 << 1, ArtTenuto = 1 << 2,
    ArtMarcato = 1 << 3, ArtFermata = 1 << 4,
    ArtStaccatissimo = 1 << 5,
    // Ornaments (played back as written-out figures, see compile::ornamentFigure).
    ArtTrill = 1 << 6, ArtMordent = 1 << 7, ArtTurn = 1 << 8,
    ArtTremolo = 1 << 9,      // unmeasured tremolo (three strokes)
    ArtArpeggio = 1 << 10,    // rolled chord
    ArtBreath = 1 << 11,      // breath mark after the note
    ArtCaesura = 1 << 12,     // caesura after the note
};

// Spanners as start/stop marks on events (as in LilyPond and MusicXML): they
// travel with the music through copy/paste, undo, chunked storage.
enum SpanMark : std::uint8_t {
    SpanNone = 0, SlurStart = 1 << 0, SlurEnd = 1 << 1,
    CrescStart = 1 << 2, DimStart = 1 << 3, HairpinEnd = 1 << 4,
};

struct MusicEvent {
    EventId id;
    EventKind kind = EventKind::Note;
    Fraction start;                 // offset from movement start, in whole notes
    Fraction duration;              // written duration (tuplet-scaled)
    std::vector<NoteHead> heads;    // empty for rests, 1 for notes, N for chords
    std::uint16_t articulations = ArtNone;
    std::optional<std::uint8_t> dynamicMark;  // e.g. index into pp..ff table
    std::uint8_t spans = SpanNone;            // SpanMark bits
};

using EventList = std::vector<MusicEvent>;    // sorted by start, no overlaps per voice

struct Voice {
    VoiceId id;
    std::uint8_t index = 1;                         // 1..4 (stem direction convention)
    std::shared_ptr<const EventList> events = std::make_shared<EventList>();
};

// --------------------------------------------------------------- automation --
enum class CurveShape : std::uint8_t { Step, Linear, Exponential, SCurve };

struct AutomationPoint {
    EventId id;
    Fraction at;
    double value = 0.0;             // plain parameter value (CLAP domain)
    CurveShape shapeToNext = CurveShape::Linear;
};

enum class AutomationMode : std::uint8_t {
    Value,       // writes the parameter       -> CLAP_EVENT_PARAM_VALUE
    Modulation,  // non-destructive offset      -> CLAP_EVENT_PARAM_MOD
};

struct AutomationTarget {
    NodeId node;                    // plugin node in the audio graph
    std::uint32_t clapParamId = 0;  // clap_id from clap_plugin_params.get_info
    AutomationMode mode = AutomationMode::Value;
    // Optional per-note modulation scope (CLAP polyphonic modulation); -1 = all.
    std::int32_t noteId = -1;
};

struct AutomationLane {
    LaneId id;
    std::string name;
    AutomationTarget target;
    std::shared_ptr<const std::vector<AutomationPoint>> points =
        std::make_shared<std::vector<AutomationPoint>>();
};

// --------------------------------------------------------------- time maps --
struct TempoEvent { Fraction at; double quarterBpm = 120.0; bool rampToNext = false; std::string text{}; };   // text: "Allegro"
struct MeterEvent { Fraction at; int numerator = 4; int denominator = 4; };
struct KeyEvent   { Fraction at; int fifths = 0; bool minor = false; };

// ------------------------------------------------------ structure & text --
enum class BarlineKind : std::uint8_t { Double, Final, RepeatStart, RepeatEnd, RepeatBoth };
struct BarlineMark { EventId id; Fraction at; BarlineKind kind = BarlineKind::Double; };
struct RehearsalMark { EventId id; Fraction at; };                     // lettered A, B, C … automatically
struct ClefChange { EventId id; Fraction at; std::string clef; };       // per staff: "treble", "bass", "alto" …
enum class TextKind : std::uint8_t { Expression, Technique };          // below italic ("dolce") / above ("pizz.")
struct TextItem { EventId id; Fraction at; std::string text; TextKind kind = TextKind::Expression; };

// ------------------------------------------------------------ players/DAW --
struct Instrument {
    InstrumentId id;
    PlayerId owner;
    std::string name;                // "Clarinet in B♭"
    std::string engraverName;        // LilyPond instrumentName
    int transpositionSemis = 0;      // written -> sounding (B♭ clarinet: -2)
    std::string clef = "treble";
    TrackId track;                   // the DAW lane this instrument sounds through
};

struct Player {
    PlayerId id;
    std::string name;                // "Player 1" / "Flute 1"
    std::vector<std::unique_ptr<Instrument>> instruments;   // doubling set
};

// A DAW track: a lane in the mixer and the address of a sound source.
struct Track {
    TrackId id;
    std::string name;
    NodeId instrumentNode;           // CLAP instrument plugin node that receives notes
    std::uint16_t notePort = 0;      // clap_note_port index on that plugin
    std::int16_t midiChannel = 0;    // 0..15 (CLAP channel field)
    float gainDb = 0.f;
    float pan = 0.f;
    bool mute = false, solo = false;
    NodeId outputBus;                // mixer bus node the plugin's audio feeds
};

// Audio routing graph description (persistent half). The RT half is compiled
// from this into PlaybackSnapshot::graph.
enum class NodeKind : std::uint8_t { ClapPlugin, Bus, MasterOut };

struct AudioNode {
    NodeId id;
    NodeKind kind = NodeKind::Bus;
    std::string name;
    std::string clapBundlePath;      // /usr/lib/clap/Foo.clap
    std::string clapPluginId;        // clap_plugin_descriptor.id
    std::shared_ptr<const std::vector<std::byte>> stateChunk;   // clap_plugin_state blob
    bool bypassed = false;
};

struct AudioEdge {
    NodeId from; std::uint16_t fromPort = 0;
    NodeId to;   std::uint16_t toPort = 0;
    float gain = 1.f;
};

struct AudioGraph {
    std::vector<AudioNode> nodes;
    std::vector<AudioEdge> edges;
};

// ---------------------------------------------------------------- movement --
struct InstrumentChange { EventId id; Fraction at; InstrumentId instrument; };

struct PlayerLane {
    PlayerId player;
    std::vector<InstrumentChange> changes;   // sorted; changes[0].at == 0
    std::vector<ClefChange> clefs;           // mid-movement clef changes (the initial clef is the instrument's)
    std::vector<TextItem> texts;
    std::vector<Voice> voices;               // 1..4

    // Active instrument at a musical position (binary search over changes).
    InstrumentId instrumentAt(Fraction pos) const;
    // Clef in force at `pos`: the last change at or before it, else `initial`
    // (the instrument's clef).
    const std::string& clefAt(Fraction pos, const std::string& initial) const;
};

// Diatonic step (step + 7 * octave) of a clef's middle staff line: treble B4,
// bass D3, alto C4 ... Unknown clefs count as treble.
int clefMiddleLine(const std::string& clef);

struct Movement {
    MovementId id;
    int ordinal = 0;
    std::string title;
    std::vector<TempoEvent> tempo{{Fraction{0}, 120.0, false}};
    std::vector<MeterEvent> meter{{Fraction{0}, 4, 4}};
    std::vector<KeyEvent> keys{{Fraction{0}, 0, false}};
    std::vector<BarlineMark> barlines;       // special barlines (double, final, repeats) at bar positions
    std::vector<RehearsalMark> rehearsal;
    std::vector<PlayerLane> lanes;
    std::vector<AutomationLane> automation;
    Fraction length{1};               // total duration in whole notes

    // Monotonic edit counter. Bumped by every Command touching this movement;
    // used to discard stale engraving / playback compilation results.
    std::uint64_t revision = 0;
};

// Everything about a movement that is not voice content: maps, structure
// marks and per-staff items. Edited as one undoable unit (EditMovementHeader).
struct LaneHeader { PlayerId player; std::vector<InstrumentChange> changes; std::vector<ClefChange> clefs; std::vector<TextItem> texts; };
struct MovementHeader {
    std::vector<TempoEvent> tempo;
    std::vector<MeterEvent> meter;
    std::vector<KeyEvent> keys;
    std::vector<BarlineMark> barlines;
    std::vector<RehearsalMark> rehearsal;
    Fraction length{1};
    std::vector<LaneHeader> lanes;
};
MovementHeader captureHeader(const Movement& m);
void applyHeader(Movement& m, const MovementHeader& h);

// Immutable view handed to worker threads (engraver, snapshot compiler, saver).
struct MovementSnapshot {
    std::shared_ptr<const Movement> movement;   // deep-ish copy, COW payloads shared
    std::uint64_t revision = 0;
};

// ----------------------------------------------------------------- project --
struct ProjectMeta {
    std::string title, composer;
    double sampleRate = 48000.0;
};

// Where an ElementId lives - lets point-and-click resolve an SVG id in O(1).
struct Locator {
    enum class Kind : std::uint8_t { Event, NoteHead, Voice, Movement, Player, Instrument, Track, Node, Lane, Point };
    Kind kind;
    MovementId movement;     // when applicable
    PlayerId player;
    VoiceId voice;
    TrackId track;           // DAW lane the element sounds through (resolved at index time)
};

class Project {
public:
    ProjectId id;
    ProjectMeta meta;
    std::vector<std::unique_ptr<Player>> players;
    std::vector<std::unique_ptr<Track>> tracks;
    AudioGraph graph;
    std::vector<std::unique_ptr<Movement>> movements;

    ElementId allocateId() noexcept { return ElementId{nextId_++}; }
    std::uint64_t nextId() const noexcept { return nextId_; }
    void setNextId(std::uint64_t v) noexcept { nextId_ = v; }

    // Lookups (linear scans are fine here: tens of items, UI thread only).
    Movement* movement(MovementId id);
    Player* player(PlayerId id);
    Instrument* instrument(InstrumentId id);
    Track* track(TrackId id);
    const AudioNode* node(NodeId id) const;

    // Element index for hit testing; rebuilt incrementally per movement.
    const Locator* locate(ElementId id) const;
    void reindexMovement(const Movement& m);

    MovementSnapshot snapshot(MovementId id) const;

private:
    std::uint64_t nextId_ = 1;
    std::unordered_map<ElementId, Locator> index_;
};

}  // namespace corinzia::model
