#include "Test.hpp"

#include "corinzia/edit/NoteInput.hpp"

using namespace corinzia;
using namespace corinzia::edit;
using model::Pitch;

namespace {
struct Ids {
    std::uint64_t next = 1000;
    IdAlloc fn = [this] { return ElementId{next++}; };
};
model::EventList bar(Ids& ids) {   // c'4 d'4 e'2
    model::EventList v;
    v.push_back(makeEvent(ids.fn, {0}, {1, 4}, {{0, 0, 4}}));
    v.push_back(makeEvent(ids.fn, {1, 4}, {1, 4}, {{1, 0, 4}}));
    v.push_back(makeEvent(ids.fn, {1, 2}, {1, 2}, {{2, 0, 4}}));
    return v;
}
bool contiguous(const model::EventList& v) {
    Fraction at{0};
    for (const auto& e : v) {
        if (e.start != at) return false;
        at = e.start + e.duration;
    }
    return true;
}
}  // namespace

TEST(edit_durations) {
    CHECK(DurationSpec{BaseDuration::Quarter, 0, false}.value() == Fraction(1, 4));
    CHECK(DurationSpec{BaseDuration::Quarter, 1, false}.value() == Fraction(3, 8));
    CHECK(DurationSpec{BaseDuration::Half, 2, false}.value() == Fraction(7, 8));
    CHECK(DurationSpec{BaseDuration::Eighth, 0, true}.value() == Fraction(1, 12));
    CHECK(DurationSpec{BaseDuration::SixtyFourth, 0, false}.value() == Fraction(1, 64));
}

TEST(edit_pitch_spelling_and_octave) {
    CHECK(keyAlter(1, 3) == 1);          // G major: F#
    CHECK(keyAlter(1, 0) == 0);
    CHECK(keyAlter(-2, 6) == -1 && keyAlter(-2, 2) == -1 && keyAlter(-2, 5) == 0);   // Bb major: Bb, Eb
    const Pitch b4{6, 0, 4};
    CHECK(nearestPitch(0, std::nullopt, 0, b4).octave == 5);    // B4 -> nearest C is C5
    CHECK(nearestPitch(4, std::nullopt, 0, b4).octave == 4);    // B4 -> G4
    const Pitch f = nearestPitch(3, std::nullopt, 1, b4);       // F in G major = F#
    CHECK(f.alter == 1 && f.octave == 4);                       // F#4 is 3 steps down, F#5 4 up
    CHECK(nearestPitch(3, 0, 1, b4).alter == 0);                // explicit natural wins
}

TEST(edit_overwrite_insert_truncates_and_fills) {
    Ids ids;
    auto v = bar(ids);
    // Dotted quarter at beat 2 covers d'4 and the first eighth of e'2.
    auto ev = makeEvent(ids.fn, {1, 4}, {3, 8}, {{4, 0, 4}});
    auto r = insertEvent(v, ev, true, ids.fn);
    const auto& out = *r.events;
    CHECK(contiguous(out));
    CHECK(out.size() == 3);                                      // c4 g4. r4.
    CHECK(out[1].heads[0].pitch.step == 4 && out[1].duration == Fraction(3, 8));
    CHECK(out[2].kind == model::EventKind::Rest && out[2].start == Fraction(5, 8) && out[2].duration == Fraction(3, 8));
    CHECK(r.caret == Fraction(5, 8) && r.focus == ev.heads[0].id.id);
    CHECK(v.size() == 3);                                        // input untouched (COW)
}

TEST(edit_insert_after_end_fills_gap_in_voice1_only) {
    Ids ids;
    auto v = bar(ids);
    auto r1 = insertEvent(v, makeEvent(ids.fn, {3, 2}, {1, 4}, {{0, 0, 5}}), true, ids.fn);
    CHECK(contiguous(*r1.events) && r1.events->size() == 5 && r1.from == Fraction{1});
    auto r2 = insertEvent(v, makeEvent(ids.fn, {3, 2}, {1, 4}, {{0, 0, 5}}), false, ids.fn);
    CHECK(r2.events->size() == 4 && !contiguous(*r2.events));  // hole kept in voice 2+
}

TEST(edit_chords_and_delete) {
    Ids ids;
    auto v = bar(ids);
    auto c = addToChord(v, {0}, {2, 0, 4}, ids.fn);
    CHECK(c && (*c->events)[0].heads.size() == 2 && (*c->events)[0].kind == model::EventKind::Chord);
    CHECK(!addToChord(*c->events, {0}, {2, 0, 4}, ids.fn));            // duplicate head
    CHECK(!addToChord(v, {1, 8}, {2, 0, 4}, ids.fn));                  // no onset there
    // Delete one head of the chord -> note; delete the note -> rest.
    const ElementId upper = (*c->events)[0].heads[1].id.id;
    auto d1 = deleteElement(*c->events, upper, true, ids.fn);
    CHECK(d1 && (*d1->events)[0].heads.size() == 1 && (*d1->events)[0].kind == model::EventKind::Note);
    auto d2 = deleteElement(*d1->events, (*d1->events)[0].heads[0].id.id, true, ids.fn);
    CHECK(d2 && (*d2->events)[0].kind == model::EventKind::Rest && contiguous(*d2->events));
    auto d3 = deleteElement(v, v[1].id.id, false, ids.fn);              // voice 2 policy: hole
    CHECK(d3 && d3->events->size() == 2);
}

TEST(edit_modify_and_lookup) {
    Ids ids;
    auto v = bar(ids);
    auto m = modifyEvent(v, v[2].heads[0].id.id, [](model::MusicEvent& e, model::NoteHead* h) {
        e.articulations |= model::ArtAccent;
        h->tieToNext = true;
    });
    CHECK(m && ((*m->events)[2].articulations & model::ArtAccent) && (*m->events)[2].heads[0].tieToNext);
    CHECK(eventAt(v, {5, 8}) == &v[2]);
    CHECK(previousPitch(v, {1, 2})->step == 1);
    CHECK(voiceEnd(v) == Fraction{1});
    const model::NoteHead* h = nullptr;
    CHECK(findEvent(v, v[1].heads[0].id.id, &h) == &v[1] && h == &v[1].heads[0]);
}
