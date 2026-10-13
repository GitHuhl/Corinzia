// Range selection (marquee, shift-click, select all), clipboard and the
// operations that act on a range. Every edit is per voice through the pure
// edit:: functions, grouped into one undo step per user action.

#include "corinzia/app/AppController.hpp"

#include <QVariant>

#include <algorithm>

namespace corinzia::app {

std::optional<std::size_t> AppController::laneOf(ElementId id) const {
    const model::Locator* loc = project_.locate(id);
    const model::Movement* m = const_cast<model::Project&>(project_).movement(displayedMovement());
    if (!loc || !m || loc->movement != m->id) return std::nullopt;
    for (std::size_t li = 0; li < m->lanes.size(); ++li)
        if (m->lanes[li].player == loc->player) return li;
    return std::nullopt;
}

void AppController::setRange(std::size_t laneFrom, std::size_t laneTo, Fraction from, Fraction to) {
    range_ = {true, std::min(laneFrom, laneTo), std::max(laneFrom, laneTo), from, to};
    caretLane_ = range_.laneFrom;
    caretPos_ = from;
    syncRange();
}

void AppController::syncRange() {
    if (canvas_) {
        if (range_.active) canvas_->setRange(range_.laneFrom, range_.laneTo, range_.from.toQuarters(), range_.to.toQuarters());
        else canvas_->clearRange();
    }
    updateSelectionLabel();
    emit selectionChanged();
}

void AppController::clearSelection() {
    range_.active = false;
    selection_ = {};
    if (canvas_) canvas_->setSelection({});
    syncRange();
}

void AppController::selectAll() {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m || m->lanes.empty()) return;
    selection_ = {};
    if (canvas_) canvas_->setSelection({});
    setRange(0, m->lanes.size() - 1, Fraction{0}, m->length);
}

// Union of the lanes and time spans of the given notes/rests (+ the current
// selection when extending): that is how Sibelius/Dorico turn a marquee or a
// shift-click into a passage selection.
void AppController::selectElements(const std::vector<ElementId>& ids, bool extend) {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m) return;
    bool any = false;
    std::size_t lo = 0, hi = 0;
    Fraction from{0}, to{0};
    auto include = [&](std::size_t lane, Fraction a, Fraction b) {
        if (!any) { lo = hi = lane; from = a; to = b; any = true; return; }
        lo = std::min(lo, lane); hi = std::max(hi, lane);
        from = std::min(from, a); to = std::max(to, b);
    };
    std::vector<ElementId> all = ids;
    if (extend && selection_.valid()) all.push_back(selection_);
    if (extend && range_.active) {
        include(range_.laneFrom, range_.from, range_.to);
        include(range_.laneTo, range_.from, range_.to);
    }
    for (ElementId id : all) {
        const auto lane = laneOf(id);
        if (!lane) continue;
        for (const auto& v : m->lanes[*lane].voices)
            if (const auto* ev = edit::findEvent(*v.events, id)) include(*lane, ev->start, ev->start + ev->duration);
    }
    if (!any) return clearSelection();
    selection_ = {};
    if (canvas_) canvas_->setSelection({});
    setRange(lo, hi, from, to);
}

void AppController::onMarqueeSelected(QVariantList ids, Qt::KeyboardModifiers mods) {
    std::vector<ElementId> v;
    for (const auto& x : ids) v.push_back(ElementId{x.value<quint64>()});
    selectElements(v, mods & Qt::ShiftModifier);
}

void AppController::editRangeVoices(
    const char* label, const std::function<std::optional<edit::EditResult>(const model::EventList&, const model::Voice&)>& fn) {
    model::Movement* m = project_.movement(displayedMovement());
    if (!m || !range_.active) return;
    undo_.beginMacro(label);
    for (std::size_t li = range_.laneFrom; li <= range_.laneTo && li < m->lanes.size(); ++li)
        for (std::size_t vi = 0; vi < m->lanes[li].voices.size(); ++vi) {
            // Look the voice up again each time: commands replace event lists.
            const model::Voice& v = project_.movement(displayedMovement())->lanes[li].voices[vi];
            if (auto r = fn(*v.events, v))
                push(std::make_unique<model::ReplaceVoiceEvents>(m->id, v.id, r->events,
                                                                 model::DirtyRange{m->id, r->from, r->to}, label, false));
        }
    undo_.endMacro();
}

// ---------------------------------------------------------------- clipboard --
void AppController::copySelection() {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m) return;
    if (!range_.active && selection_.valid()) selectElements({selection_}, false);   // a single note copies as a range
    if (!range_.active) return;
    clip_ = {};
    clip_.valid = true;
    clip_.length = range_.to - range_.from;
    for (std::size_t li = range_.laneFrom; li <= range_.laneTo; ++li) {
        std::vector<std::pair<int, model::EventList>> lane;
        for (const auto& v : m->lanes[li].voices) lane.push_back({v.index, edit::sliceRange(*v.events, range_.from, range_.to)});
        clip_.lanes.push_back(std::move(lane));
    }
    setStatus(QStringLiteral("Copied %1 staff(s), %2 beats").arg(clip_.lanes.size()).arg(clip_.length.toQuarters()));
    emit selectionChanged();
}

