#pragma once
// PipeWire audio backend built on pw_filter: a JACK-style DSP node with two
// declared float32 output ports (output_FL / output_FR). Unlike pw_stream, the
// ports exist without format negotiation, so patchbays (qpwgraph, Helvum) or
// the session manager can link them immediately. The process callback runs on
// PipeWire's realtime data thread (PW_FILTER_FLAG_RT_PROCESS; RT priority
// comes from PipeWire's module-rt / RTKit, nothing to do here).
//
// Per callback (n = position->clock.duration, the graph quantum):
//   * render n frames through AudioEngine::process (chunked to maxFrames),
//   * pass the cycle start time (clock.nsec, CLOCK_MONOTONIC) and the output
//     latency so the UI playhead is latency compensated. Output latency is
//     max(|clock.delay| (driver <-> hardware), the SPA_PARAM_Latency reported
//     on our ports by the graph downstream); max() avoids double counting a
//     sink that includes its own device latency in the port latency.
//   * xruns are counted from SPA_IO_CLOCK_FLAG_XRUN_RECOVER.
//
// JACK applications are served by PipeWire's JACK implementation (pw-jack),
// so a separate JACK backend is only needed on systems without PipeWire.

#include <atomic>
#include <cstdint>
#include <string>

struct pw_thread_loop;
struct pw_filter;
struct spa_io_position;
struct spa_pod;

namespace corinzia::audio {

class AudioEngine;

class PipeWireBackend {
public:
    struct Config {
        std::string name = "Corinzia";
        std::uint32_t sampleRate = 48000;
        std::uint32_t quantum = 256;        // requested node.latency = quantum/rate
        bool autoconnect = true;            // node.autoconnect hint for the session manager
    };

    PipeWireBackend();
    ~PipeWireBackend();
    PipeWireBackend(const PipeWireBackend&) = delete;
    PipeWireBackend& operator=(const PipeWireBackend&) = delete;

    // Main thread. The engine must already be prepare()d with
    // maxFrames >= the largest quantum you expect (chunks otherwise).
    bool start(AudioEngine& engine, const Config& cfg, std::string* error = nullptr);
    void stop();
    bool running() const noexcept { return filter_ != nullptr; }

    // Statistics (any thread).
    std::uint64_t callbacks() const noexcept { return callbacks_.load(std::memory_order_relaxed); }
    std::uint64_t framesRendered() const noexcept { return frames_.load(std::memory_order_relaxed); }
    std::uint32_t lastDelayFrames() const noexcept { return lastDelay_.load(std::memory_order_relaxed); }
    std::uint64_t xruns() const noexcept { return xruns_.load(std::memory_order_relaxed); }
    const std::string& nodeName() const noexcept { return cfg_.name; }

private:
    static void onProcess(void* data, spa_io_position* position);
    static void onParamChanged(void* data, void* portData, std::uint32_t id, const spa_pod* param);
    void process(spa_io_position* position) noexcept;

    AudioEngine* engine_ = nullptr;
    pw_thread_loop* loop_ = nullptr;
    pw_filter* filter_ = nullptr;
    void* ports_[2] = {nullptr, nullptr};
    Config cfg_;
    std::atomic<std::uint64_t> callbacks_{0}, frames_{0};
    std::atomic<std::uint32_t> lastDelay_{0};
    std::atomic<std::uint64_t> xruns_{0};
    std::atomic<double> portLatencyQuanta_{0}, portLatencyFrames_{0}, portLatencyNs_{0};
};

}  // namespace corinzia::audio
