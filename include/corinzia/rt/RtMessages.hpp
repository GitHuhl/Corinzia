#pragma once
// Message vocabulary across the realtime boundary. All messages are trivially
// copyable PODs so SpscQueue can move them without hidden allocations.

#include "corinzia/core/Types.hpp"

#include <cstdint>

namespace corinzia::audio { class ClapPluginInstance; }

namespace corinzia::rt {

struct PlaybackSnapshot;

// ------------------------------------------------------------ UI -> Audio --
struct UiToAudio {
    enum class Type : std::uint8_t {
        SwapSnapshot,   // ptr: new PlaybackSnapshot (ownership transfers to audio thread)
        Play, Stop,
        Seek,           // a: sample position
        SetLoop,        // a: start, b: end (b <= a disables)
        SetNodeGain,    // node: index, value: linear gain
        SetParam,       // node, param, value: live knob tweak (PARAM_VALUE, not recorded here)
        BypassNode,     // nodeId + plugin: stop_processing + ack (main thread may then deactivate)
        UnbypassNode,   // nodeId
        PreviewNote,    // node/port/channel/key/value(velocity); value<0 => note-off
    } type;
    std::uint32_t node = 0;
    std::uint32_t param = 0;
    std::int16_t port = 0, channel = 0, key = 0;
    double value = 0.0;
    SamplePos a = 0, b = 0;
    PlaybackSnapshot* snapshot = nullptr;
    NodeId nodeId{};
    audio::ClapPluginInstance* plugin = nullptr;
};

// ------------------------------------------------------------ Audio -> UI --
struct AudioToUi {
    enum class Type : std::uint8_t {
        RetireSnapshot,      // snapshot: delete on UI thread
        NodeBypassed,        // nodeId: safe to deactivate/restart/destroy now
        ParamChanged,        // plugin GUI moved a param: node, param, value, a = sample (for recording)
        GestureBegin, GestureEnd,
        EventsDropped,       // a: count (event list overflow - surfaced in UI, never blocks)
        Xrun,
    } type;
    std::uint32_t node = 0;
    std::uint32_t param = 0;
    double value = 0.0;
    SamplePos a = 0;
    PlaybackSnapshot* snapshot = nullptr;
    NodeId nodeId{};
};

// ------------------------------------------- Audio -> UI (seqlock, per block) --
// Everything the UI needs to place the cursor with sub-block accuracy.
struct PlayheadState {
    SamplePos blockStartSample = 0;     // timeline position of the first frame of the block
    std::int64_t callbackTimeNs = 0;    // CLOCK_MONOTONIC when the block started processing
    std::uint32_t blockFrames = 0;
    std::uint32_t outputLatencyFrames = 0;  // device/driver latency (PipeWire delay, JACK range)
    std::uint32_t graphLatencyFrames = 0;   // plugin delay compensation of the longest path
    double sampleRate = 48000.0;
    std::uint64_t snapshotGeneration = 0;
    bool playing = false;
};

}  // namespace corinzia::rt
