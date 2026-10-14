#pragma once
// Model -> PlaybackSnapshot compiler. Runs on a worker thread (never the RT
// thread, never blocking the UI). Input is fully immutable: MovementSnapshots
// plus a RoutingSnapshot captured on the UI thread in O(#tracks).
//
// Responsibilities:
//   * lay movements end to end on one sample timeline (Sections),
//   * resolve each note to its *sounding* DAW track at that instant
//     (PlayerLane::instrumentAt -> Instrument::track -> node/port/channel),
//     which is what makes mid-movement instrument changes "just work",
//   * merge ties, apply articulation gate times and dynamics -> velocity,
//   * convert automation points to sample-positioned breakpoints,
//   * topologically sort the audio graph, compute the latency of the longest
//     path (cursor compensation) and a per-edge PDC delay that aligns every
//     input of a summing node with its slowest input.

#include "corinzia/model/Model.hpp"
#include "corinzia/rt/PlaybackSnapshot.hpp"

#include <functional>
#include <memory>
#include <unordered_map>
#include <vector>

namespace corinzia::compile {

struct TrackRoute { NodeId node; std::int16_t port = 0; std::int16_t channel = 0; };

struct RoutingSnapshot {
    std::unordered_map<InstrumentId, TrackRoute> instrumentRoutes;   // via Instrument::track
    std::vector<model::AudioNode> nodes;
    std::vector<model::AudioEdge> edges;
    std::unordered_map<NodeId, float> nodeGain;                      // linear, from mixer state

    static RoutingSnapshot capture(const model::Project& p);         // UI thread
};

struct CompileOptions {
    double sampleRate = 48000.0;
    double gapBetweenMovementsSec = 2.0;
    std::uint64_t generation = 0;
    // Resolves live plugin instances (owned by the PluginManager on the UI thread).
    std::function<audio::ClapPluginInstance*(NodeId)> plugin;
    std::function<std::uint32_t(NodeId)> latency;                    // optional
};

std::unique_ptr<rt::PlaybackSnapshot> compileSnapshot(const std::vector<model::MovementSnapshot>& movements,
                                                      const RoutingSnapshot& routing, const CompileOptions& opt);

// Per-event playback shaping of one voice: velocity from dynamics with
// hairpins interpolated over time (towards the next dynamic, or ±2 steps if
// there is none), and legato inside slurs (all but the slur's last note).
struct EventShape { float velocity; bool legato; };
std::vector<EventShape> expressiveShape(const model::EventList& events);

// Playback order of a movement's written music: repeat barlines unrolled
// (each repeated passage is played twice; an end repeat with no start repeat
// goes back to the beginning or to the previous end repeat). Returns written
// [from, to) spans in performed order.
std::vector<std::pair<Fraction, Fraction>> playbackOrder(const model::Movement& m);

// Written-out playback of an ornament on one head sounding for `length`
// (whole notes, tie chain included): trill = main/upper alternation ending on
// the main note, mordent = main-lower-main, turn = upper-main-lower-main,
// tremolo = repeated notes. Neighbours follow the key (`keyFifths`). The
// figure unit is a 32nd, or a quarter of a shorter note. Returns contiguous
// segments; without an ornament one segment covering the note.
struct FigureNote { Fraction offset, length; model::Pitch pitch; };
std::vector<FigureNote> ornamentFigure(const model::Pitch& main, Fraction length, std::uint16_t articulations, int keyFifths);

// Exposed for unit tests / the engraver's MIDI-preview path.
double gateFactor(std::uint16_t articulations) noexcept;
float velocityForDynamic(std::optional<std::uint8_t> mark) noexcept;

}  // namespace corinzia::compile
