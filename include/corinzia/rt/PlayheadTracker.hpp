#pragma once
// UI-side latency-compensated playhead estimation.
//
// The audio thread publishes one PlayheadState per block (Seqlock). Blocks
// arrive in bursts (e.g. 256 frames every 5.3 ms) while the display runs at
// 60-144 Hz, and what the listener *hears* lags the block being rendered by the
// device latency plus plugin delay compensation. So, per rendered frame:
//
//   heard(now) = blockStartSample
//              + (now - callbackTime) * sampleRate      // extrapolate inside/after the block
//              - outputLatencyFrames                    // DAC/driver/PipeWire quantum delay
//              - graphLatencyFrames                     // PDC: audio leaves the graph late
//
// A first-order tracking filter removes the block-quantisation jitter, but
// snaps immediately on seeks/large errors so the cursor never "slides" to a
// new position after the user clicks.

#include "corinzia/rt/RtMessages.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>

namespace corinzia::rt {

class PlayheadTracker {
public:
    // `nowNs` must come from the same clock as callbackTimeNs (CLOCK_MONOTONIC).
    double update(const PlayheadState& s, std::int64_t nowNs) noexcept {
        const double raw = rawEstimate(s, nowNs);
        if (!s.playing) {
            smoothed_ = raw;
            lastNs_ = nowNs;
            return smoothed_;
        }
        if (lastNs_ == 0 || std::abs(raw - smoothed_) > snapThresholdFrames(s)) {
            smoothed_ = raw;                                  // seek / start / big glitch
        } else {
            const double dt = double(nowNs - lastNs_) * 1e-9;
            const double predicted = smoothed_ + dt * s.sampleRate;   // free-run at nominal rate
            smoothed_ = predicted + kGain * (raw - predicted);       // correct towards measurement
        }
        lastNs_ = nowNs;
        return smoothed_;
    }

    static double rawEstimate(const PlayheadState& s, std::int64_t nowNs) noexcept {
        if (!s.playing) return double(s.blockStartSample);
        // Never extrapolate further than two blocks past the last report: if the
        // audio thread stalls (xrun), the cursor should stall too.
        const double elapsed = std::clamp(double(nowNs - s.callbackTimeNs) * 1e-9 * s.sampleRate, 0.0,
                                          2.0 * s.blockFrames);
        return double(s.blockStartSample) + elapsed - double(s.outputLatencyFrames) -
               double(s.graphLatencyFrames);
    }

private:
    static double snapThresholdFrames(const PlayheadState& s) noexcept {
        return std::max(4.0 * s.blockFrames, 0.05 * s.sampleRate);
    }
    static constexpr double kGain = 0.15;
    double smoothed_ = 0.0;
    std::int64_t lastNs_ = 0;
};

}  // namespace corinzia::rt
