#include "corinzia/model/Model.hpp"

#include <algorithm>

namespace corinzia::model {

InstrumentId PlayerLane::instrumentAt(Fraction pos) const {
    auto it = std::upper_bound(changes.begin(), changes.end(), pos,
                               [](Fraction p, const InstrumentChange& c) { return p < c.at; });
    return it == changes.begin() ? InstrumentId{} : std::prev(it)->instrument;
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

}  // namespace corinzia::model
