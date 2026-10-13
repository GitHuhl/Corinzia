#include "corinzia/edit/NoteInput.hpp"

#include <algorithm>
#include <cstdlib>

namespace corinzia::edit {

using model::EventKind;
using model::EventList;
using model::MusicEvent;
using model::NoteHead;
using model::Pitch;

Fraction DurationSpec::value() const {
    Fraction v(1, std::int64_t(1) << int(base));
    Fraction add = v;
    for (int i = 0; i < dots; ++i) {
        add = add * Fraction(1, 2);
        v = v + add;
    }
    return triplet ? v * Fraction(2, 3) : v;
}

int diatonic(const Pitch& p) noexcept { return p.step + 7 * p.octave; }

Pitch fromDiatonic(int d, int alter) noexcept {
    const int step = ((d % 7) + 7) % 7;
    return {std::int8_t(step), std::int8_t(alter), std::int8_t((d - step) / 7)};
}

int keyAlter(int fifths, int step) noexcept {
    // Order of sharps: F C G D A E B (steps 3 0 4 1 5 2 6); flats the reverse.
    static constexpr int kSharps[7] = {3, 0, 4, 1, 5, 2, 6};
    if (fifths > 0)
        for (int i = 0; i < std::min(fifths, 7); ++i) if (kSharps[i] == step) return 1;
    if (fifths < 0)
        for (int i = 0; i < std::min(-fifths, 7); ++i) if (kSharps[6 - i] == step) return -1;
    return 0;
}

Pitch nearestPitch(int step, std::optional<int> alter, int keyFifths, const Pitch& previous) noexcept {
    const int prev = diatonic(previous);
    int best = 0, bestDist = 1 << 30;
    for (int oct = previous.octave - 1; oct <= previous.octave + 1; ++oct) {
        const int d = step + 7 * oct;
        const int dist = std::abs(d - prev);
        if (dist < bestDist) { bestDist = dist; best = d; }   // ties (a 4th = 3 steps either way can't tie) keep lower
    }
    return fromDiatonic(best, alter.value_or(keyAlter(keyFifths, step)));
}

MusicEvent makeEvent(IdAlloc& ids, Fraction start, Fraction duration, const std::vector<Pitch>& pitches,
                     std::uint16_t articulations) {
    MusicEvent e;
    e.id = {ids()};
    e.start = start;
    e.duration = duration;
    e.articulations = articulations;
    for (const auto& p : pitches) e.heads.push_back({{ids()}, p, false, 0});
    e.kind = e.heads.empty() ? EventKind::Rest : e.heads.size() == 1 ? EventKind::Note : EventKind::Chord;
    return e;
}

namespace {

MusicEvent restOf(IdAlloc& ids, Fraction start, Fraction dur) { return makeEvent(ids, start, dur, {}); }

void sortEvents(EventList& v) {
    std::stable_sort(v.begin(), v.end(), [](const MusicEvent& a, const MusicEvent& b) { return a.start < b.start; });
}

// Removes [s, t) from the list (truncating / splitting) and returns the rests
// that must replace remainders, given the fill policy.
void clearRange(EventList& out, const EventList& in, Fraction s, Fraction t, bool fill, IdAlloc& ids) {
    for (const auto& e : in) {
        const Fraction end = e.start + e.duration;
        if (end <= s || e.start >= t) {   // untouched
            out.push_back(e);
            continue;
        }
        if (e.start < s) {                // keep the head part, shortened
            MusicEvent kept = e;
            kept.duration = s - e.start;
            for (auto& h : kept.heads) h.tieToNext = false;
            out.push_back(std::move(kept));
        }
        if (end > t && fill) out.push_back(restOf(ids, t, end - t));   // tail remainder
    }
}

}  // namespace

EditResult insertEvent(const EventList& list, MusicEvent ev, bool fillWithRests, IdAlloc& ids) {
    const Fraction s = ev.start, t = ev.start + ev.duration;
    auto out = std::make_shared<EventList>();
    out->reserve(list.size() + 3);
    clearRange(*out, list, s, t, fillWithRests, ids);
    Fraction from = s;
    if (fillWithRests) {                  // keep voice 1 contiguous up to the new event
        Fraction gapStart{0};
        for (const auto& e : *out)
            if (e.start < s) gapStart = std::max(gapStart, e.start + e.duration);
        if (gapStart < s) {
            out->push_back(restOf(ids, gapStart, s - gapStart));
            from = gapStart;
        }
    }
    const ElementId focus = ev.heads.empty() ? ev.id.id : ev.heads.front().id.id;
    out->push_back(std::move(ev));
    sortEvents(*out);
    return {std::move(out), focus, from, t, t};
}

std::optional<EditResult> addToChord(const EventList& list, Fraction pos, Pitch pitch, IdAlloc& ids) {
    auto out = std::make_shared<EventList>(list);
    for (auto& e : *out) {
        if (e.start != pos || e.heads.empty()) continue;
        for (const auto& h : e.heads)
            if (diatonic(h.pitch) == diatonic(pitch) && h.pitch.alter == pitch.alter) return std::nullopt;
        NoteHead h{{ids()}, pitch, false, 0};
        const ElementId focus = h.id.id;
        e.heads.push_back(h);
        std::sort(e.heads.begin(), e.heads.end(),
                  [](const NoteHead& a, const NoteHead& b) { return a.pitch.midiKey() < b.pitch.midiKey(); });
        e.kind = EventKind::Chord;
        return EditResult{std::move(out), focus, e.start, e.start + e.duration, e.start + e.duration};
    }
    return std::nullopt;
}

std::optional<EditResult> deleteElement(const EventList& list, ElementId element, bool fillWithRests, IdAlloc& ids) {
    auto out = std::make_shared<EventList>();
    std::optional<EditResult> res;
    for (const auto& e : list) {
        const bool isEvent = e.id.id == element;
        auto hit = std::find_if(e.heads.begin(), e.heads.end(), [&](const NoteHead& h) { return h.id.id == element; });
        if (!isEvent && hit == e.heads.end()) {
            out->push_back(e);
            continue;
        }
        res = EditResult{nullptr, {}, e.start, e.start + e.duration, e.start};
        if (!isEvent && e.heads.size() > 1) {      // remove one head of a chord
            MusicEvent c = e;
            c.heads.erase(c.heads.begin() + (hit - e.heads.begin()));
            c.kind = c.heads.size() == 1 ? EventKind::Note : EventKind::Chord;
            res->focus = c.heads.front().id.id;
            out->push_back(std::move(c));
        } else if (fillWithRests && e.kind != EventKind::Rest) {
            MusicEvent r = restOf(ids, e.start, e.duration);
            res->focus = r.id.id;
            out->push_back(std::move(r));
        }
        // else: dropped (rest deleted, or hole allowed in voices 2-4)
    }
    if (!res) return std::nullopt;
    res->events = std::move(out);
    return res;
}

std::optional<EditResult> modifyEvent(const EventList& list, ElementId element,
                                      const std::function<void(MusicEvent&, NoteHead*)>& fn) {
    auto out = std::make_shared<EventList>(list);
    for (auto& e : *out) {
        NoteHead* head = nullptr;
        for (auto& h : e.heads) if (h.id.id == element) head = &h;
        if (e.id.id != element && !head) continue;
        fn(e, head);
        return EditResult{std::move(out), element, e.start, e.start + e.duration, e.start + e.duration};
    }
    return std::nullopt;
}

const MusicEvent* findEvent(const EventList& list, ElementId element, const NoteHead** head) {
    for (const auto& e : list) {
        if (e.id.id == element) return &e;
        for (const auto& h : e.heads)
            if (h.id.id == element) {
                if (head) *head = &h;
                return &e;
            }
    }
    return nullptr;
}

const MusicEvent* eventAt(const EventList& list, Fraction pos) {
    for (const auto& e : list)
        if (e.start <= pos && pos < e.start + e.duration) return &e;
    return nullptr;
}

std::optional<Pitch> previousPitch(const EventList& list, Fraction pos) {
    std::optional<Pitch> p;
    for (const auto& e : list)
        if (e.start < pos && !e.heads.empty()) p = e.heads.front().pitch;
    return p;
}

Fraction voiceEnd(const EventList& list) {
    Fraction end{0};
    for (const auto& e : list) end = std::max(end, e.start + e.duration);
    return end;
}

}  // namespace corinzia::edit
