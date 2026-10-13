#pragma once
// The realtime render core. Backend-agnostic: the PipeWire/JACK/ALSA backend
// owns the RT thread and calls process() from its callback.
//
// RT-safety contract for process():
//   * no allocation / free, no locks, no syscalls except clock_gettime,
//   * reads the domain model NEVER - only the current PlaybackSnapshot,
//   * communicates exclusively via SpscQueue (commands in, telemetry and
//     garbage out) and a Seqlock<PlayheadState>.

#include "corinzia/audio/ClapEventList.hpp"
#include "corinzia/rt/PlaybackSnapshot.hpp"
#include "corinzia/rt/RtMessages.hpp"
#include "corinzia/rt/Seqlock.hpp"
#include "corinzia/rt/SpscQueue.hpp"

#include <array>
#include <cstdint>
#include <memory>
#include <vector>

namespace corinzia::audio {

class AudioEngine {
public:
    static constexpr std::uint32_t kMaxNodes = 256;
    static constexpr std::uint32_t kMaxLanes = 1024;
    static constexpr std::uint32_t kMaxActiveNotes = 2048;
    static constexpr std::uint32_t kChannels = 2;
    static constexpr std::uint32_t kAutomationStep = 32;   // control-rate resolution, samples
    static constexpr std::uint32_t kMaxEdges = 1024;
    static constexpr std::uint32_t kMaxDelayLines = 64;     // edges that need PDC at the same time
    static constexpr std::uint32_t kDelayLineSize = 16384;  // power of two; max PDC = size-1 samples

    using CommandQueue = rt::SpscQueue<rt::UiToAudio, 1024>;
    using TelemetryQueue = rt::SpscQueue<rt::AudioToUi, 4096>;

    AudioEngine();
    ~AudioEngine();

    // Main thread, before the backend starts calling process().
    void prepare(double sampleRate, std::uint32_t maxFrames);

    // ---- UI thread API (producer of commands / consumer of telemetry) -------
    bool post(const rt::UiToAudio& m) noexcept { return commands_.tryPush(m); }
    CommandQueue& commands() noexcept { return commands_; }
    TelemetryQueue& telemetry() noexcept { return telemetry_; }
    rt::PlayheadState playhead() const noexcept { return playhead_.load(); }
    // Deletes retired snapshots; call from the UI thread (e.g. every frame).
    void collectGarbage(void (*onOther)(const rt::AudioToUi&, void*) = nullptr, void* ctx = nullptr);

    // ---- audio thread ----------------------------------------------------------
    // `out`: kChannels non-interleaved buffers of `frames` floats.
    void process(float* const* out, std::uint32_t frames, std::int64_t callbackTimeNs,
                 std::uint32_t outputLatencyFrames) noexcept;

    // For tests/diagnostics: the event list delivered to a node in the last block.
    const ClapInputEventList<>& lastEventsFor(std::uint32_t nodeIndex) const { return nodes_[nodeIndex]->in; }

private:
    struct NodeState {
        ClapInputEventList<> in;
        ClapOutputEventList<> out;
        std::vector<float> inBuf, outBuf;        // kChannels * maxFrames
        std::array<float*, kChannels> inPtr{}, outPtr{};
        bool bypassed = false;
    };
    // PDC delay line, owned by an edge identity (fromId, toId) so its contents
    // survive snapshot swaps; preallocated, assigned on the RT thread.
    struct DelayLine {
        NodeId from, to;
        std::uint32_t write = 0;
        std::vector<float> buf;                  // kChannels * kDelayLineSize
        bool used = false;
    };
    struct ActiveNote { std::uint32_t node; NodeId nodeId; std::int32_t noteId; std::int16_t port, channel, key; };

    void handleCommands() noexcept;
    void swapSnapshot(rt::PlaybackSnapshot* next) noexcept;
    void releaseAllNotes(std::uint32_t frameOffset) noexcept;
    void scheduleRange(SamplePos from, SamplePos to, std::uint32_t frameOffset) noexcept;
    void scheduleAutomation(SamplePos from, SamplePos to, std::uint32_t frameOffset, bool force) noexcept;
    void fillTransport(SamplePos pos) noexcept;
    void renderGraph(float* const* out, std::uint32_t frames, SamplePos blockStart) noexcept;
    void forwardOutputEvents(std::uint32_t node, SamplePos blockStart) noexcept;
    void assignDelayLines() noexcept;
    void addActive(const rt::ScheduledNote& n) noexcept;
    void removeActive(std::int32_t noteId, std::uint32_t node) noexcept;

    CommandQueue commands_;
    TelemetryQueue telemetry_;
    rt::Seqlock<rt::PlayheadState> playhead_;

    rt::PlaybackSnapshot* snap_ = nullptr;      // audio-thread owned
    std::vector<std::unique_ptr<NodeState>> nodes_;
    std::vector<double> lastLaneValue_;
    std::vector<DelayLine> delayLines_;
    std::array<std::int16_t, kMaxEdges> edgeLine_{};   // edge index -> delay line, -1 = none
    std::array<ActiveNote, kMaxActiveNotes> active_{};
    std::uint32_t activeCount_ = 0;

    double sampleRate_ = 48000.0;
    std::uint32_t maxFrames_ = 0;
    SamplePos pos_ = 0;
    std::int64_t steadyTime_ = 0;               // clap_process.steady_time
    SamplePos loopStart_ = 0, loopEnd_ = 0;
    std::size_t noteCursor_ = 0;
    bool playing_ = false;
    bool automationDirty_ = true;               // force-send all lane values (after seek/swap/start)
    std::uint64_t dropped_ = 0;
    clap_event_transport_t transport_{};
};

}  // namespace corinzia::audio
