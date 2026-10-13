#include "Test.hpp"

#include "corinzia/edit/Structure.hpp"

using namespace corinzia;
using namespace corinzia::edit;
using model::BarlineKind;

namespace {
struct Ids {
    std::uint64_t next = 5000;
    IdAlloc fn = [this] { return ElementId{next++}; };
};
model::MovementHeader header() {
    model::MovementHeader h;
    h.tempo = {{Fraction{0}, 120.0, false}};
    h.meter = {{Fraction{0}, 4, 4}};
    h.keys = {{Fraction{0}, 0, false}};
    h.length = Fraction{8};
    h.lanes.push_back({PlayerId{1}, {}, {}, {}});
    return h;
}
}  // namespace

TEST(structure_parse_key) {
    auto eb = parseKey("Eb");
    CHECK(eb && eb->fifths == -3 && !eb->minor);
    auto fs = parseKey("f#");
    CHECK(fs && fs->fifths == 3 && fs->minor);
    auto bbm = parseKey("Bbm");
    CHECK(bbm && bbm->fifths == -5 && bbm->minor);
    auto am = parseKey("A minor");
    CHECK(am && am->fifths == 0 && am->minor);
    auto dmaj = parseKey("D major");
    CHECK(dmaj && dmaj->fifths == 2 && !dmaj->minor);
    auto uni = parseKey("E♭");
    CHECK(uni && uni->fifths == -3);
    CHECK(!parseKey("H"));
    CHECK(!parseKey(""));
    CHECK(!parseKey("cbm"));   // 10 flats
}

TEST(structure_parse_time_clef_barline) {
    CHECK(parseTime("6/8") == std::pair{6, 8});
    CHECK(parseTime(" 3/4 ") == std::pair{3, 4});
    CHECK(parseTime("C") == std::pair{4, 4});
    CHECK(parseTime("C|") == std::pair{2, 2});
    CHECK(!parseTime("3/5"));
    CHECK(!parseTime("x/4"));
    CHECK(!parseTime("0/4"));
    CHECK(parseClef("Bass") == std::string("bass"));
    CHECK(parseClef("c") == std::string("alto"));
    CHECK(parseClef("g8") == std::string("treble_8"));
    CHECK(!parseClef("soprano-ish"));
    CHECK(parseBarline("||") == BarlineKind::Double);
    CHECK(parseBarline("final") == BarlineKind::Final);
    CHECK(parseBarline("|:") == BarlineKind::RepeatStart);
    CHECK(parseBarline(":|") == BarlineKind::RepeatEnd);
    CHECK(parseBarline(":|:") == BarlineKind::RepeatBoth);
    CHECK(!parseBarline("|||"));
}

TEST(structure_parse_tempo) {
    auto a = parseTempo("Allegro 132");
    CHECK(a && a->text == "Allegro" && a->bpm == 132.0);
    auto q = parseTempo("q=96");
    CHECK(q && q->text.empty() && q->bpm == 96.0);
    auto n = parseTempo("72");
    CHECK(n && n->text.empty() && n->bpm == 72.0);
    auto ad = parseTempo("Adagio");
    CHECK(ad && ad->text == "Adagio" && ad->bpm == 70.0);
    auto eq = parseTempo("Andante = 84");
    CHECK(eq && eq->text == "Andante" && eq->bpm == 84.0);
    CHECK(!parseTempo("Whatever"));
    CHECK(!parseTempo("q=2"));
}

TEST(structure_bar_positions) {
    auto h = header();
    h.meter.push_back({Fraction{2}, 3, 4});   // bars 1-2 in 4/4, then 3/4
    CHECK(barStartAtOrBefore(h, Fraction(3, 2)) == Fraction{1});
    CHECK(barStartAtOrBefore(h, Fraction{2}) == Fraction{2});
    CHECK(barStartAtOrBefore(h, Fraction(23, 8)) == Fraction(11, 4));
    CHECK(barLength(h, Fraction{3}) == Fraction(3, 4));
    CHECK(nearestBarline(h, Fraction(9, 8)) == Fraction{1});
    CHECK(nearestBarline(h, Fraction(15, 8)) == Fraction{2});
    CHECK(nearestBarline(h, Fraction(63, 8)) == Fraction{8});   // final barline
}

