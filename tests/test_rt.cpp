#include "Test.hpp"

#include "corinzia/rt/PlayheadTracker.hpp"
#include "corinzia/rt/Seqlock.hpp"
#include "corinzia/rt/SpscQueue.hpp"
#include "corinzia/rt/TempoMap.hpp"

#include <atomic>
#include <thread>

using namespace corinzia;

TEST(spsc_fifo_and_capacity) {
    rt::SpscQueue<int, 4> q;
    for (int i = 0; i < 4; ++i) CHECK(q.tryPush(i));
    CHECK(!q.tryPush(99));                       // full
    for (int i = 0; i < 4; ++i) CHECK(q.tryPop().value_or(-1) == i);
    CHECK(!q.tryPop().has_value());
}

TEST(spsc_threaded_stress) {
    rt::SpscQueue<std::uint64_t, 256> q;
    constexpr std::uint64_t N = 1'000'000;
    std::thread producer([&] {
        for (std::uint64_t i = 0; i < N;) if (q.tryPush(i)) ++i;
    });
    std::uint64_t expected = 0;
    bool ordered = true;
    while (expected < N)
        if (auto v = q.tryPop()) { ordered &= (*v == expected); ++expected; }
    producer.join();
    CHECK(ordered);
}

TEST(seqlock_never_tears) {
    struct P { std::uint64_t a, b, c; };
    rt::Seqlock<P> s;
    std::atomic<bool> stop{false};
    std::thread writer([&] {
        for (std::uint64_t i = 0; !stop.load(); ++i) s.store({i, i * 2, i * 3});
    });
    bool consistent = true;
    for (int i = 0; i < 200000; ++i) {
        P p = s.load();
        consistent &= (p.b == p.a * 2 && p.c == p.a * 3);
    }
    stop = true;
    writer.join();
    CHECK(consistent);
}

TEST(tempo_map_constant_and_ramp) {
    rt::TempoMap tm;
    // 120 bpm for 8 quarters, ramp 120->60 over 4 quarters, then 60.
    tm.build({{0, 120, false}, {8, 120, true}, {12, 60, false}}, 48000);
    CHECK_NEAR(tm.secondsAt(8), 4.0, 1e-9);
    // Ramp duration: 60/k * ln(b1/b0), k = -15 bpm/quarter
    const double ramp = 60.0 / -15.0 * std::log(60.0 / 120.0);
    CHECK_NEAR(tm.secondsAt(12), 4.0 + ramp, 1e-9);
    CHECK_NEAR(tm.secondsAt(13), 4.0 + ramp + 1.0, 1e-9);
    for (double q : {0.0, 3.3, 8.0, 9.7, 12.0, 20.5}) CHECK_NEAR(tm.quarterAt(tm.secondsAt(q)), q, 1e-9);
    CHECK_NEAR(tm.tempoAt(10), 90.0, 1e-9);
    CHECK_NEAR(tm.samplesAt(1), 24000.0, 1e-6);
}

TEST(tempo_map_bars) {
    rt::TempoMap tm;
    tm.buildMeters({{0, 0, 4, 4}, {8, 0, 3, 4}});     // two bars of 4/4 then 3/4
    auto b = tm.barAt(9.5);
    CHECK(b.bar == 3);
    CHECK_NEAR(b.barStartQuarter, 8.0, 1e-12);
    CHECK(b.num == 3);
    CHECK(tm.barAt(11.0).bar == 4);
}

TEST(playhead_compensates_latency_and_extrapolates) {
    rt::PlayheadState s;
    s.blockStartSample = 48000;
    s.callbackTimeNs = 1'000'000'000;
    s.blockFrames = 256;
    s.outputLatencyFrames = 512;
    s.graphLatencyFrames = 64;
    s.sampleRate = 48000;
    s.playing = true;
    // 2 ms after the callback: +96 frames, minus 576 latency.
    CHECK_NEAR(rt::PlayheadTracker::rawEstimate(s, 1'002'000'000), 48000 + 96 - 576, 1e-6);
    // Stalled audio thread: extrapolation is capped at 2 blocks.
    CHECK_NEAR(rt::PlayheadTracker::rawEstimate(s, 2'000'000'000), 48000 + 512 - 576, 1e-6);

    rt::PlayheadTracker tr;
    const double first = tr.update(s, 1'000'000'000);
    CHECK_NEAR(first, 48000 - 576, 1e-6);           // snaps on first update
    s.blockStartSample += 256;
    s.callbackTimeNs += 5'333'333;
    const double next = tr.update(s, s.callbackTimeNs);
    CHECK(next > first && std::abs(next - (first + 256)) < 2.0);
}
