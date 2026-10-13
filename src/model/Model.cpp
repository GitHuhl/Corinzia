#include "corinzia/model/Model.hpp"

#include <algorithm>

namespace corinzia::model {

InstrumentId PlayerLane::instrumentAt(Fraction pos) const {
    auto it = std::upper_bound(changes.begin(), changes.end(), pos,
                               [](Fraction p, const InstrumentChange& c) { return p < c.at; });
    return it == changes.begin() ? InstrumentId{} : std::prev(it)->instrument;
}

const std::string& PlayerLane::clefAt(Fraction pos, const std::string& initial) const {
    const std::string* c = &initial;
    for (const auto& ch : clefs)
        if (ch.at <= pos) c = &ch.clef;
    return *c;
}

int clefMiddleLine(const std::string& clef) {
    if (clef == "bass") return 1 + 7 * 3;         // D3
    if (clef == "bass_8") return 1 + 7 * 2;       // D2
    if (clef == "alto" || clef == "percussion") return 0 + 7 * 4;   // C4
    if (clef == "tenor") return 5 + 7 * 3;        // A3
    if (clef == "treble_8") return 6 + 7 * 3;     // B3
    return 6 + 7 * 4;                             // treble: B4
}

Movement* Project::movement(MovementId id) {
    for (auto& m : movements) if (m->id == id) return m.get();
    return nullptr;
}
Player* Project::player(PlayerId id) {
    for (auto& p : players) if (p->id == id) return p.get();
    return nullptr;
}
Instrument* Project::instrument(InstrumentId id) {
    for (auto& p : players)
        for (auto& i : p->instruments) if (i->id == id) return i.get();
    return nullptr;
}
Track* Project::track(TrackId id) {
    for (auto& t : tracks) if (t->id == id) return t.get();
    return nullptr;
}
const AudioNode* Project::node(NodeId id) const {
    for (auto& n : graph.nodes) if (n.id == id) return &n;
    return nullptr;
}

const Locator* Project::locate(ElementId id) const {
    auto it = index_.find(id);
    return it == index_.end() ? nullptr : &it->second;
}

void Project::reindexMovement(const Movement& m) {
    std::erase_if(index_, [&](const auto& kv) { return kv.second.movement == m.id; });
    index_[m.id.id] = {Locator::Kind::Movement, m.id, {}, {}, {}};
    for (const auto& lane : m.lanes) {
        for (const auto& v : lane.voices) {
            index_[v.id.id] = {Locator::Kind::Voice, m.id, lane.player, v.id, {}};
            for (const auto& ev : *v.events) {
                const InstrumentId inst = lane.instrumentAt(ev.start);
                const Instrument* i = const_cast<Project*>(this)->instrument(inst);
                const TrackId track = i ? i->track : TrackId{};
                index_[ev.id.id] = {Locator::Kind::Event, m.id, lane.player, v.id, track};
                for (const auto& h : ev.heads)
                    index_[h.id.id] = {Locator::Kind::NoteHead, m.id, lane.player, v.id, track};
            }
        }
    }
    for (const auto& a : m.automation) {
        index_[a.id.id] = {Locator::Kind::Lane, m.id, {}, {}, {}};
        for (const auto& p : *a.points) index_[p.id.id] = {Locator::Kind::Point, m.id, {}, {}, {}};
    }
}

MovementSnapshot Project::snapshot(MovementId id) const {
    for (const auto& m : movements)
        if (m->id == id)   // copies headers; EventLists / point lists are shared COW
            return {std::make_shared<const Movement>(*m), m->revision};
    return {};
}

MovementHeader captureHeader(const Movement& m) {
    MovementHeader h{m.tempo, m.meter, m.keys, m.barlines, m.rehearsal, m.length, {}};
    for (const auto& l : m.lanes) h.lanes.push_back({l.player, l.changes, l.clefs, l.texts});
    return h;
}

void applyHeader(Movement& m, const MovementHeader& h) {
    m.tempo = h.tempo;
    m.meter = h.meter;
    m.keys = h.keys;
    m.barlines = h.barlines;
    m.rehearsal = h.rehearsal;
    m.length = h.length;
    for (auto& l : m.lanes)
        for (const auto& lh : h.lanes)
            if (lh.player == l.player) {
                l.changes = lh.changes;
                l.clefs = lh.clefs;
                l.texts = lh.texts;
            }
}

}  // namespace corinzia::model
