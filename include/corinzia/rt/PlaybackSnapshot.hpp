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

#include <algorithm>
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
    float gain = 1.f;
    bool isMaster = false;
};

// `delay` is plugin delay compensation: it aligns this edge's signal with the
// slowest input of the same summing node. (fromId, toId) identify the edge
// across snapshots so its delay line keeps its contents through edits.
struct RtEdge {
    std::uint32_t from, to;
    float gain;
    std::uint32_t delay = 0;
    NodeId fromId, toId;
};

struct Section {                 // one movement on the timeline
    MovementId movement;
    SamplePos start, end;
    TempoMap tempo;              // movement-local, in *performed* quarters (repeats unrolled)
    double startQuarterGlobal;   // for continuous song_pos_beats across movements
    // Playback order: written spans laid end to end in performed time. One
    // identity span when the movement has no repeats.
    struct Span { double performedQ, writtenQ, lengthQ; };
    std::vector<Span> spans;

    // Performed -> written quarter (the playhead in the score).
    double writtenQuarter(double performedQ) const noexcept {
        for (auto it = spans.rbegin(); it != spans.rend(); ++it)
            if (performedQ >= it->performedQ) return it->writtenQ + std::min(performedQ - it->performedQ, it->lengthQ);
        return performedQ;
    }
    // Written -> performed quarter, first pass through that point (seek).
    double performedQuarter(double writtenQ) const noexcept {
        for (const auto& sp : spans)
            if (writtenQ >= sp.writtenQ && writtenQ < sp.writtenQ + sp.lengthQ) return sp.performedQ + (writtenQ - sp.writtenQ);
        return spans.empty() ? writtenQ : spans.back().performedQ + spans.back().lengthQ;
    }
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
