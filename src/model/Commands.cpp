#include "corinzia/model/Commands.hpp"

#include <algorithm>

namespace corinzia::model {

namespace {
template <class T>
void appendUnique(std::vector<T>& dst, const std::vector<T>& src) {
    for (const auto& v : src)
        if (std::find(dst.begin(), dst.end(), v) == dst.end()) dst.push_back(v);
}
}  // namespace

void ChangeSet::merge(const ChangeSet& o) {
    appendUnique(movements, o.movements);
    appendUnique(voices, o.voices);
    appendUnique(lanes, o.lanes);
    ranges.insert(ranges.end(), o.ranges.begin(), o.ranges.end());
    routingChanged |= o.routingChanged;
    metaChanged |= o.metaChanged;
}

bool ChangeSet::empty() const noexcept {
    return movements.empty() && voices.empty() && lanes.empty() && !routingChanged && !metaChanged;
}

// ------------------------------------------------------- ReplaceVoiceEvents --
ReplaceVoiceEvents::ReplaceVoiceEvents(MovementId m, VoiceId v, std::shared_ptr<const EventList> after,
                                       DirtyRange range, std::string label, bool mergeable)
    : movement_(m), voice_(v), after_(std::move(after)), range_(range), label_(std::move(label)), mergeable_(mergeable) {}

ChangeSet ReplaceVoiceEvents::swapIn(Project& p, std::shared_ptr<const EventList> list) {
    ChangeSet cs;
    Movement* m = p.movement(movement_);
    if (!m) return cs;
    for (auto& lane : m->lanes)
        for (auto& v : lane.voices)
            if (v.id == voice_) {
                if (!before_) before_ = v.events;   // captured on first apply
                v.events = std::move(list);
                ++m->revision;
                p.reindexMovement(*m);
                cs.movements = {movement_};
                cs.voices = {voice_};
                cs.ranges = {range_};
                return cs;
            }
    return cs;
}

ChangeSet ReplaceVoiceEvents::apply(Project& p) { return swapIn(p, after_); }
ChangeSet ReplaceVoiceEvents::revert(Project& p) { return swapIn(p, before_); }

bool ReplaceVoiceEvents::mergeWith(const Command& next) {
    auto* n = dynamic_cast<const ReplaceVoiceEvents*>(&next);
    if (!n || !mergeable_ || !n->mergeable_ || n->voice_ != voice_ || n->label_ != label_) return false;
    after_ = n->after_;                      // keep our original before_
    range_.from = std::min(range_.from, n->range_.from);
    range_.to = std::max(range_.to, n->range_.to);
    return true;
}

// ----------------------------------------------------------------- AddVoice --
ChangeSet AddVoice::apply(Project& p) {
    ChangeSet cs;
    if (Movement* m = p.movement(movement_))
        for (auto& lane : m->lanes)
            if (lane.player == player_) {
                Voice v;
                v.id = voice_;
                v.index = index_;
                lane.voices.push_back(std::move(v));
                ++m->revision;
                p.reindexMovement(*m);
                cs.movements = {movement_};
                cs.voices = {voice_};
            }
    return cs;
}

ChangeSet AddVoice::revert(Project& p) {
    ChangeSet cs;
    if (Movement* m = p.movement(movement_))
        for (auto& lane : m->lanes)
            if (std::erase_if(lane.voices, [&](const Voice& v) { return v.id == voice_; })) {
                ++m->revision;
                p.reindexMovement(*m);
                cs.movements = {movement_};
            }
    return cs;
}

// -------------------------------------------------------- EditMovementHeader --
ChangeSet EditMovementHeader::set(Project& p, const MovementHeader& h) {
    ChangeSet cs;
    if (Movement* m = p.movement(movement_)) {
        if (!captured_) { before_ = captureHeader(*m); captured_ = true; }
        applyHeader(*m, h);
        ++m->revision;
        p.reindexMovement(*m);
        cs.movements = {movement_};
        cs.ranges = {{movement_, Fraction{0}, m->length}};
        cs.metaChanged = true;
    }
    return cs;
}
ChangeSet EditMovementHeader::apply(Project& p) { return set(p, after_); }
ChangeSet EditMovementHeader::revert(Project& p) { return set(p, before_); }

// --------------------------------------------------------- SetMovementLength --
ChangeSet SetMovementLength::set(Project& p, Fraction len) {
    ChangeSet cs;
    if (Movement* m = p.movement(movement_)) {
        if (!captured_) { before_ = m->length; captured_ = true; }
        m->length = len;
        ++m->revision;
        cs.movements = {movement_};
    }
    return cs;
}
ChangeSet SetMovementLength::apply(Project& p) { return set(p, after_); }
ChangeSet SetMovementLength::revert(Project& p) { return set(p, before_); }

// ------------------------------------------------------------- SetTrackGain --
ChangeSet SetTrackGain::set(Project& p, float db) {
    ChangeSet cs;
    if (Track* t = p.track(track_)) {
        if (!captured_) { before_ = t->gainDb; captured_ = true; }
        t->gainDb = db;
        cs.routingChanged = true;
    }
    return cs;
}
ChangeSet SetTrackGain::apply(Project& p) { return set(p, after_); }
ChangeSet SetTrackGain::revert(Project& p) { return set(p, before_); }
bool SetTrackGain::mergeWith(const Command& next) {
    auto* n = dynamic_cast<const SetTrackGain*>(&next);
    if (!n || n->track_ != track_) return false;
    after_ = n->after_;
    return true;
}

// ---------------------------------------------------------------- UndoStack --
ChangeSet UndoStack::push(std::unique_ptr<Command> cmd, Project& p) {
    ChangeSet cs = cmd->apply(p);
    entries_.resize(cursor_);                // drop redo tail
    if (macroDepth_ > 0 && !entries_.empty()) {
        entries_.back().cmds.push_back(std::move(cmd));
        return cs;
    }
    if (!entries_.empty() && entries_.back().cmds.size() == 1 && cursor_ != cleanIndex_ &&
        entries_.back().cmds.front()->mergeWith(*cmd))
        return cs;
    entries_.push_back({cmd->label(), {}});
    entries_.back().cmds.push_back(std::move(cmd));
    if (entries_.size() > limit_) {
        entries_.erase(entries_.begin());
        cleanIndex_ = cleanIndex_ > 0 ? cleanIndex_ - 1 : ~std::uint64_t{0};
    }
    cursor_ = entries_.size();
    return cs;
}

ChangeSet UndoStack::undo(Project& p) {
    ChangeSet cs;
    if (!canUndo() || macroDepth_ > 0) return cs;
    auto& e = entries_[--cursor_];
    for (auto it = e.cmds.rbegin(); it != e.cmds.rend(); ++it) cs.merge((*it)->revert(p));
    return cs;
}

ChangeSet UndoStack::redo(Project& p) {
    ChangeSet cs;
    if (!canRedo() || macroDepth_ > 0) return cs;
    auto& e = entries_[cursor_++];
    for (auto& c : e.cmds) cs.merge(c->apply(p));
    return cs;
}

void UndoStack::beginMacro(std::string label) {
    if (macroDepth_++ == 0) {
        entries_.resize(cursor_);
        entries_.push_back({std::move(label), {}});
        cursor_ = entries_.size();
    }
}

void UndoStack::endMacro() {
    if (macroDepth_ > 0 && --macroDepth_ == 0 && entries_.back().cmds.empty()) {
        entries_.pop_back();
        cursor_ = entries_.size();
    }
}

}  // namespace corinzia::model