TEST(structure_header_edits) {
    Ids ids;
    auto h = header();
    setKey(h, Fraction{1}, {2, false});
    setKey(h, Fraction{1}, {-1, false});   // replaces
    CHECK(h.keys.size() == 2 && h.keys[1].fifths == -1);
    setMeter(h, Fraction{2}, 6, 8);
    CHECK(h.meter.size() == 2 && h.meter[1].numerator == 6);
    setTempo(h, Fraction{0}, {"Lento", 56});
    CHECK(h.tempo.size() == 1 && h.tempo[0].text == "Lento" && h.tempo[0].quarterBpm == 56.0);
    setBarline(h, Fraction{4}, BarlineKind::Double, ids.fn);
    setBarline(h, Fraction{4}, BarlineKind::RepeatEnd, ids.fn);
    CHECK(h.barlines.size() == 1 && h.barlines[0].kind == BarlineKind::RepeatEnd);
    toggleRehearsalMark(h, Fraction{3}, ids.fn);
    CHECK(h.rehearsal.size() == 1);
    toggleRehearsalMark(h, Fraction{3}, ids.fn);
    CHECK(h.rehearsal.empty());
    setClef(h, PlayerId{1}, Fraction{1}, "bass", ids.fn);
    CHECK(h.lanes[0].clefs.size() == 1 && h.lanes[0].clefs[0].clef == "bass");
    addText(h, PlayerId{1}, Fraction{2}, "dolce", model::TextKind::Expression, ids.fn);
    addText(h, PlayerId{1}, Fraction{1}, "pizz.", model::TextKind::Technique, ids.fn);
    CHECK(h.lanes[0].texts.size() == 2 && h.lanes[0].texts[0].text == "pizz.");
}

TEST(structure_insert_delete_header) {
    Ids ids;
    auto h = header();
    setKey(h, Fraction{2}, {1, false});
    setBarline(h, Fraction{4}, BarlineKind::Double, ids.fn);
    setClef(h, PlayerId{1}, Fraction{1}, "bass", ids.fn);
    insertSpan(h, Fraction{1}, Fraction{2});
    CHECK(h.length == Fraction{10});
    CHECK(h.keys[0].at == Fraction{0} && h.keys[1].at == Fraction{4});
    CHECK(h.barlines[0].at == Fraction{6});
    CHECK(h.lanes[0].clefs[0].at == Fraction{3});
    CHECK(h.meter[0].at == Fraction{0});   // items at 0 stay
    deleteSpan(h, Fraction{0}, Fraction{4});
    CHECK(h.length == Fraction{6});
    CHECK(h.keys.size() == 1 && h.keys[0].at == Fraction{0} && h.keys[0].fifths == 1);   // bar-4 key replaces the initial one
    CHECK(h.lanes[0].clefs.empty());
    CHECK(h.barlines[0].at == Fraction{2});
}

TEST(structure_insert_delete_events) {
    Ids ids;
    model::EventList v;
    v.push_back(makeEvent(ids.fn, {0}, {1, 2}, {{0, 0, 4}}));
    v.push_back(makeEvent(ids.fn, {1, 2}, {1}, {{1, 0, 4}}));   // crosses bar 1
    v.push_back(makeEvent(ids.fn, {2}, {1}, {{2, 0, 4}}));
    auto ins = insertSpan(v, Fraction{1}, Fraction{1});
    CHECK(ins[0].start == Fraction{0} && ins[1].start == Fraction(1, 2) && ins[2].start == Fraction{3});
    auto del = deleteSpan(v, Fraction{1}, Fraction{2});
    CHECK(del.size() == 3);
    CHECK(del[1].duration == Fraction(1, 2));   // truncated at 1
    CHECK(del[1].heads[0].tieToNext == false);
    CHECK(del[2].start == Fraction{1});         // [1,2) excludes 2, so it shifts left
    auto del2 = deleteSpan(v, Fraction{1}, Fraction{3});
    CHECK(del2.size() == 2 && del2[1].duration == Fraction(1, 2));
}
