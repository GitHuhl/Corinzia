#pragma once
// PlaybackSnapshot: an immutable, fully "flattened" render plan for the audio
// thread. Built by the SnapshotCompiler on a worker thread from
// MovementSnapshots; handed to the audio thread by raw pointer through an SPSC
// queue; returned through another SPSC queue for deletion on the UI thread.
// The audio thread therefore never allocates, frees, locks, or touches the
// domain model.
//
// Everything is in absolute samples on the project timeline: movements are laid
// end to end (Section), notes are resolved to (node, port, channel, key), and
// automation is a list of sample-positioned breakpoints.

#include "corinzia/core/Types.hpp"
#include "corinzia/rt/TempoMap.hpp"

#include <cstdint>
#include <vector>

namespace corinzia::audio { class ClapPluginInstance; }

namespace corinzia::rt {

enum class NoteEdge : std::uint8_t { Off = 0, On = 1 };

struct ScheduledNote {
    SamplePos at;               // absolute sample
    std::uint32_t nodeIndex;    // index into PlaybackSnapshot::nodes
    std::int32_t noteId;        // CLAP note_id (low 31 bits of the NoteHead ElementId)
    std::int16_t port, channel, key;
    NoteEdge edge;
    float velocity;             // 0..1
    ElementId element;          // NoteHead id - lets the UI highlight sounding notes
};

enum class Curve : std::uint8_t { Step, Linear, Exponential, SCurve };

struct Breakpoint { SamplePos at; double value; Curve toNext; };

struct RtAutomationLane {
    std::uint32_t nodeIndex;
    std::uint32_t paramId;      // clap_id
    void* cookie;               // clap_param_info.cookie (fast-path for the plugin)
    bool modulation;            // false: PARAM_VALUE, true: PARAM_MOD
    std::int32_t noteId;        // -1 = global
    std::vector<Breakpoint> points;   // sorted by `at`

    double valueAt(SamplePos s) const noexcept;
};

struct RtNode {
    NodeId id;
    audio::ClapPluginInstance* plugin = nullptr;  // null for pure busses/master
    std::uint32_t latency = 0;                    // reported plugin latency
    std::uint32_t compensation = 0;               // PDC delay added after this node
    float gain = 1.f;
    bool isMaster = false;
};

struct RtEdge { std::uint32_t from, to; float gain; };

struct Section {                 // one movement on the timeline
    MovementId movement;
    SamplePos start, end;
    TempoMap tempo;              // movement-local (quarter 0 = movement start)
    double startQuarterGlobal;   // for continuous song_pos_beats across movements
};

struct PlaybackSnapshot {
    std::uint64_t generation = 0;
    double sampleRate = 48000.0;
    std::vector<Section> sections;
    std::vector<ScheduledNote> notes;            // sorted by (at, edge Off<On)
    std::vector<RtAutomationLane> automation;
    std::vector<RtNode> nodes;                   // topologically sorted
    std::vector<RtEdge> edges;
    std::uint32_t graphLatency = 0;              // max path latency (for cursor compensation)

    const Section* sectionAt(SamplePos s) const noexcept;
    // First note index with at >= s (binary search; used on seek / swap).
    std::size_t firstNoteAtOrAfter(SamplePos s) const noexcept;
};

// ---------------------------------------------------------------- inline --
inline double RtAutomationLane::valueAt(SamplePos s) const noexcept {
    if (points.empty()) return 0.0;
    if (s <= points.front().at) return points.front().value;
    if (s >= points.back().at) return points.back().value;
    std::size_t lo = 0, hi = points.size() - 1;      // invariant: points[lo].at <= s < points[hi].at
    while (hi - lo > 1) {
        const std::size_t mid = (lo + hi) / 2;
        (points[mid].at <= s ? lo : hi) = mid;
    }
    const Breakpoint& a = points[lo];
    const Breakpoint& b = points[hi];
    const double t = double(s - a.at) / double(b.at - a.at);
    switch (a.toNext) {
        case Curve::Step: return a.value;
        case Curve::Linear: return a.value + (b.value - a.value) * t;
        case Curve::Exponential: return a.value + (b.value - a.value) * t * t;
        case Curve::SCurve: return a.value + (b.value - a.value) * (t * t * (3.0 - 2.0 * t));
    }
    return a.value;
}

inline const Section* PlaybackSnapshot::sectionAt(SamplePos s) const noexcept {
    for (const auto& sec : sections)               // a handful of movements: linear is fastest
        if (s < sec.end) return &sec;
    return sections.empty() ? nullptr : &sections.back();
}

inline std::size_t PlaybackSnapshot::firstNoteAtOrAfter(SamplePos s) const noexcept {
    std::size_t lo = 0, hi = notes.size();
    while (lo < hi) {
        const std::size_t mid = (lo + hi) / 2;
        (notes[mid].at < s ? lo = mid + 1 : hi = mid);
    }
    return lo;
}

}  // namespace corinzia::rt
