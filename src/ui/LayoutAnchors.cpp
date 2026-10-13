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
    staves_.clear();

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

    // Staff geometry: attach each engraved staff to the system it lies in.
    staves_.clear();
    for (const auto& st : index.staves()) {
        const qreal mid = st.lineYs[st.lineYs.size() / 2];
        int best = -1;
        qreal bestD = 1e18;
        for (std::size_t k = 0; k < systems_.size(); ++k) {
            if (systems_[k].page != st.page) continue;
            const qreal d = mid < systems_[k].top ? systems_[k].top - mid : mid > systems_[k].bottom ? mid - systems_[k].bottom : 0;
            if (d < bestD) { bestD = d; best = int(k); }
        }
        if (best >= 0) staves_.push_back({st.lane, best, st.page, st.lineYs, st.left, st.right, st.thickness});
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

std::vector<LayoutAnchors::Region> LayoutAnchors::rangeRects(std::size_t laneFrom, std::size_t laneTo, double fromQ,
                                                             double toQ) const {
    std::vector<Region> out;
    for (std::size_t sys = 0; sys < systems_.size(); ++sys) {
        const Point* first = nullptr;
        const Point* last = nullptr;
        const Point* after = nullptr;   // first onset at/after the range end in this system
        for (const auto& p : timeline_) {
            if (p.system != int(sys)) continue;
            if (p.quarter >= fromQ && p.quarter < toQ) {
                if (!first) first = &p;
                last = &p;
            } else if (p.quarter >= toQ && !after) {
                after = &p;
            }
        }
        if (!first) continue;
        const System& s = systems_[sys];
        double top = 1e18, bottom = -1e18;
        for (std::size_t lane = laneFrom; lane <= laneTo; ++lane) {
            double sum = 0, n = 0;
            for (const auto& h : heads_)
                if (h.lane == lane && h.system == int(sys)) { sum += h.y; ++n; }
            if (!n) continue;
            top = std::min(top, sum / n - 3.0);
            bottom = std::max(bottom, sum / n + 3.0);
        }
        if (top > bottom) { top = s.top; bottom = s.bottom; }
        const double left = first->x - 1.2;
        const double right = after ? after->x - 1.2 : std::max(last->x + 2.0, s.right + 2.0);
        out.push_back({s.page, QRectF(QPointF(left, top), QPointF(right, bottom))});
    }
    return out;
}

const LayoutAnchors::StaffGeo* LayoutAnchors::staffOf(std::size_t lane, int system) const {
    for (const auto& st : staves_)
        if (st.lane == lane && st.system == system) return &st;
    return nullptr;
}

namespace {
// Compact signature of a lane's music in [from, to): did the edit change it?
std::string laneSignature(const model::PlayerLane& lane, Fraction from, Fraction to) {
    std::string sig;
    for (const auto& v : lane.voices)
        for (const auto& e : *v.events) {
            if (e.start < from || !(e.start < to)) continue;
            sig += std::to_string(v.index) + ':' + std::to_string(e.start.num) + '/' + std::to_string(e.start.den) + ':' +
                   std::to_string(e.duration.num) + '/' + std::to_string(e.duration.den) + ':' + std::to_string(e.articulations) +
                   ':' + std::to_string(e.spans) + ':' + std::to_string(e.dynamicMark.value_or(255));
            for (const auto& h : e.heads) sig += ',' + std::to_string(h.pitch.midiKey()) + (h.tieToNext ? "~" : "");
            sig += ';';
        }
    return sig;
}
}  // namespace

LayoutAnchors::ProvisionalScene LayoutAnchors::provisionalScene(const SvgHitIndex& index, const model::Movement& engraved,
                                                                const model::Movement& edited, Fraction from, Fraction to) const {
    ProvisionalScene sc;
    const double q0 = from.toQuarters(), q1 = to.toQuarters();
    constexpr qreal kStem = 3.5, kHalfHead = 0.6, kStemW = 0.13, kLedgerHalf = 0.95, kLedgerT = 0.16;

    for (std::size_t li = 0; li < edited.lanes.size(); ++li) {
        const bool changed = li >= engraved.lanes.size() ||
                             laneSignature(engraved.lanes[li], from, to) != laneSignature(edited.lanes[li], from, to);
        if (!changed) continue;
        sc.editedLanes.push_back(li);

        // ---- masks: the old engraved notes of this lane in the range --------
        struct Box { int page; int system; QRectF r; };
        std::vector<Box> boxes;
        if (li < engraved.lanes.size())
            for (const auto& v : engraved.lanes[li].voices)
                for (const auto& e : *v.events) {
                    if (e.start < from || !(e.start < to)) continue;
                    QRectF r;
                    int page = -1;
                    std::vector<ElementId> parts{e.id.id};
                    for (const auto& h : e.heads) parts.push_back(h.id.id);
                    for (ElementId id : parts)
                        if (const auto* en = index.find(id)) { r |= en->bounds; page = en->page; }
                    if (page < 0) continue;
                    const Point* p = atOrAfter(e.start.toQuarters());
                    const int system = p ? p->system : 0;
                    const StaffGeo* st = staffOf(li, system);
                    // Stems: note below the middle line -> stem up, else down (old engraving).
                    if (!e.heads.empty() && e.duration < Fraction{1} && st) {
                        const qreal mid = st->ys[st->ys.size() / 2];
                        if (r.center().y() >= mid) r.setTop(std::min(r.top(), r.center().y() - kStem - 0.3));
                        else r.setBottom(std::max(r.bottom(), r.center().y() + kStem + 0.3));
                        r.setRight(r.right() + (e.duration < Fraction(1, 4) ? 1.1 : 0.2));   // flags
                    }
                    // A tie from the old note runs to the next onset: cover its arc too.
                    if (std::any_of(e.heads.begin(), e.heads.end(), [](const model::NoteHead& h) { return h.tieToNext; })) {
                        const double nq = (e.start + e.duration).toQuarters();
                        const Point* np = atOrAfter(nq);
                        const qreal tieEnd = (np && np->system == system) ? np->x - 0.7 : r.right() + 2.0;
                        r.setRight(std::max(r.right(), tieEnd));
                        r.setTop(r.top() - 1.0);
                        r.setBottom(r.bottom() + 1.2);
                    }
                    // ±0.5 horizontally also covers the old note's ledger lines.
                    boxes.push_back({page, system, r.adjusted(-0.5, -0.25, 0.5, 0.25)});
                }
        // Merge boxes per system into one mask (+ staff-line redraw) per system.
        std::vector<Box> merged;
        for (const auto& b : boxes) {
            auto it = std::find_if(merged.begin(), merged.end(), [&](const Box& m) { return m.system == b.system && m.page == b.page; });
            if (it == merged.end()) merged.push_back(b);
            else it->r |= b.r;
        }
        for (auto m : merged) {
            // Never reach into a neighbouring staff: clamp to the halfway lines
            // between this staff and the staves above/below in the system.
            if (const StaffGeo* own = staffOf(li, m.system)) {
                for (const auto& other : staves_) {
                    if (other.system != m.system || other.lane == li) continue;
                    if (other.ys.back() < own->ys.front())
                        m.r.setTop(std::max(m.r.top(), (other.ys.back() + own->ys.front()) / 2));
                    else if (other.ys.front() > own->ys.back())
                        m.r.setBottom(std::min(m.r.bottom(), (own->ys.back() + other.ys.front()) / 2));
                }
            }
            sc.masks.push_back({m.page, m.r});
            if (const StaffGeo* st = staffOf(li, m.system))
                for (qreal y : st->ys) {
                    const qreal l = std::max(m.r.left(), st->left), r = std::min(m.r.right(), st->right);
                    if (r > l && y >= m.r.top() && y <= m.r.bottom())
                        sc.staffLines.push_back({m.page, QRectF(l, y - st->thickness / 2, r - l, st->thickness)});
                }
        }

        // ---- provisional notes with stems and ledger lines --------------------
        const Point* a = before(q0);
        const Point* b = atOrAfter(q1);
        const Point* ref = a ? a : b;
        if (!ref) continue;
        for (const auto& v : edited.lanes[li].voices)
            for (const auto& ev : *v.events) {
                if (ev.start < from || !(ev.start < to)) continue;
                const double q = ev.start.toQuarters();
                const Point* next = atOrAfter(q);
                const Point* here = (next && next->quarter == q) ? next : (before(q) ? before(q) : next);
                const int system = here ? here->system : ref->system;
                const int page = here ? here->page : ref->page;
                const qreal x = (next && next->quarter == q) ? next->x : xAt(q, before(q), next);
                const StaffGeo* st = staffOf(li, system);
                double c0, c1;
                if (!fitLane(li, system, c0, c1)) {   // no engraved heads to fit: use the staff, treble assumption
                    c1 = kStepHeight;
                    c0 = (st ? st->ys[st->ys.size() / 2] : systems_[std::size_t(system)].top + 4.0) - c1 * 34;
                }
                if (ev.heads.empty()) {
                    sc.glyphs.push_back({ev.id.id, page, {x, st ? st->ys[st->ys.size() / 2] : c0 + c1 * 34}, true, false});
                    continue;
                }
                qreal yMin = 1e9, yMax = -1e9;
                const bool hollow = ev.duration >= Fraction(1, 2);
                for (const auto& h : ev.heads) {
                    const qreal y = c0 + c1 * (h.pitch.step + 7 * h.pitch.octave);
                    yMin = std::min(yMin, y);
                    yMax = std::max(yMax, y);
                    sc.glyphs.push_back({h.id.id, page, {x, y}, false, hollow});
                    if (!st) continue;
                    // Ledger lines: every staff space beyond the staff, out to the note.
                    const qreal top = st->ys.front(), bottom = st->ys.back();
                    for (qreal ly = top - 1.0; ly >= y - 0.3; ly -= 1.0)
                        sc.ledgers.push_back({page, QRectF(x - kLedgerHalf, ly - kLedgerT / 2, 2 * kLedgerHalf, kLedgerT)});
                    for (qreal ly = bottom + 1.0; ly <= y + 0.3; ly += 1.0)
                        sc.ledgers.push_back({page, QRectF(x - kLedgerHalf, ly - kLedgerT / 2, 2 * kLedgerHalf, kLedgerT)});
                }
                if (ev.duration >= Fraction{1}) continue;   // whole notes: no stem
                const qreal mid = st ? st->ys[st->ys.size() / 2] : c0 + c1 * 34;
                // Standard rule: the head furthest from the middle line decides;
                // below the middle -> stem up (y grows downwards). On the line: down.
                const bool up = (yMax - mid) > (mid - yMin) + 1e-6;
                if (up) {   // stem on the right, from the lowest head up past the highest (reaching the middle line)
                    const qreal tip = std::min(yMin - kStem, mid);
                    sc.stems.push_back({page, QRectF(x + kHalfHead - kStemW, tip, kStemW, yMax - tip)});
                } else {
                    const qreal tip = std::max(yMax + kStem, mid);
                    sc.stems.push_back({page, QRectF(x - kHalfHead, yMin, kStemW, tip - yMin)});
                }
            }
    }
    return sc;
}

}  // namespace corinzia::ui
