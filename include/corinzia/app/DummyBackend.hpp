#pragma once
// Fallback "driver" when no audio server is reachable (headless CI, a machine
// without PipeWire): a dedicated thread that renders the engine into scratch
// buffers at the real-time rate, so transport, playhead and plugin processing
// behave exactly as with a sound card - just silently. Equivalent to
// PipeWire's Dummy-Driver.

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/audio/ClapPluginHost.hpp"

#include <atomic>
#include <chrono>
#include <ctime>
#include <thread>
#include <vector>

namespace corinzia::app {

class DummyBackend {
public:
    explicit DummyBackend(audio::AudioEngine& engine, std::uint32_t quantum = 256)
        : engine_(engine), quantum_(quantum), l_(quantum), r_(quantum) {
        thread_ = std::jthread([this](std::stop_token st) { run(st); });
    }
    ~DummyBackend() { thread_.request_stop(); }

private:
    void run(std::stop_token st) {
        audio::ThreadRole::markAudioThread();
        const auto period = std::chrono::nanoseconds(std::int64_t(1e9 * quantum_ / engine_.sampleRate()));
        auto next = std::chrono::steady_clock::now();
        while (!st.stop_requested()) {
            timespec ts{};
            clock_gettime(CLOCK_MONOTONIC, &ts);
            float* out[2] = {l_.data(), r_.data()};
            engine_.process(out, quantum_, std::int64_t(ts.tv_sec) * 1'000'000'000 + ts.tv_nsec, 0);
            next += period;
            std::this_thread::sleep_until(next);
        }
    }
    audio::AudioEngine& engine_;
    std::uint32_t quantum_;
    std::vector<float> l_, r_;
    std::jthread thread_;   // last member: joins before buffers are destroyed
};

}  // namespace corinzia::app
