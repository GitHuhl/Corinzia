#include "corinzia/audio/PipeWireBackend.hpp"

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/audio/ClapPluginHost.hpp"

#include <pipewire/filter.h>
#include <pipewire/pipewire.h>
#include <spa/node/io.h>
#include <spa/param/latency-utils.h>

#include <algorithm>
#include <cmath>

namespace corinzia::audio {

PipeWireBackend::PipeWireBackend() {
    pw_init(nullptr, nullptr);   // refcounted inside libpipewire
}

PipeWireBackend::~PipeWireBackend() {
    stop();
    pw_deinit();
}

bool PipeWireBackend::start(AudioEngine& engine, const Config& cfg, std::string* error) {
    if (filter_) return true;
    engine_ = &engine;
    cfg_ = cfg;

    loop_ = pw_thread_loop_new("corinzia-pw", nullptr);
    if (!loop_) {
        if (error) *error = "pw_thread_loop_new failed";
        return false;
    }

    const std::string latency = std::to_string(cfg.quantum) + "/" + std::to_string(cfg.sampleRate);
    const std::string rate = "1/" + std::to_string(cfg.sampleRate);
    pw_properties* props = pw_properties_new(
        PW_KEY_MEDIA_TYPE, "Audio", PW_KEY_MEDIA_CATEGORY, "Playback", PW_KEY_MEDIA_ROLE, "Production",
        PW_KEY_MEDIA_CLASS, "Stream/Output/Audio", PW_KEY_NODE_NAME, cfg.name.c_str(), PW_KEY_APP_NAME,
        cfg.name.c_str(), PW_KEY_NODE_LATENCY, latency.c_str(), PW_KEY_NODE_RATE, rate.c_str(),
        PW_KEY_NODE_AUTOCONNECT, cfg.autoconnect ? "true" : "false", nullptr);

    // The events table must outlive the filter; one static table per process.
    static const pw_filter_events events = [] {
        pw_filter_events ev{};
        ev.version = PW_VERSION_FILTER_EVENTS;
        ev.process = &PipeWireBackend::onProcess;
        ev.param_changed = &PipeWireBackend::onParamChanged;
        return ev;
    }();

    pw_thread_loop_lock(loop_);
    filter_ = pw_filter_new_simple(pw_thread_loop_get_loop(loop_), cfg.name.c_str(), props, &events, this);
    if (!filter_) {
        pw_thread_loop_unlock(loop_);
        if (error) *error = "pw_filter_new_simple failed (is PipeWire running?)";
        pw_thread_loop_destroy(loop_);
        loop_ = nullptr;
        return false;
    }
    static const char* kPortNames[2] = {"output_FL", "output_FR"};
    static const char* kChannels[2] = {"FL", "FR"};
    for (int c = 0; c < 2; ++c)
        ports_[c] = pw_filter_add_port(filter_, PW_DIRECTION_OUTPUT, PW_FILTER_PORT_FLAG_MAP_BUFFERS, sizeof(int),
                                       pw_properties_new(PW_KEY_FORMAT_DSP, "32 bit float mono audio", PW_KEY_PORT_NAME,
                                                         kPortNames[c], PW_KEY_AUDIO_CHANNEL, kChannels[c], nullptr),
                                       nullptr, 0);
    const int rc = pw_filter_connect(filter_, PW_FILTER_FLAG_RT_PROCESS, nullptr, 0);
    pw_thread_loop_unlock(loop_);
    if (rc < 0 || !ports_[0] || !ports_[1]) {
        if (error) *error = "pw_filter_connect failed (is PipeWire running?)";
        stop();
        return false;
    }
    if (pw_thread_loop_start(loop_) < 0) {
        if (error) *error = "pw_thread_loop_start failed";
        stop();
        return false;
    }
    return true;
}

void PipeWireBackend::stop() {
    if (loop_) pw_thread_loop_stop(loop_);   // joins the loop: no further callbacks after this
    if (filter_) {
        pw_filter_destroy(filter_);
        filter_ = nullptr;
        ports_[0] = ports_[1] = nullptr;
    }
    if (loop_) {
        pw_thread_loop_destroy(loop_);
        loop_ = nullptr;
    }
}

void PipeWireBackend::onProcess(void* data, spa_io_position* position) {
    static_cast<PipeWireBackend*>(data)->process(position);
}

void PipeWireBackend::onParamChanged(void* data, void* portData, std::uint32_t id, const spa_pod* param) {
    // Main-loop thread. Latency is per port; both of ours see the same graph.
    if (id != SPA_PARAM_Latency || !portData || !param) return;
    spa_latency_info info{};
    if (spa_latency_parse(param, &info) < 0) return;
    // Our node only has outputs, so upstream (capture) latency is zero and the
    // max over both directions is the playback latency.
    auto* self = static_cast<PipeWireBackend*>(data);
    auto raise = [](std::atomic<double>& a, double v) { if (v > a.load()) a.store(v); };
    raise(self->portLatencyQuanta_, double(info.max_quantum));
    raise(self->portLatencyFrames_, double(info.max_rate));
    raise(self->portLatencyNs_, double(info.max_ns));
}

void PipeWireBackend::process(spa_io_position* position) noexcept {
    thread_local bool marked = false;
    if (!marked) {
        ThreadRole::markAudioThread();
        marked = true;
    }
    if (!position) return;
    const auto frames = std::uint32_t(position->clock.duration);
    float* out[AudioEngine::kChannels];
    for (std::uint32_t c = 0; c < AudioEngine::kChannels; ++c) {
        out[c] = static_cast<float*>(pw_filter_get_dsp_buffer(ports_[c], frames));
        if (!out[c]) return;   // port not linked / no buffer this cycle
    }
    if (position->clock.flags & SPA_IO_CLOCK_FLAG_XRUN_RECOVER) xruns_.fetch_add(1, std::memory_order_relaxed);

    const spa_io_clock& clk = position->clock;
    const double sr = cfg_.sampleRate;
    const double toFrames = clk.rate.denom ? sr * clk.rate.num / clk.rate.denom : 1.0;
    const double deviceDelay = std::abs(double(clk.delay)) * toFrames;
    const double portLatency = portLatencyQuanta_.load(std::memory_order_relaxed) * double(frames) +
                               portLatencyFrames_.load(std::memory_order_relaxed) +
                               portLatencyNs_.load(std::memory_order_relaxed) * sr * 1e-9;
    const auto delay = std::uint32_t(std::max(deviceDelay, portLatency));
    lastDelay_.store(delay, std::memory_order_relaxed);

    // clock.nsec: CLOCK_MONOTONIC start of this cycle, comparable with the UI's clock.
    const std::int64_t now = std::int64_t(clk.nsec);
    const std::uint32_t chunk = std::max<std::uint32_t>(1, engine_->maxFrames());
    for (std::uint32_t done = 0; done < frames;) {
        const std::uint32_t n = std::min(chunk, frames - done);
        float* part[AudioEngine::kChannels];
        for (std::uint32_t c = 0; c < AudioEngine::kChannels; ++c) part[c] = out[c] + done;
        engine_->process(part, n, now + std::int64_t(double(done) * 1e9 / cfg_.sampleRate), delay);
        done += n;
    }
    callbacks_.fetch_add(1, std::memory_order_relaxed);
    frames_.fetch_add(frames, std::memory_order_relaxed);
}

}  // namespace corinzia::audio
