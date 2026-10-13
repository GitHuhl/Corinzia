#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/model/Commands.hpp"

using namespace corinzia;
using namespace corinzia::model;

TEST(fraction_arithmetic) {
    CHECK(Fraction(1, 4) + Fraction(1, 4) == Fraction(1, 2));
    CHECK(Fraction(2, 3) * Fraction(3, 4) == Fraction(1, 2));
    CHECK(Fraction(1, 3) < Fraction(1, 2));
    CHECK_NEAR(Fraction(3, 8).toQuarters(), 1.5, 1e-12);
}

TEST(svg_id_roundtrip) {
    for (std::uint64_t v : {1ull, 35ull, 36ull, 123456789ull, (1ull << 62)}) {
        ElementId e{v};
        CHECK(ElementId::fromSvgId(e.toSvgId()) == e);
    }
    CHECK(!ElementId::fromSvgId("staff-7").valid());
    CHECK(!ElementId::fromSvgId("e-").valid());
}

TEST(instrument_change_resolution) {
    Project p;
    auto ids = fixture::makeProject(p);
    const auto& lane = p.movements[0]->lanes[0];
    CHECK(lane.instrumentAt(Fraction{0}) == ids.flute);
    CHECK(lane.instrumentAt(Fraction{3, 4}) == ids.flute);
    CHECK(lane.instrumentAt(Fraction{1}) == ids.piccolo);
}

TEST(locator_maps_heads_to_tracks) {
    Project p;
    auto ids = fixture::makeProject(p);
    const auto& evs = *p.movements[0]->lanes[0].voices[0].events;
    const Locator* a = p.locate(evs[2].heads[1].id.id);   // chord in bar 1 (flute)
    const Locator* b = p.locate(evs[5].heads[0].id.id);   // bar 2 (piccolo)
    CHECK(a && a->kind == Locator::Kind::NoteHead && a->track == ids.t1 && a->voice == ids.voice);
    CHECK(b && b->track == ids.t2);
    CHECK(p.locate(ElementId{999999}) == nullptr);
}

TEST(undo_redo_and_merge) {
    Project p;
    auto ids = fixture::makeProject(p);
    UndoStack undo;
    auto voiceEvents = [&] { return p.movements[0]->lanes[0].voices[0].events; };
    const auto original = voiceEvents();

    auto transpose = [&](int steps) {
        auto copy = std::make_shared<EventList>(*voiceEvents());
        (*copy)[0].heads[0].pitch.step = std::int8_t(((*copy)[0].heads[0].pitch.step + steps) % 7);
        return std::make_unique<ReplaceVoiceEvents>(ids.mov, ids.voice, std::move(copy),
                                                    DirtyRange{ids.mov, Fraction{0}, Fraction{1, 4}}, "Move note");
    };
    const auto rev0 = p.movements[0]->revision;
    auto cs = undo.push(transpose(1), p);
    CHECK(cs.movements.size() == 1 && cs.voices.size() == 1);
    undo.push(transpose(1), p);        // merges with previous (same voice, same label)
    CHECK(voiceEvents()->at(0).heads[0].pitch.step == 2);
    CHECK(p.movements[0]->revision == rev0 + 2);

    undo.undo(p);                      // one step back to the original
    CHECK(voiceEvents() == original);  // COW: the very same shared list
    CHECK(!undo.canUndo() && undo.canRedo());
    undo.redo(p);
    CHECK(voiceEvents()->at(0).heads[0].pitch.step == 2);

    undo.beginMacro("Paste");
    undo.push(transpose(1), p);
    undo.push(transpose(1), p);
    undo.endMacro();
    CHECK(voiceEvents()->at(0).heads[0].pitch.step == 4);
    undo.undo(p);
    CHECK(voiceEvents()->at(0).heads[0].pitch.step == 2);
}

TEST(snapshot_shares_cow_payloads) {
    Project p;
    auto ids = fixture::makeProject(p);
    auto snap = p.snapshot(ids.mov);
    CHECK(snap.movement && snap.movement->lanes[0].voices[0].events == p.movements[0]->lanes[0].voices[0].events);
}
