// End-to-end: AudioEngine + recorder plugin driven by a real PipeWire graph.
// Run through tests/run_with_pipewire.sh, which starts a private headless
// daemon (Dummy-Driver, no session manager). Skips if no daemon is reachable.
#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/audio/PipeWireBackend.hpp"
#include "corinzia/compile/SnapshotCompiler.hpp"
#include "corinzia/rt/PlayheadTracker.hpp"

#include <pipewire/filter.h>
#include <pipewire/pipewire.h>
#include <spa/node/io.h>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <thread>

using namespace corinzia;
using namespace std::chrono_literals;

namespace {

// Minimal JACK-style sink: two DSP input ports, counts frames, keeps the peak.
class TestSink {
public:
    explicit TestSink(const char* name) {
        loop_ = pw_thread_loop_new("test-sink", nullptr);
        static const pw_filter_events events = [] {
            pw_filter_events ev{};
            ev.version = PW_VERSION_FILTER_EVENTS;
            ev.process = [](void* data, spa_io_position* pos) {
                auto* self = static_cast<TestSink*>(data);
                const auto n = std::uint32_t(pos->clock.duration);
                for (void* port : self->ports_)
                    if (auto* in = static_cast<const float*>(pw_filter_get_dsp_buffer(port, n)))
                        for (std::uint32_t f = 0; f < n; ++f) self->peak_ = std::max(self->peak_.load(), std::abs(in[f]));
                self->frames_ += n;
            };
            return ev;
        }();
        pw_thread_loop_lock(loop_);
        filter_ = pw_filter_new_simple(pw_thread_loop_get_loop(loop_), name,
                                       pw_properties_new(PW_KEY_MEDIA_TYPE, "Audio", PW_KEY_MEDIA_CATEGORY, "Capture",
                                                         PW_KEY_NODE_NAME, name, nullptr),
                                       &events, this);
        const char* portNames[2] = {"playback_FL", "playback_FR"};
        for (int c = 0; c < 2; ++c)
            ports_[c] = pw_filter_add_port(filter_, PW_DIRECTION_INPUT, PW_FILTER_PORT_FLAG_MAP_BUFFERS, sizeof(int),
                                           pw_properties_new(PW_KEY_FORMAT_DSP, "32 bit float mono audio",
                                                             PW_KEY_PORT_NAME, portNames[c], nullptr),
                                           nullptr, 0);
        pw_filter_connect(filter_, PW_FILTER_FLAG_RT_PROCESS, nullptr, 0);
        pw_thread_loop_unlock(loop_);
        pw_thread_loop_start(loop_);
    }
    ~TestSink() {
        pw_thread_loop_stop(loop_);
        pw_filter_destroy(filter_);
        pw_thread_loop_destroy(loop_);
    }
    std::uint64_t frames() const { return frames_; }
    float peak() const { return peak_; }

private:
    pw_thread_loop* loop_ = nullptr;
    pw_filter* filter_ = nullptr;
    void* ports_[2] = {};
    std::atomic<std::uint64_t> frames_{0};
    std::atomic<float> peak_{0.f};
};

}  // namespace