void AppController::cutSelection() {
    copySelection();
    deleteSelection();
}

void AppController::paste() {
    if (!clip_.valid) return;
    model::Movement* m = project_.movement(displayedMovement());
    if (!m || m->lanes.empty()) return;
    // Target: the selected passage, else the selected note (pasting *at* it, as
    // in Sibelius/Dorico), else the input caret.
    std::size_t lane0 = range_.active ? range_.laneFrom : std::min(caretLane_, m->lanes.size() - 1);
    Fraction at = range_.active ? range_.from : caretPos_;
    if (!range_.active && selection_.valid())
        if (auto lane = laneOf(selection_))
            for (const auto& v : m->lanes[*lane].voices)
                if (const auto* e = edit::findEvent(*v.events, selection_)) {
                    lane0 = *lane;
                    at = e->start;
                }
    const Fraction end = at + clip_.length;
    auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
    undo_.beginMacro("Paste");
    extendMovementTo(*m, end);
    std::size_t lastLane = lane0;
    for (std::size_t i = 0; i < clip_.lanes.size() && lane0 + i < m->lanes.size(); ++i) {
        lastLane = lane0 + i;
        for (const auto& [voiceIndex, events] : clip_.lanes[i]) {
            model::Movement* mm = project_.movement(displayedMovement());
            model::PlayerLane& lane = mm->lanes[lane0 + i];
            const model::Voice* target = nullptr;
            for (const auto& v : lane.voices) if (v.index == voiceIndex) target = &v;
            if (!target) {
                if (events.empty()) continue;
                push(std::make_unique<model::AddVoice>(mm->id, lane.player, VoiceId{project_.allocateId()}, std::uint8_t(voiceIndex)));
                for (const auto& v : project_.movement(displayedMovement())->lanes[lane0 + i].voices)
                    if (v.index == voiceIndex) target = &v;
            }
            auto r = edit::pasteEvents(*target->events, events, at, clip_.length, voiceIndex == 1, ids);
            push(std::make_unique<model::ReplaceVoiceEvents>(mm->id, target->id, r.events,
                                                             model::DirtyRange{mm->id, r.from, r.to}, "Paste", false));
        }
    }
    undo_.endMacro();
    setRange(lane0, lastLane, at, end);   // the pasted passage becomes the selection
}

void AppController::repeatSelection() {
    if (!range_.active) {
        if (selection_.valid()) selectElements({selection_}, false);
        if (!range_.active) return;
    }
    const Clip saved = clip_;
    copySelection();
    const Range r = range_;
    range_ = {true, r.laneFrom, r.laneTo, r.to, r.to + (r.to - r.from)};   // paste target: right after
    paste();
    clip_ = saved.valid ? saved : clip_;   // R must not clobber the clipboard
    emit selectionChanged();
}

// -------------------------------------------------------------- spanners --
namespace {
// Single note selected: the span runs from it to the next note in its voice.
std::optional<std::pair<Fraction, Fraction>> spanFromNote(const model::EventList& evs, ElementId note) {
    const model::MusicEvent* e = edit::findEvent(evs, note);
    if (!e) return std::nullopt;
    for (const auto& n : evs)
        if (n.start > e->start && !n.heads.empty()) return std::pair{e->start, n.start + n.duration};
    return std::nullopt;
}
}  // namespace

void AppController::addSlur() {
    if (!range_.active && selection_.valid())
        if (auto lane = laneOf(selection_)) {
            const model::Movement* m = project_.movement(displayedMovement());
            for (const auto& v : m->lanes[*lane].voices)
                if (auto s = spanFromNote(*v.events, selection_)) setRange(*lane, *lane, s->first, s->second);
        }
    const Range r = range_;
    editRangeVoices("Slur", [r](const model::EventList& evs, const model::Voice&) {
        return edit::markSpan(evs, r.from, r.to, model::SlurStart, model::SlurEnd);
    });
}

void AppController::addHairpin(bool crescendo) {
    if (!range_.active && selection_.valid())
        if (auto lane = laneOf(selection_)) {
            const model::Movement* m = project_.movement(displayedMovement());
            for (const auto& v : m->lanes[*lane].voices)
                if (auto s = spanFromNote(*v.events, selection_)) setRange(*lane, *lane, s->first, s->second);
        }
    const Range r = range_;
    editRangeVoices(crescendo ? "Crescendo" : "Diminuendo", [r, crescendo](const model::EventList& evs, const model::Voice& v) {
        if (v.index != 1) return std::optional<edit::EditResult>{};   // one hairpin per staff, on voice 1
        return edit::markSpan(evs, r.from, r.to, crescendo ? model::CrescStart : model::DimStart, model::HairpinEnd);
    });
}

}  // namespace corinzia::app
