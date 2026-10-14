// Score structure from the UI: Dorico-style popovers (key, time, clef,
// barline, tempo, text, rehearsal marks), insert/delete bars, and deleting
// clicked structure items. Each user action is exactly one undo step: the
// header goes through EditMovementHeader, voice content through
// ReplaceVoiceEvents, grouped in a macro when both change.

#include "corinzia/app/AppController.hpp"
#include "corinzia/edit/Structure.hpp"

namespace corinzia::app {

namespace {

int barNumber(const model::Movement& m, Fraction at) {
    rt::TempoMap tm;
    std::vector<rt::TempoMap::Meter> meters;
    for (const auto& me : m.meter) meters.push_back({me.at.toQuarters(), 0, me.numerator, me.denominator});
    tm.buildMeters(meters);
    return tm.barAt(at.toQuarters()).bar;
}

}  // namespace

// Where a popover acts: the start of the passage selection, else the selected
// note/rest, else the input caret.
AppController::StructureTarget AppController::structureTarget() {
    StructureTarget t;
    t.movement = project_.movement(displayedMovement());
    if (!t.movement) return t;
    if (range_.active) {
        t.lane = range_.laneFrom;
        t.at = range_.from;
        return t;
    }
    if (const auto lane = laneOf(selection_))
        for (const auto& v : t.movement->lanes[*lane].voices)
            if (const auto* ev = edit::findEvent(*v.events, selection_)) {
                t.lane = *lane;
                t.at = ev->start;
                return t;
            }
    t.lane = std::min(caretLane_, t.movement->lanes.empty() ? std::size_t(0) : t.movement->lanes.size() - 1);
    t.at = std::min(caretPos_, t.movement->length);
    return t;
}

QString AppController::popoverPosition() {
    const auto t = structureTarget();
    return t.movement ? QStringLiteral("Bar %1").arg(barNumber(*t.movement, t.at)) : QString();
}

bool AppController::applyPopover(const QString& kind, const QString& input) {
    const auto t = structureTarget();
    if (!t.movement || t.movement->lanes.empty()) return false;
    model::Movement& m = *t.movement;
    const std::string text = input.trimmed().toStdString();
    model::MovementHeader h = model::captureHeader(m);
    auto ids = edit::IdAlloc([this] { return project_.allocateId(); });
    const Fraction bar = edit::barStartAtOrBefore(h, t.at);
    Fraction at = t.at;
    QString label;
    auto fail = [&](const QString& why) {
        setStatus(why);
        return false;
    };

    if (kind == u"ornament") {   // acts on the selection like the panel buttons
        const auto bit = edit::parseOrnament(text);
        if (!bit) return fail(QStringLiteral("Ornament not understood: tr, mordent, turn, trem, arp, breath, caesura"));
        if (!range_.active && !selection_.valid()) return fail(QStringLiteral("Select notes first"));
        toggleArticulation(*bit);
        return true;
    }
    if (kind == u"key") {
        const auto k = edit::parseKey(text);
        if (!k) return fail(QStringLiteral("Key not understood: try C, Eb, f# (minor) or \"a minor\""));
        edit::setKey(h, at = bar, *k);
        label = QStringLiteral("Key signature");
    } else if (kind == u"time") {
        const auto ts = edit::parseTime(text);
        if (!ts) return fail(QStringLiteral("Time signature not understood: try 3/4, 6/8, C or C|"));
        edit::setMeter(h, at = bar, ts->first, ts->second);
        // Keep the movement a whole number of bars in the new meter.
        const Fraction last = edit::barStartAtOrBefore(h, h.length);
        if (last != h.length) h.length = last + edit::barLength(h, last);
        label = QStringLiteral("Time signature");
    } else if (kind == u"clef") {
        const auto c = edit::parseClef(text);
        if (!c) return fail(QStringLiteral("Clef not understood: treble, bass, alto, tenor, treble_8, bass_8"));
        edit::setClef(h, m.lanes[t.lane].player, at, *c, ids);
        label = QStringLiteral("Clef");
    } else if (kind == u"barline") {
        const auto b = edit::parseBarline(text);
        if (!b) return fail(QStringLiteral("Barline not understood: ||  |.  |:  :|  :|:"));
        at = edit::nearestBarline(h, t.at);
        if (at == Fraction{0} && *b != model::BarlineKind::RepeatStart) at = std::min(bar + edit::barLength(h, bar), h.length);
        edit::setBarline(h, at, *b, ids);
        label = QStringLiteral("Barline");
    } else if (kind == u"tempo") {
        const auto tp = edit::parseTempo(text);
        if (!tp) return fail(QStringLiteral("Tempo not understood: try \"Allegro\", \"Andante 84\" or \"q=96\""));
        edit::setTempo(h, at, *tp);
        label = QStringLiteral("Tempo");
    } else if (kind == u"expression" || kind == u"technique") {
        if (text.empty()) return fail(QStringLiteral("Type some text first"));
        edit::addText(h, m.lanes[t.lane].player, at, text,
                      kind == u"expression" ? model::TextKind::Expression : model::TextKind::Technique, ids);
        label = kind == u"expression" ? QStringLiteral("Expression text") : QStringLiteral("Technique text");
    } else if (kind == u"rehearsal") {
        const std::size_t before = h.rehearsal.size();
        edit::toggleRehearsalMark(h, at = bar, ids);
        label = h.rehearsal.size() > before ? QStringLiteral("Rehearsal mark") : QStringLiteral("Remove rehearsal mark");
    } else {
        return fail(QStringLiteral("Unknown popover %1").arg(kind));
    }
    push(std::make_unique<model::EditMovementHeader>(m.id, std::move(h), label.toStdString()));
    setStatus(QStringLiteral("%1 · bar %2").arg(label).arg(barNumber(m, std::min(at, m.length))));
    return true;
}

void AppController::insertBars(int count) {
    const auto t = structureTarget();
    if (!t.movement || count < 1) return;
    model::Movement& m = *t.movement;
    model::MovementHeader h = model::captureHeader(m);
    const Fraction at = edit::barStartAtOrBefore(h, t.at);
    const Fraction len = edit::barLength(h, at) * Fraction(count);
    // Compute every voice first; pushing changes the project.
    std::vector<std::pair<VoiceId, std::shared_ptr<const model::EventList>>> voices;
    for (const auto& lane : m.lanes)
        for (const auto& v : lane.voices)
            voices.emplace_back(v.id, std::make_shared<const model::EventList>(edit::insertSpan(*v.events, at, len)));
    edit::insertSpan(h, at, len);
    const Fraction end = h.length;
    undo_.beginMacro("Insert bars");
    push(std::make_unique<model::EditMovementHeader>(m.id, std::move(h), "Insert bars"));
    for (auto& [vid, list] : voices)
        push(std::make_unique<model::ReplaceVoiceEvents>(m.id, vid, list, model::DirtyRange{m.id, at, end}, "Insert bars", false));
    undo_.endMacro();
    setStatus(QStringLiteral("Inserted %1 bar%2 before bar %3").arg(count).arg(count == 1 ? "" : "s").arg(barNumber(m, at)));
}

void AppController::deleteBars() {
    const auto t = structureTarget();
    if (!t.movement) return;
    model::Movement& m = *t.movement;
    model::MovementHeader h = model::captureHeader(m);
    // Whole bars covering the passage, or the bar holding the caret/selection.
    const Fraction from = edit::barStartAtOrBefore(h, t.at);
    Fraction to = from + edit::barLength(h, from);
    if (range_.active && range_.to > to) {
        to = edit::barStartAtOrBefore(h, range_.to);
        if (to < range_.to) to = to + edit::barLength(h, to);
    }
    to = std::min(to, h.length);
    if (from >= to || (to - from) >= h.length) {
        setStatus(QStringLiteral("A movement needs at least one bar"));
        return;
    }
    const int b0 = barNumber(m, from), b1 = barNumber(m, to - Fraction(1, 1024));
    std::vector<std::pair<VoiceId, std::shared_ptr<const model::EventList>>> voices;
    for (const auto& lane : m.lanes)
        for (const auto& v : lane.voices)
            voices.emplace_back(v.id, std::make_shared<const model::EventList>(edit::deleteSpan(*v.events, from, to)));
    const Fraction oldEnd = h.length;
    edit::deleteSpan(h, from, to);
    undo_.beginMacro("Delete bars");
    for (auto& [vid, list] : voices)
        push(std::make_unique<model::ReplaceVoiceEvents>(m.id, vid, list, model::DirtyRange{m.id, from, oldEnd}, "Delete bars", false));
    push(std::make_unique<model::EditMovementHeader>(m.id, std::move(h), "Delete bars"));
    undo_.endMacro();
    range_.active = false;
    selection_ = {};
    if (canvas_) canvas_->setSelection({});
    caretPos_ = std::min(from, m.length);
    syncRange();
    syncCaret();
    setStatus(b0 == b1 ? QStringLiteral("Deleted bar %1").arg(b0) : QStringLiteral("Deleted bars %1–%2").arg(b0).arg(b1));
}

// A clicked barline / rehearsal mark / clef / text: removed by Delete.
bool AppController::deleteStructureItem() {
    model::Movement* m = project_.movement(displayedMovement());
    if (!m || !selection_.valid()) return false;
    model::MovementHeader h = model::captureHeader(*m);
    const auto what = edit::removeItem(h, selection_);
    if (!what) return false;
    push(std::make_unique<model::EditMovementHeader>(m->id, std::move(h), "Delete " + *what));
    selection_ = {};
    if (canvas_) canvas_->setSelection({});
    updateSelectionLabel();
    setStatus(QStringLiteral("Deleted %1").arg(QString::fromStdString(*what)));
    return true;
}

}  // namespace corinzia::app
