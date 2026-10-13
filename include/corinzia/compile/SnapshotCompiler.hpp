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
//   * topologically sort the audio graph and compute the latency of the
//     longest path (used for cursor compensation; per-node PDC delay lines are
//     the next step, see docs/ARCHITECTURE.md §3.6).

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

// Exposed for unit tests / the engraver's MIDI-preview path.
double gateFactor(std::uint16_t articulations) noexcept;
float velocityForDynamic(std::optional<std::uint8_t> mark) noexcept;

}  // namespace corinzia::compile
