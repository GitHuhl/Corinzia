#include "corinzia/compile/SnapshotCompiler.hpp"
#include "corinzia/edit/NoteInput.hpp"

#include <algorithm>
#include <cmath>
#include <queue>
#include <unordered_set>

namespace corinzia::compile {

RoutingSnapshot RoutingSnapshot::capture(const model::Project& p) {
    RoutingSnapshot r;
    std::unordered_map<TrackId, const model::Track*> tracks;
    for (const auto& t : p.tracks) tracks[t->id] = t.get();
    for (const auto& pl : p.players)
        for (const auto& inst : pl->instruments)
            if (auto it = tracks.find(inst->track); it != tracks.end())
                r.instrumentRoutes[inst->id] = {it->second->instrumentNode, std::int16_t(it->second->notePort),
                                                it->second->midiChannel};
    r.nodes = p.graph.nodes;
    r.edges = p.graph.edges;
    for (const auto& t : p.tracks) {
        const float g = t->mute ? 0.f : std::pow(10.f, t->gainDb / 20.f);
        r.nodeGain[t->instrumentNode] = g;
    }
    return r;
}

double gateFactor(std::uint16_t a) noexcept {
    if (a & model::ArtStaccatissimo) return 0.25;
    if (a & model::ArtStaccato) return 0.5;
    if (a & (model::ArtBreath | model::ArtCaesura)) return 0.7;   // leave room to breathe
    if (a & model::ArtTenuto) return 1.0;
    return 0.9;
}

float velocityForDynamic(std::optional<std::uint8_t> mark) noexcept {
    static constexpr float kTable[] = {0.16f, 0.26f, 0.38f, 0.5f, 0.62f, 0.75f, 0.87f, 1.0f};  // ppp..fff
    if (!mark) return kTable[4];
    return kTable[std::min<std::uint8_t>(*mark, 7)];
}

namespace {

// Tempo and meter maps over performed time: each span starts with the tempo
// and meter in force at its written start, then carries the changes inside it.
void buildTempo(const model::Movement& m, const std::vector<std::pair<Fraction, Fraction>>& order, double sr,
                Performance& perf) {
    auto& pts = perf.tempoPoints;
    auto& meters = perf.meterPoints;
    double performed = 0.0;
    for (const auto& [a, b] : order) {
        const model::TempoEvent* t0 = &m.tempo.front();
        for (const auto& t : m.tempo) if (t.at <= a) t0 = &t;
        pts.push_back({performed, t0->quarterBpm, t0->rampToNext && t0->at == a});
        for (const auto& t : m.tempo)
            if (t.at > a && t.at < b) pts.push_back({performed + (t.at - a).toQuarters(), t.quarterBpm, t.rampToNext});
        const model::MeterEvent* m0 = &m.meter.front();
        for (const auto& me : m.meter) if (me.at <= a) m0 = &me;
        meters.push_back({performed, 0, m0->numerator, m0->denominator});
        for (const auto& me : m.meter)
            if (me.at > a && me.at < b) meters.push_back({performed + (me.at - a).toQuarters(), 0, me.numerator, me.denominator});
        performed += (b - a).toQuarters();
    }
    perf.tempo.build(pts, sr);
    perf.tempo.buildMeters(meters);
}

// Topological order of graph nodes (Kahn). Cycles are rejected by the routing
// UI; any leftovers are appended so nothing silently disappears.
std::vector<std::size_t> topoOrder(const RoutingSnapshot& r) {
    const std::size_t n = r.nodes.size();
    std::unordered_map<NodeId, std::size_t> idx;
    for (std::size_t i = 0; i < n; ++i) idx[r.nodes[i].id] = i;
    std::vector<std::vector<std::size_t>> succ(n);
    std::vector<int> indeg(n, 0);
    for (const auto& e : r.edges) {
        auto a = idx.find(e.from), b = idx.find(e.to);
        if (a == idx.end() || b == idx.end()) continue;
        succ[a->second].push_back(b->second);
        ++indeg[b->second];
    }
    std::vector<std::size_t> order;
    std::queue<std::size_t> q;
    for (std::size_t i = 0; i < n; ++i) if (indeg[i] == 0) q.push(i);
    while (!q.empty()) {
        auto v = q.front(); q.pop();
        order.push_back(v);
        for (auto s : succ[v]) if (--indeg[s] == 0) q.push(s);
    }
    for (std::size_t i = 0; i < n; ++i)
        if (std::find(order.begin(), order.end(), i) == order.end()) order.push_back(i);
    return order;
}

}  // namespace

std::vector<FigureNote> ornamentFigure(const model::Pitch& main, Fraction length, std::uint16_t a, int keyFifths) {
    auto neighbour = [&](int dir) {
        const int d = edit::diatonic(main) + dir;
        return edit::fromDiatonic(d, edit::keyAlter(keyFifths, ((d % 7) + 7) % 7));
    };
    const Fraction unit = std::min(Fraction(1, 32), length * Fraction(1, 4));
    std::vector<FigureNote> out;
    Fraction at{0};
    auto add = [&](const model::Pitch& p, Fraction len) {
        len = std::min(len, length - at);
        if (len > Fraction{0}) out.push_back({at, len, p});
        at = at + len;
    };
    if (a & model::ArtTrill) {
        const model::Pitch upper = neighbour(+1);
        // Alternate in units; an odd count keeps the main note last.
        bool onMain = true;
        while (length - at > unit) {
            add(onMain ? main : upper, unit);
            onMain = !onMain;
        }
        if (!onMain && !out.empty()) out.back().length = out.back().length + (length - at), at = length;   // end on main
        else add(main, length - at);
    } else if (a & model::ArtTurn) {
        add(neighbour(+1), unit);
        add(main, unit);
        add(neighbour(-1), unit);
        add(main, length - at);
    } else if (a & model::ArtMordent) {
        add(main, unit);
        add(neighbour(-1), unit);
        add(main, length - at);
    } else if (a & model::ArtTremolo) {
        while (length - at > unit) add(main, unit);
        add(main, length - at);
    } else {
        add(main, length);
    }
    return out;
}

// Plays the movement like a performer: an end repeat jumps back to the last
// start repeat (or the start) until the repeat's passes are done; an ending
// (volta) not marked for the current pass is skipped, together with the end
// repeat at its end. A repeat with endings takes as many passes as its
// highest ending number; without endings, two.
std::vector<std::pair<Fraction, Fraction>> playbackOrder(const model::Movement& m) {
    using K = model::BarlineKind;
    std::vector<model::BarlineMark> bars = m.barlines;
    std::sort(bars.begin(), bars.end(), [](const auto& x, const auto& y) { return x.at < y.at; });
    std::vector<model::Ending> ends = m.endings;
    std::sort(ends.begin(), ends.end(), [](const auto& x, const auto& y) { return x.from < y.from; });
    const Fraction len = m.length;

    auto barAt = [&](Fraction at) -> const model::BarlineMark* {
        for (const auto& b : bars) if (b.at == at) return &b;
        return nullptr;
    };
    auto endingAt = [&](Fraction at) -> const model::Ending* {
        for (const auto& e : ends) if (e.from == at && e.from < e.to) return &e;
        return nullptr;
    };
    // Passes of the repeat starting at `from` and ending at `end`: the chain of
    // consecutive endings beginning inside it decides.
    auto passesFor = [&](Fraction from, Fraction end) {
        int passes = 2;
        for (const auto& e : ends) {
            if (e.from < from || e.from >= end) continue;
            int top = 0;
            for (const model::Ending* c = &e; c; c = endingAt(c->to))
                for (int n = 1; n <= 8; ++n)
                    if (c->passes & (1u << (n - 1))) top = std::max(top, n);
            passes = std::max(2, top);
            break;
        }
        return passes;
    };

    std::vector<std::pair<Fraction, Fraction>> out;
    Fraction cur{0}, segStart{0}, repeatFrom{0}, resetAt{-1};
    int pass = 1;
    bool jumped = false, skipped = false;
    auto flush = [&](Fraction to) {
        if (to > segStart) {
            if (!out.empty() && out.back().second == segStart) out.back().second = to;
            else out.push_back({segStart, to});
        }
    };
    for (int guard = 0; cur <= len && guard < 100000; ++guard) {   // `len` too: a final end repeat
        const model::BarlineMark* bar = barAt(cur);
        if (cur == resetAt && !skipped) {   // the last ending of a repeat is done
            pass = 1;
            repeatFrom = cur;
            resetAt = Fraction{-1};
        }
        // End repeat (only when reached by playing up to it).
        if (bar && !jumped && !skipped && (bar->kind == K::RepeatEnd || bar->kind == K::RepeatBoth) && cur > repeatFrom) {
            if (pass < passesFor(repeatFrom, cur)) {
                flush(cur);
                ++pass;
                cur = segStart = repeatFrom;
                jumped = true;
                continue;
            }
            pass = 1;
            repeatFrom = cur;
        }
        if (bar && !jumped && (bar->kind == K::RepeatStart || bar->kind == K::RepeatBoth)) {
            repeatFrom = cur;
            pass = 1;
        }
        if (const model::Ending* e = endingAt(cur)) {
            if (!(e->passes & (1u << std::min(pass - 1, 7)))) {   // not this pass: skip it
                flush(cur);
                cur = segStart = std::min(e->to, len);
                skipped = true;
                jumped = false;
                if (!endingAt(cur)) resetAt = cur, pass = 1, repeatFrom = cur;   // skipped the last one
                continue;
            }
            const model::BarlineMark* eb = barAt(e->to);
            if (!(eb && (eb->kind == K::RepeatEnd || eb->kind == K::RepeatBoth)))
                resetAt = e->to;   // an ending that does not repeat is the last: the repeat is over after it
        }
        if (cur >= len) break;
        // Advance to the next point of interest.
        Fraction next = len;
        for (const auto& b : bars) if (b.at > cur) { next = std::min(next, b.at); break; }
        for (const auto& e : ends) {
            if (e.from > cur) next = std::min(next, e.from);
            if (e.to > cur) next = std::min(next, e.to);
        }
        if (resetAt > cur) next = std::min(next, resetAt);
        cur = next;
        jumped = skipped = false;
    }
    flush(len);
    if (out.empty()) out.push_back({Fraction{0}, len});
    return out;
}

std::vector<EventShape> expressiveShape(const model::EventList& evs) {
    std::vector<EventShape> out(evs.size());
    // Pass 1: terraced dynamics and slurs.
    std::optional<std::uint8_t> dyn;
    std::vector<int> level(evs.size());
    bool slur = false;
    for (std::size_t i = 0; i < evs.size(); ++i) {
        if (evs[i].dynamicMark) dyn = evs[i].dynamicMark;
        level[i] = dyn ? *dyn : 4;
        out[i].velocity = velocityForDynamic(dyn);
        if (evs[i].spans & model::SlurStart) slur = true;
        out[i].legato = slur && !(evs[i].spans & model::SlurEnd);
        if (evs[i].spans & model::SlurEnd) slur = false;
    }
    // Pass 2: hairpins ramp from the start note's level to the target level.
    for (std::size_t i = 0; i < evs.size(); ++i) {
        const bool cresc = evs[i].spans & model::CrescStart, dim = evs[i].spans & model::DimStart;
        if (!cresc && !dim) continue;
        std::size_t j = i + 1;
        while (j < evs.size() && !(evs[j].spans & model::HairpinEnd) && !evs[j].dynamicMark) ++j;
        if (j >= evs.size()) j = evs.size() - 1;
        int target = std::clamp(level[i] + (cresc ? 2 : -2), 0, 7);
        for (std::size_t k = j; k < evs.size(); ++k)   // the next written dynamic wins
            if (evs[k].dynamicMark) { target = *evs[k].dynamicMark; break; }
        const float v0 = velocityForDynamic(std::uint8_t(level[i])), v1 = velocityForDynamic(std::uint8_t(target));
        const double t0 = evs[i].start.toDouble(), t1 = evs[j].start.toDouble();
        for (std::size_t k = i; k <= j; ++k) {
            const double f = t1 > t0 ? (evs[k].start.toDouble() - t0) / (t1 - t0) : 1.0;
            out[k].velocity = float(v0 + (v1 - v0) * f);
        }
        // After the hairpin the reached level holds until the next dynamic.
        for (std::size_t k = j + 1; k < evs.size() && !evs[k].dynamicMark; ++k) out[k].velocity = v1;
    }
    return out;
}

Performance perform(const model::Movement& m, double sampleRate) {
    Performance perf;
    perf.order = playbackOrder(m);
    buildTempo(m, perf.order, sampleRate, perf);
    for (const auto& [a, b] : perf.order) {
        perf.spans.push_back({perf.lengthQ, a.toQuarters(), (b - a).toQuarters()});
        perf.lengthQ += (b - a).toQuarters();
    }
    auto keyAt = [&](Fraction at) {
        int fifths = 0;
        for (const auto& k : m.keys) if (k.at <= at) fifths = k.fifths;
        return fifths;
    };
    for (std::size_t si = 0; si < perf.order.size(); ++si) {
        const auto [a, b] = perf.order[si];
        const double base = perf.spans[si].performedQ;
        auto toQ = [&](Fraction f) { return base + (f - a).toQuarters(); };   // written -> performed

        for (const auto& lane : m.lanes) {
            for (const auto& voice : lane.voices) {
                const auto& evs = *voice.events;
                std::unordered_set<std::uint64_t> consumed;   // tie continuations
                const auto shape = expressiveShape(evs);
                for (std::size_t ei = 0; ei < evs.size(); ++ei) {
                    const auto& ev = evs[ei];
                    if (ev.kind == model::EventKind::Rest || ev.start < a || ev.start >= b) continue;
                    const InstrumentId inst = lane.instrumentAt(ev.start);
                    for (const auto& h : ev.heads) {
                        if (consumed.count(h.id.id.value)) continue;
                        // Follow the tie chain to find the true sounding end
                        // (cut at the end of this pass: a repeat jumps away).
                        Fraction end = ev.start + ev.duration;
                        std::uint16_t lastArt = ev.articulations;
                        bool tied = h.tieToNext;
                        for (std::size_t k = ei + 1; tied && k < evs.size() && evs[k].start < b; ++k) {
                            tied = false;
                            for (const auto& h2 : evs[k].heads)
                                if (h2.pitch.midiKey() == h.pitch.midiKey()) {
                                    consumed.insert(h2.id.id.value);
                                    end = evs[k].start + evs[k].duration;
                                    lastArt = evs[k].articulations;
                                    tied = h2.tieToNext;
                                    break;
                                }
                        }
                        end = std::min(end, b);
                        // Slurred notes are legato unless explicitly articulated short.
                        const double gate = (shape[ei].legato && !(lastArt & (model::ArtStaccato | model::ArtStaccatissimo)))
                                                ? 1.0 : gateFactor(lastArt);
                        const float vel = h.velocity ? h.velocity / 127.f
                                                     : shape[ei].velocity * ((ev.articulations & model::ArtAccent) ? 1.15f : 1.f);
                        // A rolled chord starts each head a little later, bottom up
                        // (at most half the note).
                        double roll = 0;
                        if ((ev.articulations & model::ArtArpeggio) && ev.heads.size() > 1) {
                            int rank = 0;
                            for (const auto& o : ev.heads) rank += o.pitch.midiKey() < h.pitch.midiKey();
                            roll = std::min(rank * 0.03, (perf.tempo.secondsAt(toQ(end)) - perf.tempo.secondsAt(toQ(ev.start))) / 2);
                        }
                        const auto figure = ornamentFigure(h.pitch, end - ev.start, ev.articulations, keyAt(ev.start));
                        for (std::size_t fi = 0; fi < figure.size(); ++fi) {
                            const auto& f = figure[fi];
                            PerformedNote n;
                            n.instrument = inst;
                            n.element = h.id.id;
                            n.key = std::int16_t(f.pitch.midiKey());
                            n.velocity = std::min(vel, 1.f);
                            n.onQ = toQ(ev.start + f.offset);
                            n.fullOffQ = toQ(ev.start + f.offset + f.length);
                            n.gate = fi + 1 < figure.size() ? 0.95 : gate;   // figure notes are joined
                            n.rollSec = fi == 0 ? roll : 0.0;
                            perf.notes.push_back(n);
                        }
                    }
                }
            }
        }
    }
    return perf;
}

std::unique_ptr<rt::PlaybackSnapshot> compileSnapshot(const std::vector<model::MovementSnapshot>& movements,
                                                      const RoutingSnapshot& routing, const CompileOptions& opt) {
    auto snap = std::make_unique<rt::PlaybackSnapshot>();
    snap->generation = opt.generation;
    snap->sampleRate = opt.sampleRate;

    // ---- graph -------------------------------------------------------------
    const auto order = topoOrder(routing);
    std::unordered_map<NodeId, std::uint32_t> nodeIndex;
    for (std::size_t k = 0; k < order.size(); ++k) {
        const auto& n = routing.nodes[order[k]];
        rt::RtNode rn;
        rn.id = n.id;
        rn.plugin = (n.kind == model::NodeKind::ClapPlugin && opt.plugin) ? opt.plugin(n.id) : nullptr;
        rn.latency = (rn.plugin && opt.latency) ? opt.latency(n.id) : 0;
        rn.isMaster = n.kind == model::NodeKind::MasterOut;
        if (auto g = routing.nodeGain.find(n.id); g != routing.nodeGain.end()) rn.gain = g->second;
        nodeIndex[n.id] = std::uint32_t(k);
        snap->nodes.push_back(rn);
    }
    for (const auto& e : routing.edges) {
        auto a = nodeIndex.find(e.from), b = nodeIndex.find(e.to);
        if (a != nodeIndex.end() && b != nodeIndex.end())
            snap->edges.push_back({a->second, b->second, e.gain, 0, e.from, e.to});
    }
    // Longest-latency path into each node (nodes are in topological order).
    std::vector<std::uint32_t> pathLat(snap->nodes.size(), 0);
    for (std::uint32_t i = 0; i < snap->nodes.size(); ++i) {
        std::uint32_t in = 0;
        for (const auto& e : snap->edges) if (e.to == i) in = std::max(in, pathLat[e.from]);
        pathLat[i] = in + snap->nodes[i].latency;
        if (snap->nodes[i].isMaster) snap->graphLatency = std::max(snap->graphLatency, pathLat[i]);
    }
    // PDC: every input of a summing node is delayed up to its slowest input.
    std::vector<std::uint32_t> inLat(snap->nodes.size(), 0);
    for (const auto& e : snap->edges) inLat[e.to] = std::max(inLat[e.to], pathLat[e.from]);
    for (auto& e : snap->edges) e.delay = inLat[e.to] - pathLat[e.from];

    // ---- movements ---------------------------------------------------------
    SamplePos cursor = 0;
    double globalQuarter = 0.0;
    for (const auto& ms : movements) {
        if (!ms.movement) continue;
        const model::Movement& m = *ms.movement;
        Performance perf = perform(m, opt.sampleRate);
        rt::Section sec{m.id, cursor, 0, std::move(perf.tempo), globalQuarter, std::move(perf.spans)};
        const double performed = perf.lengthQ;
        sec.end = cursor + SamplePos(std::llround(sec.tempo.samplesAt(performed)));
        auto qToSample = [&](double q) { return sec.start + SamplePos(std::llround(sec.tempo.samplesAt(q))); };

        for (const auto& n : perf.notes) {
            auto route = routing.instrumentRoutes.find(n.instrument);
            if (route == routing.instrumentRoutes.end()) continue;
            auto ni = nodeIndex.find(route->second.node);
            if (ni == nodeIndex.end()) continue;
            const SamplePos on = qToSample(n.onQ) + SamplePos(std::llround(n.rollSec * opt.sampleRate));
            const SamplePos fullOff = qToSample(n.fullOffQ);
            const SamplePos off = on + SamplePos(std::llround(double(fullOff - on) * n.gate));
            const auto noteId = std::int32_t(n.element.value & 0x7fffffff);
            const auto& rtr = route->second;
            snap->notes.push_back({on, ni->second, noteId, rtr.port, rtr.channel, n.key, rt::NoteEdge::On, n.velocity, n.element});
            snap->notes.push_back({std::max(off, on + 1), ni->second, noteId, rtr.port, rtr.channel, n.key, rt::NoteEdge::Off, 0.f,
                                   n.element});
        }

        // Automation: one RT lane per lane; every pass appends its breakpoints.
        for (const auto& lane : m.automation) {
            auto ni = nodeIndex.find(lane.target.node);
            if (ni == nodeIndex.end() || lane.points->empty()) continue;
            rt::RtAutomationLane rl{ni->second, lane.target.clapParamId, nullptr,
                                    lane.target.mode == model::AutomationMode::Modulation, lane.target.noteId, {}};
            for (std::size_t si = 0; si < perf.order.size(); ++si) {
                const auto [a, b] = perf.order[si];
                const double base = sec.spans[si].performedQ;
                for (const auto& p : *lane.points)
                    if (p.at >= a && p.at < b)
                        rl.points.push_back({qToSample(base + (p.at - a).toQuarters()), p.value,
                                             rt::Curve(static_cast<std::uint8_t>(p.shapeToNext))});
            }
            snap->automation.push_back(std::move(rl));
        }

        snap->sections.push_back(std::move(sec));
        globalQuarter += performed;
        cursor = snap->sections.back().end + SamplePos(std::llround(opt.gapBetweenMovementsSec * opt.sampleRate));
    }

    std::sort(snap->notes.begin(), snap->notes.end(), [](const rt::ScheduledNote& a, const rt::ScheduledNote& b) {
        if (a.at != b.at) return a.at < b.at;
        return a.edge < b.edge;   // Off before On at the same instant
    });
    return snap;
}

}  // namespace corinzia::compile
