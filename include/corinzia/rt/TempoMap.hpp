#pragma once
// Bidirectional musical-time <-> sample-time mapping, built off the RT thread
// (by the SnapshotCompiler) and then read-only. All queries are O(log n),
// allocation-free and therefore usable inside the audio callback and the
// UI's playhead extrapolation alike.
//
// Supports constant segments and linear tempo ramps (bpm linear in beats):
//   t(b) = 60/k * ln(bpm(b)/bpm0)              (k != 0)
//   b(t) = bpm0 * (exp(k t / 60) - 1) / k

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace corinzia::rt {

class TempoMap {
public:
    struct Segment {
        double startQuarter;   // musical position (quarter notes from movement start)
        double startSecond;    // cached absolute seconds at segment start
        double bpm0;           // tempo at segment start (quarters per minute)
        double slope;          // d(bpm)/d(quarter); 0 for constant tempo
    };
    struct Meter { double startQuarter; int bar; int num; int den; };

    // `points`: (quarter, bpm, rampToNext) sorted by quarter, first at 0.
    struct TempoPoint { double quarter; double bpm; bool rampToNext; };
    void build(const std::vector<TempoPoint>& points, double sampleRate);
    // `meters`: (quarter, num, den) sorted; first at 0. Bars are counted from 1.
    void buildMeters(const std::vector<Meter>& meters);

    double sampleRate() const noexcept { return sampleRate_; }

    double secondsAt(double quarter) const noexcept;
    double quarterAt(double seconds) const noexcept;
    double tempoAt(double quarter) const noexcept;

    double samplesAt(double quarter) const noexcept { return secondsAt(quarter) * sampleRate_; }
    double quarterAtSample(double sample) const noexcept { return quarterAt(sample / sampleRate_); }

    struct BarInfo { int bar; double barStartQuarter; int num; int den; };
    BarInfo barAt(double quarter) const noexcept;

private:
    static double segSeconds(const Segment& s, double dq) noexcept;   // seconds spent in dq quarters
    static double segQuarters(const Segment& s, double dt) noexcept;  // quarters covered in dt seconds

    std::vector<Segment> segs_;
    std::vector<Meter> meters_;
    double sampleRate_ = 48000.0;
};

// ----------------------------------------------------------------- inline --
inline double TempoMap::segSeconds(const Segment& s, double dq) noexcept {
    if (std::abs(s.slope) < 1e-12) return 60.0 * dq / s.bpm0;
    return 60.0 / s.slope * std::log((s.bpm0 + s.slope * dq) / s.bpm0);
}

inline double TempoMap::segQuarters(const Segment& s, double dt) noexcept {
    if (std::abs(s.slope) < 1e-12) return dt * s.bpm0 / 60.0;
    return s.bpm0 * (std::exp(s.slope * dt / 60.0) - 1.0) / s.slope;
}

inline void TempoMap::build(const std::vector<TempoPoint>& pts, double sampleRate) {
    sampleRate_ = sampleRate;
    segs_.clear();
    double sec = 0.0;
    for (std::size_t i = 0; i < pts.size(); ++i) {
        Segment s{pts[i].quarter, sec, pts[i].bpm, 0.0};
        if (pts[i].rampToNext && i + 1 < pts.size()) {
            const double dq = pts[i + 1].quarter - pts[i].quarter;
            if (dq > 0) s.slope = (pts[i + 1].bpm - pts[i].bpm) / dq;
        }
        if (i + 1 < pts.size()) sec += segSeconds(s, pts[i + 1].quarter - pts[i].quarter);
        segs_.push_back(s);
    }
    if (segs_.empty()) segs_.push_back({0.0, 0.0, 120.0, 0.0});
}

inline void TempoMap::buildMeters(const std::vector<Meter>& m) {
    meters_.clear();
    int bar = 1;
    for (std::size_t i = 0; i < m.size(); ++i) {
        meters_.push_back({m[i].startQuarter, bar, m[i].num, m[i].den});
        if (i + 1 < m.size()) {
            const double barLen = 4.0 * m[i].num / m[i].den;
            bar += int(std::llround((m[i + 1].startQuarter - m[i].startQuarter) / barLen));
        }
    }
    if (meters_.empty()) meters_.push_back({0.0, 1, 4, 4});
}

inline double TempoMap::secondsAt(double q) const noexcept {
    auto it = std::upper_bound(segs_.begin(), segs_.end(), q,
                               [](double v, const Segment& s) { return v < s.startQuarter; });
    const Segment& s = it == segs_.begin() ? segs_.front() : *std::prev(it);
    return s.startSecond + segSeconds(s, q - s.startQuarter);
}

inline double TempoMap::quarterAt(double t) const noexcept {
    auto it = std::upper_bound(segs_.begin(), segs_.end(), t,
                               [](double v, const Segment& s) { return v < s.startSecond; });
    const Segment& s = it == segs_.begin() ? segs_.front() : *std::prev(it);
    return s.startQuarter + segQuarters(s, t - s.startSecond);
}

inline double TempoMap::tempoAt(double q) const noexcept {
    auto it = std::upper_bound(segs_.begin(), segs_.end(), q,
                               [](double v, const Segment& s) { return v < s.startQuarter; });
    const Segment& s = it == segs_.begin() ? segs_.front() : *std::prev(it);
    return s.bpm0 + s.slope * (q - s.startQuarter);
}

inline TempoMap::BarInfo TempoMap::barAt(double q) const noexcept {
    auto it = std::upper_bound(meters_.begin(), meters_.end(), q,
                               [](double v, const Meter& m) { return v < m.startQuarter; });
    const Meter& m = it == meters_.begin() ? meters_.front() : *std::prev(it);
    const double barLen = 4.0 * m.num / m.den;
    const int n = int(std::floor((q - m.startQuarter) / barLen + 1e-9));
    return {m.bar + n, m.startQuarter + n * barLen, m.num, m.den};
}

}  // namespace corinzia::rt
