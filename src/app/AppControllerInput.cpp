// Note input for AppController: caret, keyboard/mouse entry, edits on the
// selection. All edits go through edit:: (pure) + ReplaceVoiceEvents, so each
// entered note is one undo step and flows through the normal ChangeSet fan-out
// (provisional glyphs next frame, audio swap, re-engrave, autosave).

#include "corinzia/app/AppController.hpp"

#include "corinzia/rt/TempoMap.hpp"

#include <QTimer>

#include <cmath>

namespace corinzia::app {

namespace {

int keyFifthsAt(const model::Movement& m, Fraction pos) {
    int fifths = 0;
    for (const auto& k : m.keys)
        if (k.at <= pos) fifths = k.fifths;
    return fifths;
}

Fraction barLengthAt(const model::Movement& m, Fraction pos) {
    Fraction len{1};
    for (const auto& me : m.meter)
        if (me.at <= pos) len = Fraction(me.numerator, me.denominator);
    return len;
}

// Smallest whole number of bars (from the last meter change) covering `end`.
Fraction roundUpToBar(const model::Movement& m, Fraction end) {
    Fraction start{0};
    for (const auto& me : m.meter)
        if (me.at <= end) start = me.at;
    const Fraction bar = barLengthAt(m, end);
    const Fraction rel = end - start;
    std::int64_t bars = (rel.num * bar.den + (rel.den * bar.num) - 1) / (rel.den * bar.num);   // ceil
    return start + bar * Fraction(bars);
}

}  // namespace

// ------------------------------------------------------------------ state --
void AppController::setInputMode(bool on) {
    if (on == inputMode_) return;
    inputMode_ = on;
    // Entering input mode starts at the selected item, like Sibelius/Dorico.
    if (on)
        if (const model::Locator* loc = project_.locate(selection_))
            if (model::Movement* m = project_.movement(loc->movement))
                for (std::size_t li = 0; li < m->lanes.size(); ++li)
                    for (const auto& v : m->lanes[li].voices)
                        if (v.id == loc->voice)
                            if (const auto* ev = edit::findEvent(*v.events, selection_)) moveCaretTo(li, v.index, ev->start);
    syncCaret();
}

void AppController::setDuration(int d) {
    spec_.base = edit::BaseDuration(std::clamp(d, 0, 6));
    syncCaret();
}
void AppController::setDots(int d) {
    spec_.dots = std::clamp(d, 0, 2);
    syncCaret();
}
void AppController::setTriplet(bool t) {
    spec_.triplet = t;
    syncCaret();
}
void AppController::setAccidental(int a) {
    accidental_ = (a == 9 || (a >= -2 && a <= 2)) ? a : 9;
    syncCaret();
}
void AppController::setChordMode(bool c) {
    chordMode_ = c;
    syncCaret();
}
void AppController::setVoice(int v) {
    voice_ = std::clamp(v, 1, 4);
    syncCaret();
}

void AppController::moveCaretTo(std::size_t lane, int voice, Fraction pos) {
    caretLane_ = lane;
    voice_ = std::clamp(voice, 1, 4);
    caretPos_ = pos < Fraction{0} ? Fraction{0} : pos;
}

void AppController::syncCaret() {
    if (canvas_) {
        canvas_->setInputMode(inputMode_);
        canvas_->setCaret(caretLane_, inputMode_ ? caretPos_.toQuarters() : std::numeric_limits<double>::quiet_NaN());
    }
    emit inputStateChanged();
}

QString AppController::caretLabel() const {
    if (project_.movements.empty()) return {};
    const model::Movement& m = *project_.movements.front();
    rt::TempoMap tm;
    std::vector<rt::TempoMap::Meter> meters;
    for (const auto& me : m.meter) meters.push_back({me.at.toQuarters(), 0, me.numerator, me.denominator});
    tm.buildMeters(meters);
    const double q = caretPos_.toQuarters();
    const auto bar = tm.barAt(q);
    const double beatLen = 4.0 / bar.den;
    const double beat = (q - bar.barStartQuarter) / beatLen + 1.0;
    QString who;
    if (caretLane_ < m.lanes.size())
        if (const model::Player* pl = const_cast<model::Project&>(project_).player(m.lanes[caretLane_].player))
            who = QString::fromStdString(pl->name);
    return QStringLiteral("Bar %1, beat %2 · %3 · voice %4")
        .arg(bar.bar)
        .arg(std::abs(beat - std::round(beat)) < 1e-6 ? QString::number(int(std::lround(beat))) : QString::number(beat, 'f', 2))
        .arg(who)
        .arg(voice_);
}

// ---------------------------------------------------------------- helpers --
AppController::VoiceRef AppController::caretVoice(bool create) {
    VoiceRef ref{project_.movement(displayedMovement()), nullptr, nullptr};
    if (!ref.movement || ref.movement->lanes.empty()) return ref;
    caretLane_ = std::min(caretLane_, ref.movement->lanes.size() - 1);
    auto find = [&] {
        ref.lane = &ref.movement->lanes[caretLane_];
        for (auto& v : ref.lane->voices)
            if (v.index == voice_) ref.voice = &v;
    };
    find();
    if (!ref.voice && create) {
        push(std::make_unique<model::AddVoice>(ref.movement->id, ref.lane->player, VoiceId{project_.allocateId()},
                                               std::uint8_t(voice_)));
        find();   // vector may have reallocated: look it up again
    }
    return ref;
}

void AppController::extendMovementTo(model::Movement& m, Fraction end) {
    if (end > m.length) push(std::make_unique<model::SetMovementLength>(m.id, roundUpToBar(m, end)));
}

void AppController::commitVoiceEdit(const VoiceRef& ref, const edit::EditResult& r, const char* label, bool extendLength) {
    if (extendLength) extendMovementTo(*ref.movement, r.to);
    push(std::make_unique<model::ReplaceVoiceEvents>(ref.movement->id, ref.voice->id, r.events,
                                                     model::DirtyRange{ref.movement->id, r.from, r.to}, label,
                                                     /*mergeable=*/false));
}

void AppController::audition(const model::PlayerLane& lane, Fraction at, const std::vector<model::Pitch>& pitches) {
    const model::Instrument* inst = project_.instrument(lane.instrumentAt(at));
    const model::Track* track = inst ? project_.track(inst->track) : nullptr;
    if (!track || !track->instrumentNode.valid()) return;
    for (const auto& p : pitches) {
        rt::UiToAudio on{rt::UiToAudio::Type::PreviewNote};
        on.nodeId = track->instrumentNode;
        on.channel = track->midiChannel;
        on.port = std::int16_t(track->notePort);
        on.key = std::int16_t(p.midiKey());
        on.value = 0.7;
        engine_.post(on);
        rt::UiToAudio off = on;
        off.value = -1.0;
        QTimer::singleShot(350, this, [this, off] { engine_.post(off); });
    }
}

// ------------------------------------------------------------------ entry --
void AppController::enterNote(int step, bool addToChord) {
    step = ((step % 7) + 7) % 7;
    undo_.beginMacro("Note input");
    VoiceRef ref = caretVoice(true);
    if (!ref.voice) {
        undo_.endMacro();
        return;
    }
    auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
    const model::EventList& evs = *ref.voice->events;
    const int fifths = keyFifthsAt(*ref.movement, caretPos_);
    const std::optional<int> alter = accidental_ == 9 ? std::nullopt : std::optional<int>(accidental_);

    // Chord: add above the event just entered (the one ending at the caret).
    const model::MusicEvent* last = nullptr;
    for (const auto& e : evs)
        if (e.start < caretPos_ && e.start + e.duration == caretPos_ && !e.heads.empty()) last = &e;
    if ((addToChord || chordMode_) && last) {
        const model::Pitch top = last->heads.back().pitch;
        model::Pitch p = edit::nearestPitch(step, alter, fifths, top);
        if (edit::diatonic(p) <= edit::diatonic(top)) p = edit::fromDiatonic(edit::diatonic(p) + 7, p.alter);
        const Fraction at = last->start;   // `last` points into the list we are about to replace
        if (auto r = edit::addToChord(evs, at, p, ids)) {
            commitVoiceEdit(ref, *r, "Add to chord", false);
            selection_ = r->focus;
            audition(*caretVoice(false).lane, at, {p});
        }
    } else {
        model::Pitch reference{6, 0, 4};   // B4: middle of the treble staff
        if (auto prev = edit::previousPitch(evs, caretPos_)) reference = *prev;
        const model::Pitch p = edit::nearestPitch(step, alter, fifths, reference);
        auto r = edit::insertEvent(evs, edit::makeEvent(ids, caretPos_, spec_.value(), {p}), voice_ == 1, ids);
        const Fraction at = caretPos_;
        commitVoiceEdit(ref, r, "Note input", true);
        selection_ = r.focus;
        caretPos_ = r.caret;
        audition(*caretVoice(false).lane, at, {p});
    }
    undo_.endMacro();
    accidental_ = 9;   // accidentals apply to one note, like Sibelius
    if (canvas_) canvas_->setSelection(selection_);
    updateSelectionLabel();
    syncCaret();
}

void AppController::enterRest() {
    undo_.beginMacro("Rest input");
    VoiceRef ref = caretVoice(true);
    if (ref.voice) {
        auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
        auto r = edit::insertEvent(*ref.voice->events, edit::makeEvent(ids, caretPos_, spec_.value(), {}), voice_ == 1, ids);
        commitVoiceEdit(ref, r, "Rest input", true);
        selection_ = r.focus;
        caretPos_ = r.caret;
    }
    undo_.endMacro();
    if (canvas_) canvas_->setSelection(selection_);
    syncCaret();
}

void AppController::placeNoteAt(int page, QPointF pt, Qt::KeyboardModifiers mods) {
    if (!canvas_ || !canvas_->engraving() || !canvas_->engraving()->anchors) return;
    const auto hit = canvas_->engraving()->anchors->locate(page, pt);
    if (!hit.valid) return;
    // Snap to the grid of the current (undotted) duration, biased to the left
    // so clicking just after a beat still lands on it.
    const Fraction grid = edit::DurationSpec{spec_.base, 0, spec_.triplet}.value();
    const double gridQ = grid.toQuarters();
    const auto k = std::int64_t(std::floor(hit.quarter / gridQ + 0.35));
    caretLane_ = hit.lane;
    caretPos_ = grid * Fraction(std::max<std::int64_t>(0, k));

    const int step = ((hit.diatonic % 7) + 7) % 7;
    undo_.beginMacro("Note input");
    VoiceRef ref = caretVoice(true);
    if (ref.voice) {
        auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
        const int alter = accidental_ == 9 ? edit::keyAlter(keyFifthsAt(*ref.movement, caretPos_), step) : accidental_;
        const model::Pitch p = edit::fromDiatonic(hit.diatonic, alter);
        const auto* onset = edit::eventAt(*ref.voice->events, caretPos_);
        std::optional<edit::EditResult> r;
        if ((mods & Qt::ShiftModifier || chordMode_) && onset && onset->start == caretPos_ && !onset->heads.empty())
            r = edit::addToChord(*ref.voice->events, caretPos_, p, ids);
        else
            r = edit::insertEvent(*ref.voice->events, edit::makeEvent(ids, caretPos_, spec_.value(), {p}), voice_ == 1, ids);
        if (r) {
            const Fraction at = caretPos_;
            commitVoiceEdit(ref, *r, "Note input", true);
            selection_ = r->focus;
            caretPos_ = r->caret;
            audition(*caretVoice(false).lane, at, {p});
        }
    }
    undo_.endMacro();
    accidental_ = 9;
    if (canvas_) canvas_->setSelection(selection_);
    updateSelectionLabel();
    syncCaret();
}

// ---------------------------------------------------------------- caret --
void AppController::moveCaret(int direction) {
    const Fraction step = spec_.value();
    Fraction next = caretPos_ + step * Fraction(direction);
    if (next < Fraction{0}) next = Fraction{0};
    if (const model::Movement* m = project_.movement(displayedMovement()); m && next > m->length) next = m->length;
    caretPos_ = next;
    syncCaret();
}

void AppController::moveCaretStaff(int direction) {
    const model::Movement* m = project_.movement(displayedMovement());
    if (!m || m->lanes.empty()) return;
    const auto lanes = std::int64_t(m->lanes.size());
    caretLane_ = std::size_t(std::clamp<std::int64_t>(std::int64_t(caretLane_) + direction, 0, lanes - 1));
    voice_ = 1;
    syncCaret();
}

void AppController::backspace() {
    VoiceRef ref = caretVoice(false);
    if (!ref.voice) return;
    const model::MusicEvent* prev = nullptr;
    for (const auto& e : *ref.voice->events)
        if (e.start < caretPos_ && e.start + e.duration <= caretPos_) prev = &e;
    if (!prev) return;
    const Fraction to = prev->start;
    if (!(prev->kind == model::EventKind::Rest && voice_ == 1)) {   // a voice-1 rest is already "empty"
        auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
        if (auto r = edit::deleteElement(*ref.voice->events, prev->id.id, voice_ == 1, ids))
            commitVoiceEdit(ref, *r, "Delete", false);
    }
    caretPos_ = to;
    syncCaret();
}

// ------------------------------------------------------ selection edits --
namespace {
struct SelectionVoice { model::Movement* m = nullptr; model::Voice* v = nullptr; };
SelectionVoice voiceOf(model::Project& p, ElementId sel) {
    SelectionVoice out;
    const model::Locator* loc = p.locate(sel);
    if (!loc) return out;
    out.m = p.movement(loc->movement);
    if (!out.m) return out;
    for (auto& lane : out.m->lanes)
        for (auto& v : lane.voices)
            if (v.id == loc->voice) out.v = &v;
    return out;
}
}  // namespace

void AppController::deleteSelection() {
    if (range_.active) {
        const Range r = range_;
        auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
        editRangeVoices("Delete", [r, &ids](const model::EventList& evs, const model::Voice& v) {
            return std::optional<edit::EditResult>(edit::eraseRange(evs, r.from, r.to, v.index == 1, ids));
        });
        return;
    }
    auto sv = voiceOf(project_, selection_);
    if (!sv.v) return;
    auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
    if (auto r = edit::deleteElement(*sv.v->events, selection_, sv.v->index == 1, ids)) {
        push(std::make_unique<model::ReplaceVoiceEvents>(sv.m->id, sv.v->id, r->events,
                                                         model::DirtyRange{sv.m->id, r->from, r->to}, "Delete", false));
        selection_ = r->focus;
        if (canvas_) canvas_->setSelection(selection_);
        updateSelectionLabel();
    }
}

void AppController::toggleArticulation(int flag) {
    if (range_.active) {
        // Sibelius rule: if every note in the passage already has it, remove it; else add it.
        const Range r = range_;
        bool all = true, any = false;
        const model::Movement* m = project_.movement(displayedMovement());
        for (std::size_t li = r.laneFrom; m && li <= r.laneTo; ++li)
            for (const auto& v : m->lanes[li].voices)
                for (const auto& e : *v.events)
                    if (e.start >= r.from && e.start < r.to && !e.heads.empty()) {
                        any = true;
                        all &= (e.articulations & flag) != 0;
                    }
        if (!any) return;
        const bool set = !all;
        editRangeVoices("Articulation", [r, flag, set](const model::EventList& evs, const model::Voice&) {
            return edit::applyToRange(evs, r.from, r.to, [flag, set](model::MusicEvent& e) {
                if (e.heads.empty()) return;
                e.articulations = set ? std::uint16_t(e.articulations | flag) : std::uint16_t(e.articulations & ~flag);
            });
        });
        return;
    }
    auto sv = voiceOf(project_, selection_);
    if (!sv.v) return;
    if (auto r = edit::modifyEvent(*sv.v->events, selection_,
                                   [flag](model::MusicEvent& e, model::NoteHead*) { e.articulations ^= std::uint16_t(flag); }))
        push(std::make_unique<model::ReplaceVoiceEvents>(sv.m->id, sv.v->id, r->events,
                                                         model::DirtyRange{sv.m->id, r->from, r->to}, "Articulation", false));
}

void AppController::setDynamic(int mark) {
    if (range_.active) {   // the dynamic goes on the first note of the passage on each staff
        const Range r = range_;
        editRangeVoices("Dynamic", [r, mark](const model::EventList& evs, const model::Voice& v) -> std::optional<edit::EditResult> {
            if (v.index != 1) return std::nullopt;
            for (const auto& e : evs)
                if (e.start >= r.from && e.start < r.to && !e.heads.empty())
                    return edit::modifyEvent(evs, e.id.id, [mark](model::MusicEvent& x, model::NoteHead*) {
                        if (mark < 0) x.dynamicMark.reset();
                        else x.dynamicMark = std::uint8_t(std::min(mark, 7));
                    });
            return std::nullopt;
        });
        return;
    }
    auto sv = voiceOf(project_, selection_);
    if (!sv.v) return;
    if (auto r = edit::modifyEvent(*sv.v->events, selection_, [mark](model::MusicEvent& e, model::NoteHead*) {
            if (mark < 0) e.dynamicMark.reset();
            else e.dynamicMark = std::uint8_t(std::min(mark, 7));
        }))
        push(std::make_unique<model::ReplaceVoiceEvents>(sv.m->id, sv.v->id, r->events,
                                                         model::DirtyRange{sv.m->id, r->from, r->to}, "Dynamic", false));
}

void AppController::toggleTie() {
    auto sv = voiceOf(project_, selection_);
    if (!sv.v) return;
    if (auto r = edit::modifyEvent(*sv.v->events, selection_, [](model::MusicEvent& e, model::NoteHead* h) {
            if (h) h->tieToNext = !h->tieToNext;
            else
                for (auto& x : e.heads) x.tieToNext = !x.tieToNext;
        }))
        push(std::make_unique<model::ReplaceVoiceEvents>(sv.m->id, sv.v->id, r->events,
                                                         model::DirtyRange{sv.m->id, r->from, r->to}, "Tie", false));
}

}  // namespace corinzia::app