TEST(pipewire_drives_engine_in_realtime) {
    model::Project project;
    auto ids = fixture::makeProject(project);
    audio::ThreadRole::markMainThread();
    auto lib = audio::ClapLibrary::fromEntry(&fixture::kEntry, "mem://recorder");
    auto a = audio::ClapPluginInstance::create(lib, "test.recorder", ids.synthA);
    auto b = audio::ClapPluginInstance::create(lib, "test.recorder", ids.synthB);
    a->activate(48000, 1, 4096);
    b->activate(48000, 1, 4096);

    audio::AudioEngine engine;
    engine.prepare(48000, 4096);
    compile::CompileOptions opt;
    opt.generation = 1;
    opt.plugin = [&](NodeId n) { return n == ids.synthA ? a.get() : n == ids.synthB ? b.get() : nullptr; };
    auto snap = compile::compileSnapshot({project.snapshot(ids.mov)}, compile::RoutingSnapshot::capture(project), opt);
    rt::UiToAudio swap{rt::UiToAudio::Type::SwapSnapshot};
    swap.snapshot = snap.release();
    engine.post(swap);
    engine.post({rt::UiToAudio::Type::Play});

    audio::PipeWireBackend pw;
    audio::PipeWireBackend::Config cfg;
    cfg.autoconnect = false;      // no session manager in the test daemon: we link below
    TestSink sink("corinzia-test-sink");   // stands in for the sound card
    std::string err;
    const bool started = pw.start(engine, cfg, &err);
    CHECK(started);
    if (!started) { std::printf("  %s\n", err.c_str()); return; }
    std::this_thread::sleep_for(300ms);       // let both nodes and their ports appear
    for (const char* ch : {"FL", "FR"}) {
        const std::string cmd = std::string("pw-link Corinzia:output_") + ch + " corinzia-test-sink:playback_" + ch;
        if (std::system(cmd.c_str()) != 0) std::printf("  pw-link failed: %s\n", cmd.c_str());
    }

    // Let ~1.2 s of audio play in real time; collect garbage like the UI would.
    const auto t0 = std::chrono::steady_clock::now();
    while (std::chrono::steady_clock::now() - t0 < 1200ms) {
        engine.collectGarbage();
        std::this_thread::sleep_for(10ms);
    }
    const auto ph = engine.playhead();
    pw.stop();
    a->stopProcessing();
    b->stopProcessing();

    std::printf("  %llu callbacks, %llu frames, quantum %u, delay %u, xruns %llu\n", (unsigned long long)pw.callbacks(),
                (unsigned long long)pw.framesRendered(), ph.blockFrames, pw.lastDelayFrames(),
                (unsigned long long)pw.xruns());
    std::printf("  sink received %llu frames, peak %.3f\n", (unsigned long long)sink.frames(), sink.peak());
    CHECK(pw.callbacks() > 50);
    CHECK(sink.frames() > 0 && std::abs(sink.peak() - 0.5f) < 1e-6);   // chord = 2 notes x 0.25 reached the "card"
    // Real time: ~1.2 s at 48 kHz (generous bounds for a loaded CI box).
    CHECK(pw.framesRendered() > 48000 * 0.6 && pw.framesRendered() < 48000 * 2.0);
    // The C4 at sample 0 and the chord at 48000 reached the plugin on time.
    const auto& ev = fixture::recorder(*a)->events;
    std::size_t ons = 0;
    for (const auto& e : ev) if (e.type == CLAP_EVENT_NOTE_ON) ++ons;
    CHECK(ons >= 1);
    CHECK(!ev.empty() && ev.front().sample == 0);
    if (pw.framesRendered() > 50000) {
        CHECK(ons == 3);
        for (const auto& e : ev)
            if (e.type == CLAP_EVENT_NOTE_ON && e.key != 60) CHECK(e.sample == 48000);
    }
    // Playhead was published from the RT thread with a real monotonic timestamp.
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    const std::int64_t nowNs = std::int64_t(ts.tv_sec) * 1'000'000'000 + ts.tv_nsec;
    CHECK(ph.playing && ph.callbackTimeNs > 0 && ph.callbackTimeNs <= nowNs);
    CHECK(ph.blockStartSample > 0);
}

int main(int argc, char** argv) {
    int run = 0;
    for (auto& c : t::registry()) {
        if (argc > 1 && !std::strstr(c.name, argv[1])) continue;
        const int before = t::failures();
        c.fn();
        std::printf("%s %s\n", t::failures() == before ? "[ OK ]" : "[FAIL]", c.name);
        ++run;
    }
    std::printf("%d tests, %d failed checks\n", run, t::failures());
    return t::failures() == 0 ? 0 : 1;
}
