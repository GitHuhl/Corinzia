#include "corinzia/ui/LayoutAnchors.hpp"

#include <algorithm>
#include <cmath>

namespace corinzia::ui {

namespace {
constexpr qreal kFallbackQuarterWidth = 4.0;   // staff spaces per crotchet when extrapolating
constexpr qreal kStepHeight = -0.5;            // staff spaces per diatonic step (up = negative y)
int diatonic(const model::Pitch& p) { return p.step + 7 * p.octave; }
}  // namespace

void LayoutAnchors::build(const SvgHitIndex& index, const model::Movement& m) {
    timeline_.clear();
    heads_.clear();
    systems_.clear();

    struct Raw { double q; int page; QRectF r; std::size_t lane; int diatonic; bool head; };
    std::vector<Raw> raw;
    for (std::size_t li = 0; li < m.lanes.size(); ++li)
        for (const auto& v : m.lanes[li].voices)
            for (const auto& ev : *v.events) {
                const double q = ev.start.toQuarters();
                if (ev.heads.empty()) {
                    if (auto* e = index.find(ev.id.id)) raw.push_back({q, e->page, e->bounds, li, 0, false});
                    continue;
                }
                for (const auto& h : ev.heads)
                    if (auto* e = index.find(h.id.id)) raw.push_back({q, e->page, e->bounds, li, diatonic(h.pitch), true});
            }
    std::sort(raw.begin(), raw.end(), [](const Raw& a, const Raw& b) { return a.q < b.q; });

    // Walk onsets in time; a backwards jump in x (or a new page) starts a new system.
    int system = -1;
    for (std::size_t i = 0; i < raw.size();) {
        std::size_t j = i;
        qreal x = 0;
        while (j < raw.size() && raw[j].q == raw[i].q) x = std::max(x, raw[j].r.center().x()), ++j;
        const int page = raw[i].page;
        if (timeline_.empty() || page != timeline_.back().page || x < timeline_.back().x - 1e-6) {
            ++system;
            systems_.push_back({page, 1e9, -1e9, x, x});
        }
        timeline_.push_back({raw[i].q, page, system, x});
        auto& s = systems_.back();
        for (std::size_t k = i; k < j; ++k) {
            s.top = std::min(s.top, raw[k].r.top() - 2.0);
            s.bottom = std::max(s.bottom, raw[k].r.bottom() + 2.0);
            if (raw[k].head) heads_.push_back({raw[k].lane, system, raw[k].diatonic, raw[k].r.center().y()});
        }
        s.right = std::max(s.right, x);
        i = j;
    }
}

const LayoutAnchors::Point* LayoutAnchors::before(double q) const {
    auto it = std::lower_bound(timeline_.begin(), timeline_.end(), q, [](const Point& p, double v) { return p.quarter < v; });
    return it == timeline_.begin() ? nullptr : &*std::prev(it);
}

const LayoutAnchors::Point* LayoutAnchors::atOrAfter(double q) const {
    auto it = std::lower_bound(timeline_.begin(), timeline_.end(), q, [](const Point& p, double v) { return p.quarter < v; });
    return it == timeline_.end() ? nullptr : &*it;
}

qreal LayoutAnchors::xAt(double q, const Point* a, const Point* b) const {
    if (a && b && a->system == b->system && b->quarter > a->quarter)
        return a->x + (b->x - a->x) * (q - a->quarter) / (b->quarter - a->quarter);
    if (a) return std::min(a->x + (q - a->quarter) * kFallbackQuarterWidth, systems_[std::size_t(a->system)].right + 4.0);
    if (b) return b->x - (b->quarter - q) * kFallbackQuarterWidth;
    return 0;
}

LayoutAnchors::CursorPos LayoutAnchors::cursorAt(double q) const {
    if (timeline_.empty()) return {};
    const Point* b = atOrAfter(q);
    const Point* a = (b && b->quarter == q) ? b : before(q);
    const Point* sysPt = a ? a : b;
    const System& s = systems_[std::size_t(sysPt->system)];
    return {true, sysPt->page, xAt(q, a, b == a ? nullptr : b), s.top, s.bottom};
}

std::vector<LayoutAnchors::Glyph> LayoutAnchors::provisional(const model::Movement& m, Fraction from, Fraction to) const {
    std::vector<Glyph> out;
    const double q0 = from.toQuarters(), q1 = to.toQuarters();
    const Point* a = before(q0);
    const Point* b = atOrAfter(q1);
    const Point* ref = a ? a : b;
    if (!ref) return out;
    const int system = ref->system;

    for (std::size_t li = 0; li < m.lanes.size(); ++li) {
        double c0, c1;
        if (!fitLane(li, system, c0, c1)) {
            c1 = kStepHeight;
            c0 = systems_[std::size_t(system)].top + 4.0 - c1 * 34;   // centre B4 in the strip
        }

        for (const auto& v : m.lanes[li].voices)
            for (const auto& ev : *v.events) {
                if (ev.start < from || !(ev.start < to)) continue;
                // Onsets that still exist keep their engraved x (pitch edits,
                // articulation changes); new onsets interpolate between neighbours.
                const double q = ev.start.toQuarters();
                const Point* next = atOrAfter(q);
                const qreal x = (next && next->quarter == q) ? next->x : xAt(q, before(q), next);
                if (ev.heads.empty()) {
                    out.push_back({ev.id.id, ref->page, {x, c0 + c1 * 34}, true});
                    continue;
                }
                for (const auto& h : ev.heads) out.push_back({h.id.id, ref->page, {x, c0 + c1 * diatonic(h.pitch)}, false});
            }
    }
    return out;
}

std::vector<LayoutAnchors::Region> LayoutAnchors::staleRegions(Fraction from, Fraction to) const {
    std::vector<Region> out;
    const double q0 = from.toQuarters(), q1 = to.toQuarters();
    // One rectangle per system the range touches, spanning the engraved onsets in range.
    for (std::size_t i = 0; i < timeline_.size(); ++i) {
        const Point& p = timeline_[i];
        if (p.quarter < q0 || p.quarter >= q1) continue;
        const System& s = systems_[std::size_t(p.system)];
        if (!out.empty() && out.back().page == p.page && std::abs(out.back().rect.top() - s.top) < 1e-9) {
            out.back().rect.setRight(p.x + 1.5);
            continue;
        }
        out.push_back({p.page, QRectF(QPointF(p.x - 1.5, s.top), QPointF(p.x + 1.5, s.bottom))});
    }
    return out;
}

bool LayoutAnchors::fitLane(std::size_t lane, int system, double& c0, double& c1) const {
    double n = 0, sx = 0, sy = 0, sxx = 0, sxy = 0;
    for (const auto& h : heads_)
        if (h.lane == lane && h.system == system) {
            n += 1; sx += h.diatonic; sy += h.y; sxx += double(h.diatonic) * h.diatonic; sxy += h.diatonic * h.y;
        }
    if (n < 1) return false;
    c1 = (n >= 2 && (n * sxx - sx * sx) > 1e-9) ? (n * sxy - sx * sy) / (n * sxx - sx * sx) : kStepHeight;
    // A badly conditioned fit (e.g. all heads on one pitch) falls back to the known slope.
    if (c1 > -0.3 || c1 < -0.7) c1 = kStepHeight;
    c0 = (sy - c1 * sx) / n;
    return true;
}

int LayoutAnchors::systemAt(int page, qreal y) const {
    int best = -1;
    qreal bestDist = 1e18;
    for (std::size_t s = 0; s < systems_.size(); ++s) {
        if (systems_[s].page != page) continue;
        const qreal d = y < systems_[s].top ? systems_[s].top - y : y > systems_[s].bottom ? y - systems_[s].bottom : 0;
        if (d < bestDist) { bestDist = d; best = int(s); }
    }
    return bestDist < 6.0 ? best : -1;   // within 6 staff spaces of a system
}

LayoutAnchors::CursorPos LayoutAnchors::caretAt(std::size_t lane, double quarter) const {
    CursorPos c = cursorAt(quarter);
    if (!c.valid) return c;
    const Point* p = atOrAfter(quarter);
    if (!p || p->quarter != quarter) p = before(quarter);
    if (!p) p = atOrAfter(quarter);
    double c0, c1;
    if (p && fitLane(lane, p->system, c0, c1)) {
        // The staff's middle line is the median of the lane's engraved heads'
        // fitted positions; a staff is 4 spaces tall: draw a 6-space caret.
        double sum = 0, n = 0;
        for (const auto& h : heads_)
            if (h.lane == lane && h.system == p->system) { sum += h.y; ++n; }
        const double mid = n ? sum / n : c0 + c1 * 34;
        c.top = mid - 3.0;
        c.bottom = mid + 3.0;
    }
    return c;
}

LayoutAnchors::Hit LayoutAnchors::locate(int page, QPointF pt) const {
    Hit hit;
    const int system = systemAt(page, pt.y());
    if (system < 0) return hit;

    // x -> quarter: invert the piecewise-linear onset map inside this system.
    const Point* first = nullptr;
    const Point* last = nullptr;
    const Point* left = nullptr;
    const Point* right = nullptr;
    for (const auto& p : timeline_) {
        if (p.system != system) continue;
        if (!first) first = &p;
        last = &p;
        if (p.x <= pt.x()) left = &p;
        if (p.x > pt.x() && !right) right = &p;
    }
    if (!first) return hit;
    if (left && right && right->x > left->x)
        hit.quarter = left->quarter + (pt.x() - left->x) / (right->x - left->x) * (right->quarter - left->quarter);
    else if (!left)
        hit.quarter = std::max(0.0, first->quarter - (first->x - pt.x()) / kFallbackQuarterWidth);
    else
        hit.quarter = last->quarter + (pt.x() - last->x) / kFallbackQuarterWidth;

    // y -> lane: the lane whose engraved heads sit closest; then the pitch.
    double bestDist = 1e18;
    std::size_t lanes = 0;
    for (const auto& h : heads_) lanes = std::max(lanes, h.lane + 1);
    for (std::size_t li = 0; li < lanes; ++li) {
        double sum = 0, n = 0;
        for (const auto& h : heads_)
            if (h.lane == li && h.system == system) { sum += h.y; ++n; }
        if (!n) continue;
        const double d = std::abs(pt.y() - sum / n);
        double c0, c1;
        if (d < bestDist && fitLane(li, system, c0, c1)) {
            bestDist = d;
            hit.lane = li;
            hit.diatonic = int(std::lround((pt.y() - c0) / c1));
            hit.valid = true;
        }
    }
    return hit;
}

}  // namespace corinzia::ui
