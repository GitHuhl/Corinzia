#include "corinzia/compile/SnapshotCompiler.hpp"

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
    if (a & model::ArtStaccato) return 0.5;
    if (a & model::ArtTenuto) return 1.0;
    return 0.9;
}

float velocityForDynamic(std::optional<std::uint8_t> mark) noexcept {
    static constexpr float kTable[] = {0.16f, 0.26f, 0.38f, 0.5f, 0.62f, 0.75f, 0.87f, 1.0f};  // ppp..fff
    if (!mark) return kTable[4];
    return kTable[std::min<std::uint8_t>(*mark, 7)];
}

namespace {

rt::TempoMap buildTempo(const model::Movement& m, double sr) {
    rt::TempoMap tm;
    std::vector<rt::TempoMap::TempoPoint> pts;
    for (const auto& t : m.tempo) pts.push_back({t.at.toQuarters(), t.quarterBpm, t.rampToNext});
    tm.build(pts, sr);
    std::vector<rt::TempoMap::Meter> meters;
    for (const auto& me : m.meter) meters.push_back({me.at.toQuarters(), 0, me.numerator, me.denominator});
    tm.buildMeters(meters);
    return tm;
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
        if (a != nodeIndex.end() && b != nodeIndex.end()) snap->edges.push_back({a->second, b->second, e.gain});
    }
    // Longest-latency path into each node (nodes are in topological order).
    std::vector<std::uint32_t> pathLat(snap->nodes.size(), 0);
    for (std::uint32_t i = 0; i < snap->nodes.size(); ++i) {
        std::uint32_t in = 0;
        for (const auto& e : snap->edges) if (e.to == i) in = std::max(in, pathLat[e.from]);
        pathLat[i] = in + snap->nodes[i].latency;
        if (snap->nodes[i].isMaster) snap->graphLatency = std::max(snap->graphLatency, pathLat[i]);
    }
    for (std::uint32_t i = 0; i < snap->nodes.size(); ++i)
        snap->nodes[i].compensation = snap->graphLatency - std::min(snap->graphLatency, pathLat[i]);

    // ---- movements ---------------------------------------------------------
    SamplePos cursor = 0;
    double globalQuarter = 0.0;
    for (const auto& ms : movements) {
        if (!ms.movement) continue;
        const model::Movement& m = *ms.movement;
        rt::Section sec{m.id, cursor, 0, buildTempo(m, opt.sampleRate), globalQuarter};
        sec.end = cursor + SamplePos(std::llround(sec.tempo.samplesAt(m.length.toQuarters())));
        auto toSample = [&](Fraction f) { return sec.start + SamplePos(std::llround(sec.tempo.samplesAt(f.toQuarters()))); };

        for (const auto& lane : m.lanes) {
            for (const auto& voice : lane.voices) {
                const auto& evs = *voice.events;
                std::unordered_set<std::uint64_t> consumed;   // tie continuations
                std::optional<std::uint8_t> dyn;
                for (std::size_t ei = 0; ei < evs.size(); ++ei) {
                    const auto& ev = evs[ei];
                    if (ev.dynamicMark) dyn = ev.dynamicMark;
                    if (ev.kind == model::EventKind::Rest) continue;
                    auto route = routing.instrumentRoutes.find(lane.instrumentAt(ev.start));
                    if (route == routing.instrumentRoutes.end()) continue;
                    auto ni = nodeIndex.find(route->second.node);
                    if (ni == nodeIndex.end()) continue;

                    for (const auto& h : ev.heads) {
                        if (consumed.count(h.id.id.value)) continue;
                        // Follow the tie chain to find the true sounding end.
                        Fraction end = ev.start + ev.duration;
                        std::uint16_t lastArt = ev.articulations;
                        bool tied = h.tieToNext;
                        for (std::size_t k = ei + 1; tied && k < evs.size(); ++k) {
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
                        const SamplePos on = toSample(ev.start);
                        const SamplePos fullOff = toSample(end);
                        const SamplePos off = on + SamplePos(std::llround(double(fullOff - on) * gateFactor(lastArt)));
                        const float vel = h.velocity ? h.velocity / 127.f
                                                     : velocityForDynamic(dyn) *
                                                           ((ev.articulations & model::ArtAccent) ? 1.15f : 1.f);
                        const auto noteId = std::int32_t(h.id.id.value & 0x7fffffff);
                        const auto key = std::int16_t(h.pitch.midiKey());
                        const auto& rtr = route->second;
                        snap->notes.push_back({on, ni->second, noteId, rtr.port, rtr.channel, key, rt::NoteEdge::On,
                                               std::min(vel, 1.f), h.id.id});
                        snap->notes.push_back({std::max(off, on + 1), ni->second, noteId, rtr.port, rtr.channel, key,
                                               rt::NoteEdge::Off, 0.f, h.id.id});
                    }
                }
            }
        }

        for (const auto& lane : m.automation) {
            auto ni = nodeIndex.find(lane.target.node);
            if (ni == nodeIndex.end() || lane.points->empty()) continue;
            rt::RtAutomationLane rl{ni->second, lane.target.clapParamId, nullptr,
                                    lane.target.mode == model::AutomationMode::Modulation, lane.target.noteId, {}};
            for (const auto& p : *lane.points)
                rl.points.push_back({toSample(p.at), p.value, rt::Curve(static_cast<std::uint8_t>(p.shapeToNext))});
            snap->automation.push_back(std::move(rl));
        }

        snap->sections.push_back(std::move(sec));
        globalQuarter += m.length.toQuarters();
        cursor = snap->sections.back().end + SamplePos(std::llround(opt.gapBetweenMovementsSec * opt.sampleRate));
    }

    std::sort(snap->notes.begin(), snap->notes.end(), [](const rt::ScheduledNote& a, const rt::ScheduledNote& b) {
        if (a.at != b.at) return a.at < b.at;
        return a.edge < b.edge;   // Off before On at the same instant
    });
    return snap;
}

}  // namespace corinzia::compile
